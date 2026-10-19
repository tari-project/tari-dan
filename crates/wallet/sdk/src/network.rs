//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, future::Future, pin::Pin, time::Duration};

use futures::Stream;
use serde::{Deserialize, Serialize};
use tari_consensus_types::Decision;
use tari_engine_types::{
    SubstateVersion,
    Utxo,
    commit_result::ExecuteResult,
    substate::{Substate, SubstateId, SubstateValue},
    transaction_receipt::FinalizeOutcome,
};
use tari_indexer_client::types::{
    EpochCheckpointEntry,
    ProvenTransactionReceipt,
    SubstateProof,
    ValidatorSetCommittee,
    WatchedSubstateItem,
};
use tari_ootle_common_types::{
    Epoch,
    StateVersion,
    optional::IsNotFoundError,
    response_status::TransactionStatusResponseError,
    shard::Shard,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId};
use tari_template_abi::TemplateDef;
use tari_template_lib::types::{
    ResourceAddress,
    TemplateAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};
use time::PrimitiveDateTime;

use crate::models::UtxoUpdatePayload;

/// A pinned, boxed stream of UTXO updates
pub type UtxoUpdateStream<E> = Pin<Box<dyn Stream<Item = Result<UtxoUpdatePayload, E>> + Send + 'static>>;

/// A pinned, boxed stream of transaction finalization notifications.
pub type TransactionFinalizedStream<E> =
    Pin<Box<dyn Stream<Item = Result<TransactionFinalizedNotification, E>> + Send + 'static>>;

/// A push notification that a transaction committed a receipt.
///
/// Only transactions that commit something (`Commit` or `FeeIntentCommit`) produce a notification: an aborted
/// transaction writes no substate, so it never appears on the stream. Subscribers must still query the result of
/// transactions that stay silent for too long.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionFinalizedNotification {
    pub transaction_id: TransactionId,
    pub outcome: FinalizeOutcome,
}

pub trait WalletNetworkInterface {
    type Error: IsNotFoundError + TransactionStatusResponseError + std::error::Error + Send + Sync + 'static;

    /// Queries a substate, at `version` or its latest. With `include_proof`, the result carries the proof the indexer
    /// verified it with, when a committee member could prove it.
    fn query_substate(
        &self,
        address: &SubstateId,
        version: Option<SubstateVersion>,
        local_search_only: bool,
        include_proof: bool,
    ) -> impl Future<Output = Result<SubstateQueryResult, Self::Error>> + Send;

    /// Fetches the latest version of each of `substate_ids` that exists. With `include_proofs`, the result carries the
    /// proof of each substate a committee member could prove.
    fn get_substates(
        &self,
        substate_ids: Vec<SubstateId>,
        include_proofs: bool,
    ) -> impl Future<Output = Result<FetchedSubstates, Self::Error>> + Send;

    fn submit_transaction(
        &self,
        transaction: Transaction,
    ) -> impl Future<Output = Result<TransactionId, Self::Error>> + Send;
    fn submit_transaction_envelope(
        &self,
        transaction: TransactionEnvelope,
    ) -> impl Future<Output = Result<TransactionId, Self::Error>> + Send;

    fn submit_dry_run_transaction(
        &self,
        transaction: Transaction,
    ) -> impl Future<Output = Result<TransactionQueryResult, Self::Error>> + Send;

    /// Queries the result of a transaction. With `include_proof`, a committed transaction's result carries its receipt
    /// and the proof that the receipt was committed.
    fn query_transaction_result(
        &self,
        transaction_id: TransactionId,
        include_proof: bool,
    ) -> impl Future<Output = Result<TransactionQueryResult, Self::Error>> + Send;

    /// Subscribes to finalization notifications for every transaction the network commits from this point on.
    /// The stream ends or yields an error when the connection is lost; the subscriber reconnects by calling again.
    fn subscribe_transaction_finalized(
        &self,
    ) -> impl Future<Output = Result<TransactionFinalizedStream<Self::Error>, Self::Error>> + Send;

    fn fetch_template_definition(
        &self,
        template_address: TemplateAddress,
    ) -> impl Future<Output = Result<TemplateDef, Self::Error>> + Send;

    fn stream_stealth_utxo_updates(
        &self,
        from_epoch: Epoch,
        resource_address: ResourceAddress,
        shard_state_versions: Vec<(Shard, StateVersion)>,
        unspent_only: bool,
    ) -> impl Future<Output = Result<UtxoUpdateStream<Self::Error>, Self::Error>> + Send;

    fn get_unspent_utxos(
        &self,
        resource_address: ResourceAddress,
        tag_and_nonce_pairs: Vec<(UtxoTag, RistrettoPublicKeyBytes)>,
    ) -> impl Future<Output = Result<Vec<(UtxoId, Utxo)>, Self::Error>> + Send;

    fn list_watched_substates(
        &self,
        template_address: Option<TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> impl Future<Output = Result<Vec<WatchedSubstateItem>, Self::Error>> + Send;
    /// The latest end-of-epoch checkpoint of the shard group that holds `shard`, with the next epoch's committee of
    /// that shard group and its proof.
    fn get_latest_epoch_checkpoint(
        &self,
        shard: Shard,
    ) -> impl Future<Output = Result<EpochCheckpointEntry, Self::Error>> + Send;

    /// Up to `limit` end-of-epoch checkpoints of the shard group that holds `shard`, from `from_epoch` in ascending
    /// epoch order, each with the next epoch's committee of that shard group and its proof.
    fn list_epoch_checkpoints(
        &self,
        from_epoch: Epoch,
        limit: u32,
        shard: Shard,
    ) -> impl Future<Output = Result<Vec<EpochCheckpointEntry>, Self::Error>> + Send;

    /// The committee of the shard group that holds `shard` in `epoch`, with its proof against that epoch's validator
    /// set.
    fn get_committee(
        &self,
        epoch: Epoch,
        shard: Shard,
    ) -> impl Future<Output = Result<ValidatorSetCommittee, Self::Error>> + Send;

    /// The epoch the network executes transactions in.
    fn get_current_epoch(&self) -> impl Future<Output = Result<Epoch, Self::Error>> + Send;

    fn wait_until_ready(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateQueryResult {
    pub version: SubstateVersion,
    pub substate: SubstateValue,
    /// The proof that the substate held this value, when one was asked for and served.
    #[serde(default)]
    pub proof: Option<SubstateProof>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FetchedSubstates {
    pub substates: HashMap<SubstateId, Substate>,
    /// The proof of each substate in `substates` that was asked for and served.
    #[serde(default)]
    pub proofs: HashMap<SubstateId, SubstateProof>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionQueryResult {
    pub result: TransactionFinalizedResult,
    pub transaction_id: TransactionId,
    /// The committed receipt and its proof, when the query asked for a proof and the transaction committed.
    #[serde(default)]
    pub receipt: Option<ProvenTransactionReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionFinalizedResult {
    Pending,
    Finalized {
        final_decision: Decision,
        execution_result: Option<Box<ExecuteResult>>,
        execution_time: Duration,
        finalized_time: PrimitiveDateTime,
        abort_details: Option<String>,
    },
    /// The transaction was rejected by mempool validation at submission and was never sequenced.
    Rejected {
        details: String,
        rejected_time: PrimitiveDateTime,
    },
}

impl TransactionFinalizedResult {
    pub fn into_execute_result(self) -> Option<ExecuteResult> {
        match self {
            TransactionFinalizedResult::Pending => None,
            TransactionFinalizedResult::Finalized { execution_result, .. } => execution_result.map(|r| *r),
            TransactionFinalizedResult::Rejected { .. } => None,
        }
    }
}
