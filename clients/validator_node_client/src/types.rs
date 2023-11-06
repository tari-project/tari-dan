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

use std::ops::RangeInclusive;

use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use tari_common_types::{transaction::TxId, types::PublicKey};
use tari_dan_common_types::{committee::CommitteeShard, shard_bucket::ShardBucket, Epoch, ShardId};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, ExecutedTransaction, QuorumDecision, SubstateRecord},
    global::models::ValidatorNode,
    Ordering,
};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult},
    fees::FeeCostBreakdown,
    serde_with,
    substate::{SubstateAddress, SubstateValue},
    TemplateAddress,
};
use tari_transaction::{Transaction, TransactionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetIdentityResponse {
    pub node_id: String,
    pub public_key: PublicKey,
    pub public_addresses: Vec<Multiaddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct TemplateRegistrationResponse {
    #[serde(with = "serde_with::base64")]
    pub template_address: Vec<u8>,
    pub transaction_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTemplateRequest {
    pub template_address: TemplateAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTemplateResponse {
    pub registration_metadata: TemplateMetadata,
    pub abi: TemplateAbi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateAbi {
    pub template_name: String,
    pub functions: Vec<FunctionDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub arguments: Vec<ArgDef>,
    pub output: String,
    pub is_mut: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgDef {
    pub name: String,
    pub arg_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTemplatesRequest {
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTemplatesResponse {
    pub templates: Vec<TemplateMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMetadata {
    pub name: String,
    pub address: TemplateAddress,
    pub url: String,
    /// SHA hash of binary
    pub binary_sha: Vec<u8>,
    /// Block height in which the template was published
    pub height: u64,
}

/// A request to submit a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionRequest {
    pub transaction: Transaction,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionResponse {
    pub transaction_id: TransactionId,
    /// The result is a _dry run_ transaction.
    pub dry_run_result: Option<DryRunTransactionFinalizeResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunTransactionFinalizeResult {
    // TODO: we should not return the whole state but only the addresses and perhaps a hash of the state
    pub decision: QuorumDecision,
    pub finalize: FinalizeResult,
    pub fee_breakdown: Option<FeeCostBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetTransactionResponse {
    pub transaction: ExecutedTransaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstatesByTransactionRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstatesByTransactionResponse {
    pub substates: Vec<SubstateRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionResultRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionResultResponse {
    pub result: Option<ExecuteResult>,
    pub is_finalized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRecentTransactionsRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRecentTransactionsResponse {
    pub transactions: Vec<Transaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListBlocksRequest {
    /// If provided, `limit` blocks from the specified block back will be returned. Otherwise `limit` blocks from the
    /// leaf block will be provided.
    pub from_id: Option<BlockId>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListBlocksResponse {
    pub blocks: Vec<Block<PublicKey>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockResponse {
    pub block: Block<PublicKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlocksResponse {
    pub blocks: Vec<Block<PublicKey>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlocksCountResponse {
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: u64,
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetCommitteeRequest {
    pub epoch: Epoch,
    pub shard_id: ShardId,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetNetworkCommitteeResponse {
    pub current_epoch: Epoch,
    pub committees: Vec<CommitteeShardInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitteeShardInfo {
    pub bucket: ShardBucket,
    pub shard_range: RangeInclusive<ShardId>,
    pub validators: Vec<ValidatorNode<PublicKey>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetShardKey {
    pub height: u64,
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetStateRequest {
    pub shard_id: ShardId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetStateResponse {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstateRequest {
    pub address: SubstateAddress,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstateResponse {
    pub value: Option<SubstateValue>,
    pub created_by_tx: Option<TransactionId>,
    pub status: SubstateStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SubstateStatus {
    Up,
    Down,
    DoesNotExist,
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
    pub is_valid: bool,
    pub committee_shard: Option<CommitteeShard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterValidatorNodeRequest {
    pub fee_claim_public_key: PublicKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterValidatorNodeResponse {
    pub transaction_id: TxId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetValidatorFeesRequest {
    pub epoch_range: RangeInclusive<Epoch>,
    pub validator_public_key: Option<PublicKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetValidatorFeesResponse {
    pub fees: Vec<ValidatorFee<PublicKey>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorFee<TAddr> {
    pub validator_addr: TAddr,
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub total_fee_due: u64,
    pub total_transaction_fee: u64,
}

impl<TAddr: Clone> From<Block<TAddr>> for ValidatorFee<TAddr> {
    fn from(value: Block<TAddr>) -> Self {
        Self {
            validator_addr: value.proposed_by().clone(),
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
pub struct GetBlockRequest {
    pub block_id: BlockId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlocksRequest {
    pub limit: u64,
    pub offset: u64,
    pub ordering: Option<Ordering>,
}
