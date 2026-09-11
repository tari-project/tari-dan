//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, future::Future, pin::Pin, time::Duration};

use futures::Stream;
use serde::{Deserialize, Serialize};
use tari_consensus_types::Decision;
use tari_engine_types::{
    Utxo,
    commit_result::ExecuteResult,
    substate::{Substate, SubstateId, SubstateValue},
    transaction_receipt::FinalizeOutcome,
};
use tari_indexer_client::types::WatchedSubstateItem;
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

    fn query_substate(
        &self,
        address: &SubstateId,
        version: Option<u64>,
        local_search_only: bool,
    ) -> impl Future<Output = Result<SubstateQueryResult, Self::Error>> + Send;

    fn get_substates(
        &self,
        substate_ids: Vec<SubstateId>,
    ) -> impl Future<Output = Result<HashMap<SubstateId, Substate>, Self::Error>> + Send;

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

    fn query_transaction_result(
        &self,
        transaction_id: TransactionId,
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
    fn get_current_epoch(&self) -> impl Future<Output = Result<Epoch, Self::Error>> + Send;

    fn wait_until_ready(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateQueryResult {
    pub version: u64,
    pub substate: SubstateValue,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionQueryResult {
    pub result: TransactionFinalizedResult,
    pub transaction_id: TransactionId,
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
