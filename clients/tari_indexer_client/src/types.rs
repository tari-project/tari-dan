//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use serde_with::{serde_as, DisplayFromStr};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::Epoch;
use tari_dan_storage::consensus_models::Decision;
use tari_engine_types::{
    commit_result::ExecuteResult,
    serde_with as serde_tools,
    substate::{Substate, SubstateId},
};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstateRequest {
    #[serde(with = "serde_tools::string")]
    pub address: SubstateId,
    pub version: Option<u32>,
    #[serde(default)]
    pub local_search_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstateResponse {
    #[serde(with = "serde_tools::string")]
    pub address: SubstateId,
    pub version: u32,
    pub substate: Substate,
    pub created_by_transaction: TransactionId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectSubstateRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub version: Option<u32>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectSubstateResponse {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub version: u32,
    pub substate_contents: serde_json::Value,
    pub created_by_transaction: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionRequest {
    pub transaction: Transaction,
    pub required_substates: Vec<SubstateRequirement>,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionResponse {
    pub transaction_id: TransactionId,
    pub result: IndexerTransactionFinalizedResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionResultRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionResultResponse {
    pub result: IndexerTransactionFinalizedResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexerTransactionFinalizedResult {
    Pending,
    Finalized {
        final_decision: Decision,
        execution_result: Option<ExecuteResult>,
        abort_details: Option<String>,
        json_results: Vec<JsonValue>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetIdentityResponse {
    pub peer_id: String,
    pub public_key: PublicKey,
    pub public_addresses: Vec<Multiaddr>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddAddressRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteAddressRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetNonFungibleCountRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetNonFungiblesRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub start_index: u64,
    pub end_index: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetNonFungiblesResponse {
    pub non_fungibles: Vec<NonFungibleSubstate>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct NonFungibleSubstate {
    pub index: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub substate: Substate,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRelatedTransactionsRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateId,
    pub version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRelatedTransactionsResponse {
    pub transaction_results: Vec<IndexerTransactionFinalizedResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddPeerRequest {
    pub public_key: PublicKey,
    pub addresses: Vec<Multiaddr>,
    pub wait_for_dial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddPeerResponse {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetEpochManagerStatsResponse {
    pub current_epoch: Epoch,
    pub current_block_height: u64,
}

#[derive(Serialize, Debug)]
pub struct Connection {
    pub connection_id: String,
    pub peer_id: String,
    pub address: Multiaddr,
    pub direction: ConnectionDirection,
    pub age: Duration,
    pub ping_latency: Option<Duration>,
}

#[derive(Serialize, Debug)]
pub enum ConnectionDirection {
    Inbound,
    Outbound,
}

#[derive(Serialize, Debug)]
pub struct GetConnectionsResponse {
    pub connections: Vec<Connection>,
}
