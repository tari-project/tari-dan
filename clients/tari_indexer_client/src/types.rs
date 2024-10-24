//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{sync::Arc, time::Duration};

use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use serde_with::{serde_as, DisplayFromStr};
use tari_base_node_client::types::BaseLayerValidatorNode;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{substate_type::SubstateType, Epoch, SubstateRequirement};
use tari_dan_storage::consensus_models::Decision;
use tari_engine_types::{
    commit_result::ExecuteResult,
    serde_with as serde_tools,
    substate::{Substate, SubstateId},
    TemplateAddress,
};
use tari_template_abi::TemplateDef;
use tari_transaction::{Transaction, TransactionId};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct ListSubstatesRequest {
    #[serde(default, deserialize_with = "serde_tools::string::option::deserialize")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub filter_by_template: Option<TemplateAddress>,
    pub filter_by_type: Option<SubstateType>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct ListSubstatesResponse {
    pub substates: Vec<ListSubstateItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct ListSubstateItem {
    pub substate_id: SubstateId,
    pub module_name: Option<String>,
    pub version: u32,
    #[serde(default, with = "serde_tools::string::option")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub template_address: Option<TemplateAddress>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetSubstateRequest"
    )
)]
pub struct GetSubstateRequest {
    #[serde(with = "serde_tools::string")]
    pub address: SubstateId,
    pub version: Option<u32>,
    #[serde(default)]
    pub local_search_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetSubstateResponse"
    )
)]
pub struct GetSubstateResponse {
    #[serde(with = "serde_tools::string")]
    pub address: SubstateId,
    pub version: u32,
    pub substate: Substate,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub created_by_transaction: TransactionId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct InspectSubstateRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub version: Option<u32>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct InspectSubstateResponse {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub version: u32,
    pub substate: Substate,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub created_by_transaction: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerSubmitTransactionRequest"
    )
)]
pub struct SubmitTransactionRequest {
    pub transaction: Transaction,
    #[serde(default)]
    pub required_substates: Vec<SubstateRequirement>,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerSubmitTransactionResponse"
    )
)]
pub struct SubmitTransactionResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub result: IndexerTransactionFinalizedResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct ListTemplatesRequest {
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct ListTemplatesResponse {
    pub templates: Vec<TemplateMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct TemplateMetadata {
    pub name: String,
    #[cfg_attr(feature = "ts", ts(type = "Uint8Array"))]
    pub address: TemplateAddress,
    pub url: String,
    /// SHA hash of binary
    pub binary_sha: String,
    /// Block height in which the template was published
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub height: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetTransactionResultRequest"
    )
)]
pub struct GetTransactionResultRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetTransactionResultResponse"
    )
)]
pub struct GetTransactionResultResponse {
    pub result: IndexerTransactionFinalizedResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub enum IndexerTransactionFinalizedResult {
    Pending,
    Finalized {
        final_decision: Decision,
        execution_result: Option<Box<ExecuteResult>>,
        #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number}"))]
        execution_time: Duration,
        #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number}"))]
        finalized_time: Duration,
        abort_details: Option<String>,
        #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
        json_results: Vec<JsonValue>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetIdentityResponse"
    )
)]
pub struct GetIdentityResponse {
    pub peer_id: String,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub public_addresses: Vec<Multiaddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetAllVnsRequest"
    )
)]
pub struct GetAllVnsRequest {
    pub epoch: Epoch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetAllVnsResponse"
    )
)]
pub struct GetAllVnsResponse {
    pub vns: Vec<BaseLayerValidatorNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetNonFungibleCollectionsResponse {
    #[cfg_attr(feature = "ts", ts(type = "Array<[string, number]>"))]
    pub collections: Vec<(String, i64)>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetNonFungibleCountRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetNonFungibleCountResponse {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub count: u64,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetNonFungiblesRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub start_index: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub end_index: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetNonFungiblesResponse {
    pub non_fungibles: Vec<NonFungibleSubstate>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct NonFungibleSubstate {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub index: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub substate: Substate,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetRelatedTransactionsRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetRelatedTransactionsResponse {
    pub transaction_results: Vec<IndexerTransactionFinalizedResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerAddPeerRequest"
    )
)]
pub struct AddPeerRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub addresses: Vec<Multiaddr>,
    pub wait_for_dial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerAddPeerResponse"
    )
)]
pub struct AddPeerResponse {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetCommsStatsResponse"
    )
)]
pub struct GetCommsStatsResponse {
    pub connection_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetEpochManagerStatsResponse"
    )
)]
pub struct GetEpochManagerStatsResponse {
    pub current_epoch: Epoch,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub current_block_height: u64,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub current_block_hash: FixedHash,
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerConnection"
    )
)]
pub struct Connection {
    pub connection_id: String,
    pub peer_id: String,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub address: Multiaddr,
    pub direction: ConnectionDirection,
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number}"))]
    pub age: Duration,
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number} | null"))]
    pub ping_latency: Option<Duration>,
    pub user_agent: Option<Arc<String>>,
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerConnectionDirection"
    )
)]
pub enum ConnectionDirection {
    Inbound,
    Outbound,
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/tari-indexer-client/",
        rename = "IndexerGetConnectionsResponse"
    )
)]
pub struct GetConnectionsResponse {
    pub connections: Vec<Connection>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetTemplateDefinitionRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "serde_tools::string")]
    pub template_address: TemplateAddress,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/tari-indexer-client/")
)]
pub struct GetTemplateDefinitionResponse {
    pub name: String,
    pub definition: TemplateDef,
}
