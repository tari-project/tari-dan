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

use std::{ops::RangeInclusive, time::Duration};

use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use tari_base_node_client::types::BaseLayerValidatorNode;
use tari_common_types::{
    transaction::TxId,
    types::{FixedHash, PublicKey},
};
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    shard::Shard,
    Epoch,
    PeerAddress,
    SubstateAddress,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Decision,
        ExecutedTransaction,
        QuorumDecision,
        SubstateRecord,
        TransactionPoolRecord,
    },
    global::models,
    Ordering,
};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult},
    fees::FeeCostBreakdown,
    serde_with,
    substate::{SubstateId, SubstateValue},
    TemplateAddress,
};
use tari_transaction::{Transaction, TransactionId};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetIdentityResponse {
    pub peer_id: String,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub public_addresses: Vec<Multiaddr>,
    pub supported_protocols: Vec<String>,
    pub protocol_version: String,
    pub user_agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct TemplateRegistrationRequest {
    pub template_name: String,
    pub template_version: u16,
    pub repo_url: String,
    #[serde(with = "serde_with::base64")]
    pub commit_hash: Vec<u8>,
    #[serde(with = "serde_with::base64")]
    pub binary_sha: Vec<u8>,
    pub binary_url: String,
    pub template_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct TemplateRegistrationResponse {
    #[serde(with = "serde_with::base64")]
    pub template_address: Vec<u8>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub transaction_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTemplateRequest {
    #[cfg_attr(feature = "ts", ts(type = "Uint8Array"))]
    pub template_address: TemplateAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTemplateResponse {
    pub registration_metadata: TemplateMetadata,
    pub abi: TemplateAbi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct TemplateAbi {
    pub template_name: String,
    pub functions: Vec<FunctionDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
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
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct ArgDef {
    pub name: String,
    pub arg_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTemplatesRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTemplatesResponse {
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
    pub binary_sha: Vec<u8>,
    /// Block height in which the template was published
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub height: u64,
}

/// A request to submit a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct SubmitTransactionRequest {
    pub transaction: Transaction,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct SubmitTransactionResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    /// The result is a _dry run_ transaction.
    pub dry_run_result: Option<DryRunTransactionFinalizeResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct DryRunTransactionFinalizeResult {
    pub decision: QuorumDecision,
    pub finalize: FinalizeResult,
    pub fee_breakdown: Option<FeeCostBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetAllVnsRequest {
    pub epoch: Epoch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetAllVnsResponse {
    pub vns: Vec<BaseLayerValidatorNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTransactionRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTransactionResponse {
    pub transaction: ExecutedTransaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetSubstatesByTransactionRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetSubstatesByTransactionResponse {
    pub substates: Vec<SubstateRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTransactionResultRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTransactionResultResponse {
    pub result: Option<ExecuteResult>,
    pub final_decision: Option<Decision>,
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number} | null"))]
    pub finalized_time: Option<Duration>,
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number} | null"))]
    pub execution_time: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetRecentTransactionsRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetRecentTransactionsResponse {
    pub transactions: Vec<Transaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct ListBlocksRequest {
    /// If provided, `limit` blocks from the specified block back will be returned. Otherwise `limit` blocks from the
    /// leaf block will be provided.
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub from_id: Option<BlockId>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct ListBlocksResponse {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetTxPoolResponse {
    pub tx_pool: Vec<TransactionPoolRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetBlockResponse {
    pub block: Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetBlocksResponse {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetBlocksCountResponse {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct LogEntry {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub timestamp: u64,
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetCommitteeRequest {
    pub epoch: Epoch,
    pub substate_address: SubstateAddress,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetCommitteeResponse {
    pub committee: Committee<PeerAddress>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetNetworkCommitteeResponse {
    pub current_epoch: Epoch,
    pub committees: Vec<CommitteeShardInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct CommitteeShardInfo {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub shard: Shard,
    pub substate_address_range: RangeInclusive<SubstateAddress>,
    pub validators: Vec<ValidatorNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct ValidatorNode {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub address: PeerAddress,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    pub shard_key: SubstateAddress,
    pub epoch: Epoch,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub committee_shard: Option<Shard>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub fee_claim_public_key: PublicKey,
}

impl From<models::ValidatorNode<PeerAddress>> for ValidatorNode {
    fn from(value: models::ValidatorNode<PeerAddress>) -> Self {
        Self {
            address: value.address,
            public_key: value.public_key,
            shard_key: value.shard_key,
            epoch: value.epoch,
            committee_shard: value.committee_shard,
            fee_claim_public_key: value.fee_claim_public_key,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetShardKeyRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub height: u64,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetShardKeyResponse {
    pub shard_key: Option<SubstateAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetStateRequest {
    pub address: SubstateAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetStateResponse {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetSubstateRequest {
    pub address: SubstateId,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetSubstateResponse {
    pub value: Option<SubstateValue>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub created_by_tx: Option<TransactionId>,
    pub status: SubstateStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub enum SubstateStatus {
    Up,
    Down,
    DoesNotExist,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
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
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct AddPeerResponse {}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetCommsStatsResponse {
    pub connection_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetEpochManagerStatsResponse {
    pub current_epoch: Epoch,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub current_block_height: u64,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub current_block_hash: FixedHash,
    pub is_valid: bool,
    pub committee_shard: Option<CommitteeShard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct RegisterValidatorNodeRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub fee_claim_public_key: PublicKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct RegisterValidatorNodeResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TxId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/validator-node-client/VNGetValidatorFeesRequest.ts"
    ),
    ts(rename = "VNGetValidatorFeesRequest")
)]
pub struct GetValidatorFeesRequest {
    pub epoch_range: RangeInclusive<Epoch>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub validator_public_key: Option<PublicKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(
        export,
        export_to = "../../bindings/src/types/validator-node-client/VNGetValidatorFeesResponse.ts"
    ),
    ts(rename = "VNGetValidatorFeesResponse")
)]
pub struct GetValidatorFeesResponse {
    pub fees: Vec<ValidatorFee>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct ValidatorFee {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub validator_public_key: PublicKey,
    pub epoch: Epoch,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub block_id: BlockId,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total_fee_due: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total_transaction_fee: u64,
}

impl From<Block> for ValidatorFee {
    fn from(value: Block) -> Self {
        Self {
            validator_public_key: value.proposed_by().clone(),
            epoch: value.epoch(),
            block_id: *value.id(),
            total_fee_due: value.total_leader_fee(),
            total_transaction_fee: value
                .commands()
                .iter()
                .filter_map(|c| c.accept())
                .map(|t| t.transaction_fee)
                .sum(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetBlockRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub block_id: BlockId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetBlocksRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub limit: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub offset: u64,
    pub ordering: Option<Ordering>,
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
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
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub enum ConnectionDirection {
    Inbound,
    Outbound,
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetConnectionsResponse {
    pub connections: Vec<Connection>,
}

#[derive(Serialize, Debug)]
#[cfg_attr(
    feature = "ts",
    derive(TS),
    ts(export, export_to = "../../bindings/src/types/validator-node-client/")
)]
pub struct GetMempoolStatsResponse {
    pub size: usize,
}
