//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    str::FromStr,
    time::Duration,
};

use bounded_vec::BoundedVec;
use serde::{Deserialize, Serialize};
use tari_consensus_types::Decision;
use tari_engine_types::{
    SubstateVersion,
    Utxo,
    commit_result::ExecuteResult,
    encoded::Encoded,
    events::Event,
    resource::Resource,
    substate::{EncodedSubstate, SubstateId, SubstateValue},
    transaction_receipt::{FinalizeOutcome, TransactionReceipt},
};
use tari_ootle_common_types::{Epoch, NumPreshards, ShardGroup, StateVersion, ValidatorSetHash, shard::Shard};
use tari_ootle_transaction::{Network, PrunedTransaction, TransactionEnvelope, TransactionId};
use tari_template_abi::TemplateDef;
use tari_template_lib_types::{
    Amount,
    Hash32,
    ResourceAddress,
    TemplateAddress,
    TemplateMetadataHash,
    TransactionReceiptAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};
use time::PrimitiveDateTime;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListSubstateItem {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub substate_id: SubstateId,
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub module_name: Option<String>,
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub version: SubstateVersion,
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub template_address: Option<TemplateAddress>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub timestamp: PrimitiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerGetSubstateRequest")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetSubstateRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u64>))]
    pub version: Option<SubstateVersion>,
    #[serde(default)]
    pub local_search_only: bool,
    /// If true, the response carries the proof the substate was verified with. Not combinable with
    /// `local_search_only`.
    #[serde(default)]
    pub include_proof: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerGetSubstateResponse")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetSubstateResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub version: SubstateVersion,
    /// What the shard group holds for `version`. Only a request with `include_proof` that names the version is
    /// answered with [`SubstateState::Down`], and only when the indexer verified a down proof for it; any other down
    /// is a 404.
    pub state: SubstateState,
    /// True when the indexer verified this answer against the shard group committee (via a merkle
    /// proof). False when proofs are disabled, or when no committee member could supply a proof yet
    /// (e.g. nothing committed since an epoch change) and the value was served unverified.
    pub verified: bool,
    /// The proof the indexer verified this answer with. Present only when `include_proof` was set
    /// and a committee member could prove the answer. A down answer always carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub proof: Option<SubstateProof>,
}

impl GetSubstateResponse {
    /// The substate's encoded value, or `None` when `version` is down.
    pub fn into_substate_value(self) -> Option<Encoded<SubstateValue>> {
        match self.state {
            SubstateState::Up { substate } => Some(substate),
            SubstateState::Down {} => None,
        }
    }
}

/// What the shard group holds for a substate version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub enum SubstateState {
    /// The version is live, holding `substate`.
    Up {
        /// The CBOR-encoded `SubstateValue`, as the validator that answered the indexer served it. A proof is checked
        /// against the value these bytes decode to.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
        substate: Encoded<SubstateValue>,
    },
    /// The version was live and has been spent. The proof's `down_proof` shows both.
    Down {},
}

/// Proof that a substate value was committed by its shard group.
///
/// Two parts: a commit proof that the shard group committee committed a block whose header carries
/// `anchor.state_merkle_root`, and a value proof that the substate's leaf is in the state tree under
/// that root. Verifying it requires the committee for `anchor.epoch` and `anchor.shard_group`.
///
/// A proof shows the value was committed at the anchor block. It says nothing about later blocks, so
/// a caller judges freshness by the anchor's epoch and height.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct SubstateProof {
    /// The committed block the proof is anchored at, as read from `commit_proof`.
    pub anchor: SubstateProofAnchor,
    /// CBOR-encoded `SidechainBlockCommitProof`: the anchor block's header and the chain of quorum
    /// certificates that committed it.
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub commit_proof: Vec<u8>,
    /// CBOR-encoded `SubstateValueProof`: the inclusion proof of the substate's leaf under
    /// `anchor.state_merkle_root`.
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub value_proof: Vec<u8>,
    /// The epoch the substate's leaf value hash was computed in. It is an input to that hash, so a
    /// verifier needs it to recompute the leaf.
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub value_hash_epoch: Epoch,
    /// CBOR-encoded `SubstateDownProof`, present when the substate version is down: an inclusion proof that the
    /// version was committed, anchored at a block of its own, and an exclusion proof under `anchor.state_merkle_root`
    /// that it is no longer in the state tree. `value_proof` is that exclusion proof.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "ootle_serde::base64::option"
    )]
    #[cfg_attr(feature = "ts", ts(optional, type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>, format = Byte))]
    pub down_proof: Option<Vec<u8>>,
}

/// The committed block a [`SubstateProof`] is anchored at.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct SubstateProofAnchor {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub shard_group: ShardGroup,
    pub height: u64,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub block_id: Hash32,
    /// The shard group state root the block header commits.
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub state_merkle_root: Hash32,
}

/// The most substates the indexer serves in one [`GetSubstatesRequest`].
pub const MAX_SUBSTATES_PER_FETCH: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetSubstatesRequest {
    // Note that we may permit less than 50 in the handler, but this is the max we'll deserialize for DoS mitigation
    /// The list of substate IDs to fetch
    #[cfg_attr(feature = "ts", ts(as = "Vec<SubstateId>"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<String>))]
    pub requests: BoundedVec<SubstateId, 1, 50>,
    /// If true, only search local storage for the substates. This may result in substates not being found even if they
    /// exist. Otherwise, the indexer will attempt to fetch substates from validator nodes across various shard groups
    /// which may result in more failures.
    pub cached_only: bool,
    /// If true, the response carries the proof each substate was verified with. Not combinable with
    /// `cached_only`.
    #[serde(default)]
    pub include_proofs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetSubstatesResponse {
    /// Each live substate's version and CBOR-encoded value, as in [`SubstateState::Up`].
    #[cfg_attr(feature = "utoipa", schema(value_type = HashMap<String, Object>))]
    pub substates: HashMap<SubstateId, EncodedSubstate>,
    /// The proof each substate was verified with, when `include_proofs` was set, less the commit proof of the block
    /// it is anchored at, which is in `commit_proofs`. A substate no committee member could prove has no entry.
    #[serde(default)]
    #[cfg_attr(feature = "utoipa", schema(value_type = HashMap<String, BatchedSubstateProof>))]
    pub proofs: HashMap<SubstateId, BatchedSubstateProof>,
    /// The commit proof of each block a proof in `proofs` is anchored at, by block id. Proofs anchored at the same
    /// block share it.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "{ [block_id in string]?: string }"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = HashMap<String, String>))]
    pub commit_proofs: HashMap<Hash32, CommitProofBytes>,
}

impl GetSubstatesResponse {
    /// A response serving `substates` with `proofs`, each anchor block's commit proof served once.
    pub fn new(
        substates: HashMap<SubstateId, EncodedSubstate>,
        proofs: impl IntoIterator<Item = (SubstateId, SubstateProof)>,
    ) -> Self {
        let mut commit_proofs = HashMap::new();
        let proofs = proofs
            .into_iter()
            .map(|(id, proof)| {
                commit_proofs
                    .entry(proof.anchor.block_id)
                    .or_insert(CommitProofBytes(proof.commit_proof));
                (id, BatchedSubstateProof {
                    anchor: proof.anchor,
                    value_proof: proof.value_proof,
                    value_hash_epoch: proof.value_hash_epoch,
                    down_proof: proof.down_proof,
                })
            })
            .collect();
        Self {
            substates,
            proofs,
            commit_proofs,
        }
    }

    /// The substates, and each proof whole with the commit proof of its anchor block. A proof whose anchor block has
    /// no commit proof in the response is left out, so its substate reads as unproven.
    pub fn into_proven(self) -> (HashMap<SubstateId, EncodedSubstate>, HashMap<SubstateId, SubstateProof>) {
        let Self {
            substates,
            proofs,
            commit_proofs,
        } = self;
        let proofs = proofs
            .into_iter()
            .filter_map(|(id, proof)| {
                let commit_proof = commit_proofs.get(&proof.anchor.block_id)?;
                Some((id, SubstateProof {
                    anchor: proof.anchor,
                    commit_proof: commit_proof.0.clone(),
                    value_proof: proof.value_proof,
                    value_hash_epoch: proof.value_hash_epoch,
                    down_proof: proof.down_proof,
                }))
            })
            .collect();
        (substates, proofs)
    }
}

/// A [`SubstateProof`] served in a [`GetSubstatesResponse`], whose commit proof the response serves once for every
/// proof anchored at the same block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct BatchedSubstateProof {
    /// The committed block the proof is anchored at. The response's `commit_proofs` holds its commit proof under
    /// `anchor.block_id`.
    pub anchor: SubstateProofAnchor,
    /// CBOR-encoded `SubstateValueProof`, as in [`SubstateProof::value_proof`].
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub value_proof: Vec<u8>,
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub value_hash_epoch: Epoch,
    /// CBOR-encoded `SubstateDownProof`, as in [`SubstateProof::down_proof`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "ootle_serde::base64::option"
    )]
    #[cfg_attr(feature = "ts", ts(optional, type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>, format = Byte))]
    pub down_proof: Option<Vec<u8>>,
}

/// CBOR-encoded `SidechainBlockCommitProof`: a block's header and the chain of quorum certificates that committed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitProofBytes(#[serde(with = "ootle_serde::base64")] pub Vec<u8>);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct InspectSubstateRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub address: SubstateId,
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u64>))]
    pub version: Option<SubstateVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct InspectSubstateResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub version: SubstateVersion,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub substate: SubstateValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "tari-indexer-client/",
        rename = "IndexerSubmitTransactionRequest"
    )
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct SubmitTransactionRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    /// A BOR-encoded transaction envelope, base64 encoded as a string
    pub transaction: TransactionEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "tari-indexer-client/",
        rename = "IndexerSubmitTransactionResponse"
    )
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct SubmitTransactionResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    /// The ID of the submitted transaction
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "tari-indexer-client/",
        rename = "IndexerSubmitTransactionDryRunResponse"
    )
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct SubmitTransactionDryRunResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    /// The ID of the transaction that was dry-run
    pub transaction_id: TransactionId,
    /// The CBOR-encoded `ExecuteResult` of the dry run, including any emitted events and state changes, but without
    /// a final decision or commitment to the ledger.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub result: Encoded<ExecuteResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListTemplateCatalogueRequest {
    /// Optional substring filter on template name.
    pub name_filter: Option<String>,
    /// Maximum number of entries to return (default: 20, max: 100).
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub limit: Option<u64>,
    /// Cursor: return entries inserted after the row with this template address.
    /// When omitted, returns from the beginning.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub after: Option<TemplateAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListTemplateCatalogueResponse {
    pub entries: Vec<TemplateCatalogueItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct TemplateCatalogueItem {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub template_address: TemplateAddress,
    pub template_name: String,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub author_public_key: RistrettoPublicKeyBytes,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub binary_hash: Hash32,
    pub at_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub metadata_hash: Option<TemplateMetadataHash>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "tari-indexer-client/",
        rename = "IndexerGetTransactionResultRequest"
    )
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetTransactionResultRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    /// The ID of the transaction to query the result for
    pub transaction_id: TransactionId,
    /// If true, a committed transaction's result carries its receipt and the proof that the receipt
    /// was committed.
    #[serde(default)]
    pub include_proof: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "tari-indexer-client/",
        rename = "IndexerGetTransactionResultResponse"
    )
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetTransactionResultResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    /// The result of the transaction, which may be pending (not yet finalized) or finalized with details such as the
    /// final decision, execution result, and timestamps
    pub result: IndexerTransactionFinalizedResult,
    /// The committed transaction receipt and its proof, when `include_proof` was set and the
    /// transaction committed. An aborted transaction commits no receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub receipt: Option<ProvenTransactionReceipt>,
}

/// Query parameters of `GET /transactions/{transaction_id}/result`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct GetTransactionResultQuery {
    /// If true, a committed transaction's result carries its receipt and the proof that the receipt
    /// was committed.
    #[serde(default)]
    pub include_proof: bool,
}

/// A committed transaction's receipt substate, with the proof it was verified with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ProvenTransactionReceipt {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub version: SubstateVersion,
    /// The receipt substate's CBOR-encoded `SubstateValue`, as in [`SubstateState::Up`]. It decodes to
    /// `SubstateValue::TransactionReceipt`.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub value: Encoded<SubstateValue>,
    /// `None` when no committee member could prove the receipt, e.g. when nothing has been committed
    /// since an epoch change.
    pub proof: Option<SubstateProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerGetTransactionResponse")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetTransactionResponse {
    /// The stored transaction, including its instructions, fee instructions and signatures.
    pub transaction: TransactionEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct QueryTransactionEventsRequest {
    /// Filter events by topic
    pub topic: Option<String>,
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub substate_id: Option<SubstateId>,
    /// Filter by resource address. Matches the events whose `substate_id` is that resource, which
    /// is the `std.resource.*` family. Vault events name no resource — filter those by
    /// `substate_id`, the vault's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub resource_address: Option<ResourceAddress>,
    pub limit: Option<u32>,
    /// Matches to skip, at most 10,000. Page deeper with `before_id`.
    pub offset: Option<u32>,
    /// Return only events older than this cursor. Pass the previous page's `next_before_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    pub before_id: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct QueryTransactionEventsResponse {
    /// Each event's transaction and its CBOR-encoded `Event`.
    #[cfg_attr(feature = "ts", ts(type = "Array<[string, string]>"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(String, String)>))]
    pub events: Vec<(TransactionId, Encoded<Event>)>,
    /// The `before_id` for the next page, or `None` once no older event can match. A wildcard
    /// topic query examines a bounded number of events per request, so a page can be short, or
    /// empty, and still carry a cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_before_id: Option<u64>,
}

/// Filter parameters for the transaction events SSE stream.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct StreamTransactionEventsRequest {
    pub topic: Option<String>,
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub substate_id: Option<SubstateId>,
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub template_address: Option<TemplateAddress>,
    /// Filter by resource address. Matches the events whose `substate_id` is that resource, which
    /// is the `std.resource.*` family. Vault events name no resource — filter those by
    /// `substate_id`, the vault's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub resource_address: Option<ResourceAddress>,
    /// Resume the event stream from this event ID (exclusive). Events with id > after_id will be
    /// replayed from the database before switching to the live stream.
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub after_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListRecentTransactionsRequest {
    pub limit: Option<u32>,
    #[serde(default)]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub last_id: Option<TransactionId>,
    /// Restrict the listing to transactions from this source. Omitted, transactions from every
    /// source are listed.
    #[serde(default)]
    pub source: Option<TransactionSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListRecentTransactionsResponse {
    pub transactions: Vec<TransactionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct TransactionEntry {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub transaction_id: TransactionId,
    /// Pruned transaction — blob commitments retained, raw blob bytes omitted to keep the
    /// response size bounded. The transaction id and signatures remain verifiable.
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub transaction: PrunedTransaction,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub created_at: PrimitiveDateTime,
    /// Result summary from the locally indexed transaction receipt. None when no receipt has been
    /// indexed yet (pending or aborted).
    pub summary: Option<TransactionResultSummary>,
    /// Reason the transaction was rejected by mempool validation when it was submitted through
    /// this indexer. None if the transaction was not rejected at submission.
    pub rejected_reason: Option<String>,
    /// Where this indexer learned of the transaction.
    pub source: TransactionSource,
}

/// Where an indexer learned of a transaction it has stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum TransactionSource {
    /// Submitted directly to this indexer. A transaction the indexer first saw on the gossip topic
    /// and that was later submitted to it directly is recorded as local.
    Local,
    /// Observed on the network-wide transaction gossip topic.
    Gossip,
}

impl TransactionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Gossip => "gossip",
        }
    }
}

impl Display for TransactionSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for TransactionSource {
    type Err = UnknownTransactionSource;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "gossip" => Ok(Self::Gossip),
            _ => Err(UnknownTransactionSource(s.to_string())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnknownTransactionSource(pub String);

impl Display for UnknownTransactionSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unknown transaction source '{}'", self.0)
    }
}

impl std::error::Error for UnknownTransactionSource {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct TransactionResultSummary {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub outcome: FinalizeOutcome,
    pub total_fees_paid: u64,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub finalized_at: PrimitiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub enum IndexerTransactionFinalizedResult {
    Pending,
    Finalized {
        #[cfg_attr(feature = "utoipa", schema(value_type = String))]
        final_decision: Decision,
        /// The CBOR-encoded `ExecuteResult`, as the committee member that answered served it.
        #[cfg_attr(feature = "ts", ts(type = "string | null"))]
        #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>, format = Byte))]
        execution_result: Option<Encoded<ExecuteResult>>,
        #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number}"))]
        execution_time: Duration,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        finalized_time: PrimitiveDateTime,
        abort_details: Option<String>,
    },
    /// The transaction was rejected by mempool validation when it was submitted through this
    /// indexer and was never sequenced by the network.
    Rejected {
        details: String,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        rejected_time: PrimitiveDateTime,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerGetIdentityResponse")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetIdentityResponse {
    pub peer_id: String,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub public_key: RistrettoPublicKeyBytes,
    pub public_addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerGetCommsStatsResponse")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetCommsStatsResponse {
    pub connection_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "tari-indexer-client/",
        rename = "IndexerGetEpochManagerStatsResponse"
    )
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetEpochManagerStatsResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    /// The current epoch according to the indexer's epoch oracle view
    pub current_epoch: Epoch,
    /// The epoch the validators execute transactions in, as the indexer has observed from their
    /// committed blocks, held within one epoch below `current_epoch`. It trails `current_epoch` after
    /// each epoch boundary until every committee has committed its end-of-epoch block. `None` until
    /// the indexer has observed every committee.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u64>))]
    pub consensus_epoch: Option<Epoch>,
    pub current_block_height: u64,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub current_block_hash: Hash32,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerConnection")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct Connection {
    pub connection_id: String,
    pub peer_id: String,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub direction: ConnectionDirection,
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number}"))]
    pub age: Duration,
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number} | null"))]
    pub ping_latency: Option<Duration>,
    pub user_agent: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerConnectionDirection")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub enum ConnectionDirection {
    Inbound,
    Outbound,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerGetConnectionsResponse")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetConnectionsResponse {
    pub connections: Vec<Connection>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetTemplateDefinitionResponse {
    pub name: String,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub definition: TemplateDef,
    pub code_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct IndexerReadyResponse {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetUtxoUpdatesRequest {
    #[serde(default)]
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub from_epoch: Epoch,
    #[cfg_attr(feature = "utoipa", schema(value_type = (u32, u64)))]
    pub shard_state_versions: Vec<(Shard, StateVersion)>,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub resource_address: ResourceAddress,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unspent_only: bool,
    /// The number of UTXO rows to read per shard in this pass, from 1 to 1000. `from_epoch` and
    /// `unspent_only` are applied to the rows read, so a shard can return fewer updates than this,
    /// or none, and still have more: `StartOfShard::has_more` says whether it does. A state version
    /// wider than the limit is returned whole, overrunning it.
    pub per_shard_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UtxoUpdateSet {
    pub shard_updates: HashMap<Shard, UtxoStateUpdateSet>,
    pub per_shard_high_watermark: Vec<(Shard, StateVersion)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UtxoStateUpdateSet {
    pub updates: Vec<WalletUtxoUpdate>,
    /// The state version the read reached. Every update of the shard at or below it is either in
    /// `updates` or excluded by the request's filters, so it is a resume point: a later request
    /// filtering on `state_version > max_state_version` loses nothing. It can be above the highest
    /// version in `updates`, and is the request's own cursor when the read found nothing past it.
    pub max_state_version: StateVersion,
    pub max_epoch: Epoch,
    /// The shard holds further rows above `max_state_version`, which may or may not pass the
    /// request's filters.
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum WalletUtxoUpdate {
    Unspent(UtxoUnspent),
    Spent(UtxoSpent),
    Burnt(UtxoBurnt),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UtxoUnspent {
    pub tag: UtxoTag,
    pub public_nonce: RistrettoPublicKeyBytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UtxoSpent {
    pub id: UtxoId,
    pub version: SubstateVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UtxoBurnt {
    pub id: UtxoId,
    pub version: SubstateVersion,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetUtxoUpdatesResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub updates: UtxoUpdateSet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetUtxosRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(u32, String)>))]
    pub tag_and_nonce_pairs: Vec<(UtxoTag, RistrettoPublicKeyBytes)>,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub resource_address: ResourceAddress,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetUtxosResponse {
    /// The unspent UTXOs matching a requested tag and nonce pair, each a CBOR-encoded `Utxo`.
    #[cfg_attr(feature = "ts", ts(type = "Array<[string, string]>"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(String, String)>))]
    pub utxos: Vec<(UtxoId, Encoded<Utxo>)>,
    /// The requested tag and nonce pairs this indexer holds as spent or burnt. A requested pair in neither list is
    /// unknown to this indexer, which may not yet have indexed it.
    #[serde(default)]
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(u32, String)>))]
    pub spent: Vec<(UtxoTag, RistrettoPublicKeyBytes)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListUtxosRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub resource_address: ResourceAddress,
    pub limit: u32,
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub from_id: Option<UtxoId>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListUtxosResponse {
    /// Each UTXO's id and its CBOR-encoded `Utxo`.
    #[cfg_attr(feature = "ts", ts(type = "Array<[string, string]>"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(String, String)>))]
    pub utxos: Vec<(UtxoId, Encoded<Utxo>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetNetworkInfoResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub network: Network,
    pub network_byte: u8,
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
}

/// The configuration of a single indexer node, as far as it affects what its API returns. Every field
/// is local to the node answering the request: two indexers on the same network can disagree on all of
/// them, so a client that cares must ask the node it is talking to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetIndexerInfoResponse {
    /// The indexer's build version.
    pub version: String,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub network: Network,
    pub network_byte: u8,
    /// The sidechain this indexer follows, if it is configured for one.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub sidechain_id: Option<RistrettoPublicKeyBytes>,
    /// The current epoch, so a client can resolve the epoch-denominated fields below against a clock
    /// without a second request.
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub current_epoch: Epoch,
    /// How many epochs past its terminal epoch this indexer retains a transaction before pruning the
    /// record. `None` means transactions are retained indefinitely. A client paginating transaction
    /// history hits this floor rather than the start of the chain.
    pub transaction_retention_epochs: Option<u64>,
    /// How many epochs past the epoch its transaction committed in this indexer retains a transaction
    /// receipt before pruning it. `None` means receipts are retained indefinitely. Longer than
    /// `transaction_retention_epochs`, so a retained transaction always has its receipt.
    pub transaction_receipt_retention_epochs: Option<u64>,
    /// How many epochs past the epoch its transaction committed in this indexer retains an event
    /// before pruning it. `None` means events are retained indefinitely. Event queries and the event
    /// stream's catch-up answer no further back than this.
    pub event_retention_epochs: Option<u64>,
    /// Whether this indexer stores transactions observed on the network gossip topic in addition to
    /// those submitted directly to it. When false, a transaction submitted elsewhere is unknown to
    /// this indexer until its receipt is synced.
    ///
    /// Even when true the stored set of transactions is best effort: an indexer misses whatever was
    /// gossiped while it was offline or while its inbound queue was full, and there is no backfill.
    /// Transaction receipts carry no such caveat — they are synced from network state and are
    /// complete back to `transaction_receipt_retention_epochs` — so a committed transaction always
    /// has a receipt even when its body is missing here. Transactions that never committed get no receipt, so gossip
    /// is the only source for them.
    pub index_gossiped_transactions: bool,
    /// Whether substates served by this indexer are verified against a shard group committee proof
    /// before being returned. When false, values are served as fetched from a single validator and
    /// carry no proof of correctness.
    pub verify_substate_proofs: bool,
    /// How long after a shard was last confirmed synced this indexer keeps serving cached substates
    /// for it, in seconds. Cached values are invalidated by the state transitions this indexer syncs
    /// rather than expired on a timer, so a served value trails the chain by the sync interval rather
    /// than by this bound - which is what stops a validator that has stopped serving transitions from
    /// holding a stale value open indefinitely.
    pub substate_cache_max_serve_lag_secs: u64,
    /// Whether this indexer stores every event on the network. When false it is configured with event
    /// filters, so event queries answer over a subset and absence is not proof an event did not occur.
    pub indexes_all_events: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetNetworkSyncStateResponse {
    pub network_desc: NetworkDescription,
    pub sync_progress: Option<SyncProgress>,
    /// The latest probe of each validator this indexer has synced from. Populated lazily, so
    /// validators that have never been contacted for a sync will not appear here.
    #[serde(default)]
    pub validators: Vec<ValidatorStatus>,
}

/// The latest probe of one validator's consensus pacemaker state, made by the indexer when it
/// opened a sync stream from that validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ValidatorStatus {
    /// libp2p PeerId of the validator.
    pub peer_id: String,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub shard_group: ShardGroup,
    /// Unix timestamp (seconds) of the latest probe.
    pub probed_at_unix_s: u64,
    /// Why the latest probe produced no snapshot, or `None` when it produced `snapshot`. While this
    /// is set, `snapshot` is stale: it is the last one an earlier probe produced.
    pub probe_error: Option<ValidatorProbeError>,
    /// The last snapshot any probe of this validator produced, or `None` if none has succeeded.
    pub snapshot: Option<ValidatorStatusSnapshot>,
}

/// Why a probe of a validator produced no snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ValidatorProbeError {
    pub kind: ValidatorProbeErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub enum ValidatorProbeErrorKind {
    /// The validator did not return a usable consensus state.
    StatusUnavailable,
    /// The validator served a committed block proof that failed verification, so the indexer does
    /// not sync from it.
    InvalidProof,
}

/// A validator's consensus pacemaker state as it reported it to the indexer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ValidatorStatusSnapshot {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub height: u64,
    /// The validator's self-reported consensus pacemaker state. Diagnostic only - this is not
    /// verified, so it should not be relied upon for anything but display.
    pub state: ValidatorConsensusState,
    /// Unix timestamp (seconds) at which this snapshot was captured.
    pub observed_at_unix_s: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub enum ValidatorConsensusState {
    Initialising,
    Idle,
    CheckSync,
    Syncing,
    Running,
    Sleeping,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListValidatorsRequest {
    /// The epoch to fetch the roster for. Defaults to the current epoch.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u64>))]
    pub epoch: Option<Epoch>,
}

/// The validator roster for an epoch, as registered on the base layer and tracked by the epoch
/// manager. Unlike the lazily-observed snapshots in `GetNetworkSyncStateResponse`, this is the
/// complete consensus-derived validator set for the epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListValidatorsResponse {
    /// The epoch the roster applies to.
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    pub validators: Vec<ValidatorInfo>,
}

/// A single validator's registration entry for an epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ValidatorInfo {
    /// The validator's consensus public key.
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub public_key: RistrettoPublicKeyBytes,
    /// libp2p PeerId derived from the public key. Can be used to join against the liveness
    /// snapshots in `GetNetworkSyncStateResponse.validators`.
    pub peer_id: String,
    /// The committee (shard group) the validator belongs to in this epoch.
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub shard_group: ShardGroup,
    /// The first epoch the validator's registration is active.
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub start_epoch: Epoch,
    /// The epoch the validator was deactivated, if any.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u64>))]
    pub end_epoch: Option<Epoch>,
    /// The public key that may claim fees earned by this validator.
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub fee_claim_public_key: RistrettoPublicKeyBytes,
    pub vote_power: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct NetworkDescription {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    // (shard group, num members)
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(Object, u32)>))]
    pub shard_groups: Vec<(ShardGroup, u32)>,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub num_preshards: NumPreshards,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct SyncProgress {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub last_epoch: Epoch,
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(Object, u64)>))]
    pub checkpoint_progress: Vec<(ShardGroup, Epoch)>,
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(u32, (u64, u64))>))]
    pub last_state_versions: Vec<(Shard, (StateVersion, Epoch))>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListTransactionReceiptsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub last_id: Option<TransactionReceiptAddress>,
    #[serde(default)]
    pub ordering: Ordering,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub enum Ordering {
    // Use default only where you "don't care" about the order. Ascending is more performant so it's the default.
    #[default]
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListTransactionReceiptsResponse {
    /// Each receipt's address and its CBOR-encoded `TransactionReceipt`.
    #[cfg_attr(feature = "ts", ts(type = "Array<[string, string]>"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(String, String)>))]
    pub receipts: Vec<(TransactionReceiptAddress, Encoded<TransactionReceipt>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetTransactionReceiptResponse {
    /// The CBOR-encoded `TransactionReceipt`.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub receipt: Encoded<TransactionReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetResourceResponse {
    /// The CBOR-encoded `Resource`.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub resource: Encoded<Resource>,
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub version: SubstateVersion,
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub total_supply: Option<Amount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetNetworkEconomicsResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub current_epoch: Epoch,
    /// Total XTR claimed (peg-in).
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub total_claimed: Amount,
    /// Total exhaust burned, sourced from checkpoint headers (consensus-backed, complete since genesis).
    /// Kept as a cross-check against `receipt_exhaust_burned`.
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub total_exhaust_burned: Amount,
    /// Total fees paid by transaction payers, summed from transaction receipts.
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub fee_volume: Amount,
    /// Total exhaust burned summed from the same receipts as `fee_volume`; `receipt_exhaust_burned /
    /// fee_volume` is the exact realized burn share, and this is the burn netted from `total_supply`. May
    /// transiently trail `total_exhaust_burned` while the receipt sync frontier catches up to the checkpoint
    /// frontier.
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub receipt_exhaust_burned: Amount,
    /// Circulating L2 supply: `total_claimed - receipt_exhaust_burned`.
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub total_supply: Amount,
    /// Leader fees sitting in validator fee pools that validators have not yet claimed: the sum of every pool's
    /// current balance.
    #[serde(default)]
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub validator_claimable_fees: Amount,
    /// Number of transaction receipts the indexer has stored.
    pub transaction_receipt_count: u64,
    /// The share of collected fees burned rather than paid to leaders, in basis points, in effect at `current_epoch`.
    pub target_burn_rate_bps: u16,
    /// Rate changes the council has scheduled to activate after `current_epoch`, ascending by epoch.
    /// Empty when nothing is pending.
    #[serde(default)]
    pub scheduled_burn_rates: Vec<ScheduledBurnRate>,
    /// The first epoch the release-scheduled rate governs again, when the council has retired.
    #[serde(default)]
    pub burn_rate_retired_from: Option<u64>,
}

/// A burn rate the council has scheduled, taking effect from `activation_epoch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ScheduledBurnRate {
    pub activation_epoch: u64,
    pub rate_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListEpochCheckpointsRequest {
    /// The epoch to start listing from (inclusive). Defaults to 0.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u64>))]
    pub from_epoch: Option<Epoch>,
    /// Maximum number of checkpoints to return (default: 20, max: 100).
    pub limit: Option<u32>,
    /// Only the checkpoint of the shard group that contains this shard, one per epoch. Shard groups change when the
    /// number of committees does, so a fixed shard is what follows one line of committees across epochs.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u32>))]
    pub shard: Option<Shard>,
    /// Include, with each checkpoint, the next epoch's committee for the shard group that contains `shard`, proved
    /// against the validator set hash the checkpoint names. Requires `shard`.
    #[serde(default)]
    pub include_committees: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListEpochCheckpointsResponse {
    /// Ascending by epoch. Only checkpoints that carry their end-of-epoch header's metadata preimage are listed.
    pub checkpoints: Vec<EpochCheckpointEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetLatestEpochCheckpointRequest {
    /// The latest checkpoint of the shard group that contains this shard, rather than of any shard group.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u32>))]
    pub shard: Option<Shard>,
    /// Include the next epoch's committee for the shard group that contains `shard`, proved against the validator set
    /// hash the checkpoint names. Requires `shard`.
    #[serde(default)]
    pub include_committees: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetLatestEpochCheckpointResponse {
    pub checkpoint: EpochCheckpointEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetLatestShardGroupCheckpointsResponse {
    /// The latest checkpoint of the shard group holding each shard, each listed once, in shard order and without
    /// committees. Each group is at its own latest epoch, so where the shard group layout changed and this indexer
    /// holds the new layout's checkpoints of only some groups, two listed groups can share shards. Empty while this
    /// indexer holds no checkpoint.
    pub checkpoints: Vec<EpochCheckpointEntry>,
}

/// A shard group's checkpoint of the end of an epoch: the commit proof of its end-of-epoch block, which the
/// committee of `epoch` signed.
///
/// Every field but `checkpoint` is read out of `checkpoint` for convenience, and is untrusted until the checkpoint
/// verifies against that committee.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct EpochCheckpointEntry {
    /// The epoch the checkpoint ends.
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub shard_group: ShardGroup,
    /// The validator set hash of the next epoch that the end-of-epoch block names, read from the header metadata
    /// preimage the checkpoint carries. Absent if the block names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub next_epoch_validator_set_hash: Option<ValidatorSetHash>,
    /// CBOR-encoded `EpochCheckpoint`.
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub checkpoint: Vec<u8>,
    /// The next epoch's committee for the requested shard, when `include_committees` was set and this indexer holds
    /// that epoch's validator set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub next_epoch_committee: Option<ValidatorSetCommittee>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetCommitteeRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    /// Any shard of the shard group whose committee to return.
    #[cfg_attr(feature = "utoipa", schema(value_type = u32))]
    pub shard: Shard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetCommitteeResponse {
    pub committee: ValidatorSetCommittee,
}

/// One shard group's committee in an epoch, with the proof that it is that shard group's committee in the validator
/// set `validator_set_hash` commits to.
///
/// The proof is only as good as the hash it is checked against: a verifier checks it against a validator set hash it
/// trusts, such as one a verified end-of-epoch checkpoint names, never against `validator_set_hash` alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ValidatorSetCommittee {
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub shard_group: ShardGroup,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub validator_set_hash: ValidatorSetHash,
    /// Ascending by public key.
    pub members: Vec<ValidatorSetCommitteeMember>,
    /// CBOR-encoded `ValidatorSetCommitteeProof`. It carries the members, and a verifier reads them from it.
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = String, format = Byte))]
    pub proof: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ValidatorSetCommitteeMember {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub public_key: RistrettoPublicKeyBytes,
    pub vote_power: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListWatchedSubstatesRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub template_address: Option<TemplateAddress>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub limit: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListWatchedSubstatesResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<WatchedSubstateItem>))]
    pub substates: Vec<WatchedSubstateItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct WatchedSubstateItem {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub component_address: SubstateId,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub template_address: TemplateAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListWatchedTemplatesResponse {
    pub templates: Vec<WatchedTemplateItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct WatchedTemplateItem {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub template_address: TemplateAddress,
    pub template_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use tari_engine_types::{
        confidential::ClaimedOutputTombstone,
        substate::{Substate, SubstateValue},
    };
    use tari_template_lib_types::ClaimedOutputTombstoneAddress;

    use super::*;

    fn substate_id(seed: u8) -> SubstateId {
        SubstateId::ClaimedOutputTombstone(ClaimedOutputTombstoneAddress::from_bytes(&[seed; 32]).unwrap())
    }

    fn substate() -> EncodedSubstate {
        EncodedSubstate::encode(&Substate::new(
            0,
            SubstateValue::ClaimedOutputTombstone(ClaimedOutputTombstone { value: 1 }),
        ))
        .unwrap()
    }

    fn proof(block: u8, seed: u8) -> SubstateProof {
        SubstateProof {
            anchor: SubstateProofAnchor {
                epoch: Epoch(3),
                shard_group: ShardGroup::new(0u32, 255u32),
                height: 7,
                block_id: Hash32::from_array([block; 32]),
                state_merkle_root: Hash32::from_array([block; 32]),
            },
            commit_proof: vec![block; 64],
            value_proof: vec![seed; 8],
            value_hash_epoch: Epoch(2),
            down_proof: seed.is_multiple_of(2).then(|| vec![seed; 4]),
        }
    }

    #[test]
    fn proofs_anchored_at_one_block_share_its_commit_proof() {
        let ids = [substate_id(1), substate_id(2), substate_id(3)];
        let substates = ids.iter().map(|id| (id.clone(), substate())).collect();
        let proofs = [
            (ids[0].clone(), proof(10, 1)),
            (ids[1].clone(), proof(10, 2)),
            (ids[2].clone(), proof(11, 3)),
        ];

        let resp = GetSubstatesResponse::new(substates, proofs.clone());
        assert_eq!(resp.commit_proofs.len(), 2);

        let json = serde_json::to_string(&resp).unwrap();
        let (substates, served) = serde_json::from_str::<GetSubstatesResponse>(&json)
            .unwrap()
            .into_proven();
        assert_eq!(substates.len(), 3);
        assert_eq!(served.len(), 3);
        for (id, proof) in proofs {
            let served = &served[&id];
            assert_eq!(served.anchor.block_id, proof.anchor.block_id);
            assert_eq!(served.commit_proof, proof.commit_proof);
            assert_eq!(served.value_proof, proof.value_proof);
            assert_eq!(served.value_hash_epoch, proof.value_hash_epoch);
            assert_eq!(served.down_proof, proof.down_proof);
        }
    }

    #[test]
    fn a_proof_whose_commit_proof_is_not_served_is_left_out() {
        let id = substate_id(1);
        let substates = [(id.clone(), substate())].into();
        let mut resp = GetSubstatesResponse::new(substates, [(id.clone(), proof(10, 1))]);
        resp.commit_proofs.clear();

        let (substates, proofs) = resp.into_proven();
        assert!(substates.contains_key(&id));
        assert!(proofs.is_empty());
    }
}
