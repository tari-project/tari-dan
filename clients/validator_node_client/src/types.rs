//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{ops::RangeInclusive, path::PathBuf, sync::Arc, time::Duration};

use indexmap::IndexMap;
use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use tari_base_node_client::types::BaseLayerValidatorNode;
use tari_common_types::types::FixedHash;
use tari_consensus_types::{BlockId, Decision};
use tari_engine_types::{
    commit_result::FinalizeResult,
    fees::FeeCostBreakdown,
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    StateVersion,
    SubstateAddress,
    committee::{Committee, CommitteeInfo},
    shard::Shard,
};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::{
    Ordering,
    consensus_models::{Block, TransactionExecution, TransactionPoolRecord},
    global::models,
    time::PrimitiveDateTime,
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_sidechain::QuorumDecision;
use tari_template_abi::{ArgDef, version::WasmAbiVersion};
use tari_template_lib_types::{TemplateAddress, crypto::RistrettoPublicKeyBytes};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNGetIdentityResponse")
)]
pub struct GetIdentityResponse {
    pub peer_id: String,
    pub public_key: RistrettoPublicKeyBytes,
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub public_addresses: Vec<Multiaddr>,
    pub supported_protocols: Vec<String>,
    pub protocol_version: String,
    pub user_agent: String,
    pub fee_claim_public_key: RistrettoPublicKeyBytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetTemplateRequest {
    pub template_address: TemplateAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetTemplateResponse {
    pub metadata: TemplateMetadata,
    pub abi: TemplateAbi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct TemplateAbi {
    pub template_name: String,
    pub functions: Vec<FunctionDef>,
    pub version: WasmAbiVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNFunctionDef")
)]
pub struct FunctionDef {
    pub name: String,
    pub arguments: Vec<ArgDef>,
    pub output: String,
    pub is_mut: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNTemplateMetadata")
)]
pub struct TemplateMetadata {
    pub name: String,
    pub address: TemplateAddress,
    pub code_size: usize,
}

/// A request to submit a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNSubmitTransactionRequest")
)]
pub struct SubmitTransactionRequest {
    pub transaction: Transaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNSubmitTransactionResponse")
)]
pub struct SubmitTransactionResponse {
    pub transaction_id: TransactionId,
    /// The result is a _dry run_ transaction.
    pub dry_run_result: Option<DryRunTransactionFinalizeResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct DryRunTransactionFinalizeResult {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub decision: QuorumDecision,
    pub finalize: FinalizeResult,
    pub fee_breakdown: Option<FeeCostBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNGetAllVnsRequest")
)]
pub struct GetAllVnsRequest {
    pub epoch: Epoch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNGetAllVnsResponse")
)]
pub struct GetAllVnsResponse {
    pub vns: Vec<BaseLayerValidatorNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetBaseLayerEpochChangesRequest {
    pub start_epoch: Epoch,
    pub end_epoch: Epoch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetBaseLayerEpochChangesResponse {
    pub changes: Vec<(Epoch, Vec<ValidatorNodeChange>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetConsensusStatusResponse {
    pub epoch: Epoch,
    pub height: NodeHeight,
    pub state: String,
    pub state_versions: Option<IndexMap<Shard, StateVersion>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
/// Represents a validator node state change
pub enum ValidatorNodeChange {
    Add {
        public_key: RistrettoPublicKeyBytes,
        activation_epoch: Epoch,
        minimum_value_promise: u64,
        shard_key: SubstateAddress,
    },
    Remove {
        public_key: RistrettoPublicKeyBytes,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetTransactionRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetTransactionResponse {
    pub transaction: Transaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "validator-node-client/",
        rename = "VNGetTransactionResultRequest"
    )
)]
pub struct GetTransactionResultRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "validator-node-client/",
        rename = "VNGetTransactionResultResponse"
    )
)]
pub struct GetTransactionResultResponse {
    pub transaction_execution: TransactionExecution,
    pub final_decision: Decision,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub finalize_at: PrimitiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct ListBlocksRequest {
    /// If provided, `limit` blocks from the specified block back will be returned. Otherwise `limit` blocks from the
    /// leaf block will be provided.
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub from_id: Option<BlockId>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct ListBlocksResponse {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetTxPoolResponse {
    pub tx_pool: Vec<TransactionPoolRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetBlockResponse {
    pub block: Block,
    /// Total WASM metering points consumed by transactions executed for this block.
    pub total_wasm_execution_points: u64,
    /// Total native verification points charged to transactions executed for this block.
    pub total_native_execution_points: u64,
    /// The per-block execution points budget a leader packs against. It governs the sum of the two totals above,
    /// so only that sum is a meaningful proportion of it.
    pub max_block_execution_points: u64,
    /// Total execution weight of the block's transaction commands: each transaction's weight discounted by how much
    /// of the work its command stage adds. Foreign proposal commands carry no execution weight and so
    /// contribute nothing.
    pub total_block_execution_weight: u64,
    /// The execution weight a block may carry and still be voted for. Weight stands in for the size, IO and storage
    /// cost that metering does not see, so for ordinary traffic this budget fills long before the execution points
    /// one.
    pub max_block_validation_weight: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetBlocksResponse {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetBlocksCountResponse {
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNLogEntry")
)]
pub struct LogEntry {
    pub timestamp: u64,
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNLogLevel")
)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetCommitteeRequest {
    pub epoch: Epoch,
    pub substate_address: SubstateAddress,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetCommitteeResponse {
    pub committee: Arc<Committee<PeerAddress>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetNetworkCommitteeResponse {
    pub current_epoch: Epoch,
    pub committees: Vec<CommitteeShardInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNCommitteeShardInfo")
)]
pub struct CommitteeShardInfo {
    pub shard: Shard,
    pub substate_address_range: RangeInclusive<SubstateAddress>,
    pub validators: Vec<ValidatorNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct ValidatorNode {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub address: PeerAddress,
    pub public_key: RistrettoPublicKeyBytes,
    pub shard_key: SubstateAddress,
    pub start_epoch: Epoch,
    pub fee_claim_public_key: RistrettoPublicKeyBytes,
}

impl From<models::ValidatorNode<PeerAddress>> for ValidatorNode {
    fn from(value: models::ValidatorNode<PeerAddress>) -> Self {
        Self {
            address: value.address,
            public_key: value.public_key,
            shard_key: value.shard_key,
            start_epoch: value.start_epoch,
            fee_claim_public_key: value.fee_claim_public_key,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetShardKeyRequest {
    pub epoch: Epoch,
    pub public_key: RistrettoPublicKeyBytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetShardKeyResponse {
    pub shard_key: Option<SubstateAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetStateRequest {
    pub address: SubstateAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetStateResponse {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNGetSubstateRequest")
)]
pub struct GetSubstateRequest {
    pub address: SubstateId,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNGetSubstateResponse")
)]
pub struct GetSubstateResponse {
    pub value: Option<SubstateValue>,
    pub status: SubstateStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub enum SubstateStatus {
    Up,
    Down,
    DoesNotExist,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNAddPeerRequest")
)]
pub struct AddPeerRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: RistrettoPublicKeyBytes,
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub addresses: Vec<Multiaddr>,
    pub wait_for_dial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNAddPeerResponse")
)]
pub struct AddPeerResponse {}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNGetCommsStatsResponse")
)]
pub struct GetCommsStatsResponse {
    pub connection_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetEpochManagerStatsResponse {
    pub current_epoch: Epoch,
    pub current_block_height: u64,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub current_block_hash: FixedHash,
    pub is_valid: bool,
    pub is_initial_scanning_complete: bool,
    pub start_epoch: Option<Epoch>,
    pub committee_info: Option<CommitteeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetBlockRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub block_id: BlockId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetBlocksRequest {
    pub limit: u32,
    pub offset: u32,
    pub ordering_index: Option<usize>,
    pub ordering: Option<Ordering>,
    pub filter_index: Option<usize>,
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetFilteredBlocksCountRequest {
    pub filter_index: Option<usize>,
    pub filter: Option<String>,
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNConnection")
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
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNConnectionDirection")
)]
pub enum ConnectionDirection {
    Inbound,
    Outbound,
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "validator-node-client/", rename = "VNGetConnectionsResponse")
)]
pub struct GetConnectionsResponse {
    pub connections: Vec<Connection>,
}

#[derive(Serialize, Debug)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct GetMempoolStatsResponse {
    pub size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct PrepareLayerOneTransactionRequest {
    pub params: LayerOneTransactionParams,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub enum LayerOneTransactionParams {
    Registration,
    Exit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "validator-node-client/"))]
pub struct PrepareLayerOneTransactionResponse {
    pub path: PathBuf,
}
