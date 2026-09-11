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
    Utxo,
    commit_result::ExecuteResult,
    events::Event,
    resource::Resource,
    substate::{Substate, SubstateId, SubstateValue},
    transaction_receipt::{FinalizeOutcome, TransactionReceipt},
};
use tari_ootle_common_types::{Epoch, NumPreshards, ShardGroup, StateVersion, shard::Shard};
use tari_ootle_template_metadata::MetadataHash;
use tari_ootle_transaction::{Network, PrunedTransaction, TransactionEnvelope, TransactionId};
use tari_template_abi::TemplateDef;
use tari_template_lib_types::{
    Amount,
    Hash32,
    NonFungibleAddress,
    ResourceAddress,
    TemplateAddress,
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
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
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
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub version: Option<u64>,
    #[serde(default)]
    pub local_search_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "tari-indexer-client/", rename = "IndexerGetSubstateResponse")
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetSubstateResponse {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub substate: SubstateValue,
    /// True when the indexer verified this substate's value against the shard group committee (via a
    /// merkle proof). False when proofs are disabled, or when no committee member could supply a
    /// proof yet (e.g. nothing committed since an epoch change) and the value was served unverified.
    pub verified: bool,
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetSubstatesResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = HashMap<String, Object>))]
    pub substates: HashMap<SubstateId, Substate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct InspectSubstateRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub address: SubstateId,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub version: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct InspectSubstateResponse {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
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
        rename = "IndexerSubmitTransactionResponse"
    )
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct SubmitTransactionDryRunResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    /// The ID of the transaction that was dry-run
    pub transaction_id: TransactionId,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    /// The result of the dry-run execution, including any emitted events and state changes, but without a final
    /// decision or commitment to the ledger
    pub result: ExecuteResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListTemplatesRequest {
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListTemplatesResponse {
    pub templates: Vec<TemplateMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct TemplateMeta {
    pub name: String,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub address: TemplateAddress,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub binary_sha: Hash32,
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub author_public_key: RistrettoPublicKeyBytes,
    pub code_size: usize,
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    /// Optional multihash of off-chain CBOR metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub metadata_hash: Option<MetadataHash>,
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
    pub metadata_hash: Option<MetadataHash>,
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
    /// Filter by resource address. Matches when either the event's `substate_id` is the given
    /// resource (std.resource.* events) or the event payload contains a `resource_address` entry
    /// equal to the given address (std.vault.deposit / std.vault.withdraw).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<String>))]
    pub resource_address: Option<ResourceAddress>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct QueryTransactionEventsResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(String, Object)>))]
    pub events: Vec<(TransactionId, Event)>,
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
    /// Filter by resource address. Matches when either the event's `substate_id` is the given
    /// resource (std.resource.* events) or the event payload contains a `resource_address` entry
    /// equal to the given address (std.vault.deposit / std.vault.withdraw).
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
        #[cfg_attr(feature = "utoipa", schema(value_type = Option<Object>))]
        execution_result: Option<Box<ExecuteResult>>,
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetNonFungiblesRequest {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub address: ResourceAddress,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub start_index: u64,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub end_index: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetNonFungiblesResponse {
    pub non_fungibles: Vec<NonFungibleSubstate>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct NonFungibleSubstate {
    #[cfg_attr(feature = "utoipa", schema(value_type = String))]
    pub address: NonFungibleAddress,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub substate: SubstateValue,
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
    /// The highest state version in `updates`. Every update at that version is included, so it is a
    /// resume point: a later request filtering on `state_version > max_state_version` loses nothing.
    pub max_state_version: StateVersion,
    pub max_epoch: Epoch,
    /// The shard holds further updates above `max_state_version`.
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
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UtxoBurnt {
    pub id: UtxoId,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
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
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(String, Object)>))]
    pub utxos: Vec<(UtxoId, Utxo)>,
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
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(String, Object)>))]
    pub utxos: Vec<(UtxoId, Utxo)>,
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
    /// record. `None` means transactions are retained indefinitely. Transaction receipts are retained
    /// regardless of this setting. A client paginating transaction history hits this floor rather
    /// than the start of the chain.
    pub transaction_retention_epochs: Option<u64>,
    /// Whether this indexer stores transactions observed on the network gossip topic in addition to
    /// those submitted directly to it. When false, a transaction submitted elsewhere is unknown to
    /// this indexer until its receipt is synced.
    ///
    /// Even when true the stored set of transactions is best effort: an indexer misses whatever was
    /// gossiped while it was offline or while its inbound queue was full, and there is no backfill.
    /// Transaction receipts carry no such caveat — they are synced from network state and are
    /// complete from genesis — so a committed transaction always has a receipt even when its body is
    /// missing here. Transactions that never committed get no receipt, so gossip is the only source
    /// for them.
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
    /// Per-validator consensus state as last observed by this indexer while
    /// syncing from the network. Populated lazily, so validators that have
    /// never been contacted for a sync will not appear here. Each entry
    /// carries an `observed_at_unix_s` timestamp so callers can judge whether
    /// the reading is fresh.
    #[serde(default)]
    pub validators: Vec<ValidatorStatus>,
}

/// A snapshot of one validator's consensus pacemaker state as observed by the
/// indexer during a recent sync round.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ValidatorStatus {
    /// libp2p PeerId of the validator.
    pub peer_id: String,
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub shard_group: ShardGroup,
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub epoch: Epoch,
    #[cfg_attr(feature = "utoipa", schema(value_type = u64))]
    pub height: u64,
    /// The validator's self-reported consensus pacemaker state. Diagnostic only - this is not
    /// verified, so it should not be relied upon for anything but display.
    pub state: ValidatorConsensusState,
    /// Unix timestamp (seconds) at which this snapshot was captured. Clients
    /// can derive the freshness of the snapshot by comparing this to the
    /// current wall-clock time.
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
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<(String, Object)>))]
    pub receipts: Vec<(TransactionReceiptAddress, TransactionReceipt)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetTransactionReceiptResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub receipt: TransactionReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetResourceResponse {
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub resource: Resource,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
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
    /// Number of transaction receipts the indexer has stored.
    pub transaction_receipt_count: u64,
    /// The share of collected fees burned rather than paid to leaders, in basis points, in effect at `current_epoch`.
    pub target_burn_rate_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListEpochCheckpointsRequest {
    /// The epoch to start listing from (inclusive). Defaults to 0.
    #[cfg_attr(feature = "utoipa", schema(value_type = Option<u64>))]
    pub from_epoch: Option<Epoch>,
    /// Maximum number of checkpoints to return (default: 20, max: 100).
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ListEpochCheckpointsResponse {
    #[cfg_attr(feature = "ts", ts(type = "Array<Record<string, unknown>>"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Vec<Object>))]
    pub checkpoints: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "tari-indexer-client/"))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct GetLatestEpochCheckpointResponse {
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
    pub checkpoint: serde_json::Value,
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
