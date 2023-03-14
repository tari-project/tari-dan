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

use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{
    quorum_certificate::{QuorumCertificate, QuorumDecision},
    serde_with,
    Epoch,
    ShardId,
};
use tari_dan_core::models::RecentTransaction;
use tari_engine_types::{
    commit_result::FinalizeResult,
    substate::{SubstateAddress, SubstateValue},
    TemplateAddress,
};
use tari_transaction::Transaction;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetIdentityResponse {
    pub node_id: String,
    pub public_key: String,
    pub public_address: String,
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
    pub arguments: Vec<String>,
    pub output: String,
    pub is_mut: bool,
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
/// ```json
/// instructions": [{
///    "type": "CallFunction",
///    "template_address": "55886cfee6e91503b7f1df2dc6d11951b53db64733521595c3505747b83be277",
///    "function": "new",
///    "args": [{
///       "type":"Literal",
///       "value": "1232"
///    }]
///  }],
///  "signature": {
///    "public_nonce": "90392b9cebd7bf7d693f938911ccd3fb735a6cf24fcf1341a2edca38c560b563",
///    "signature": "90392b9cebd7bf7d693f938911ccd3fb735a6cf24fcf1341a2edca38c560b563"
///   },
///   "fee": 1,
///   "sender_public_key": "90392b9cebd7bf7d693f938911ccd3fb735a6cf24fcf1341a2edca38c560b563",
///   "num_new_components": 1
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionRequest {
    pub transaction: Transaction,
    /// Set to true to wait for the transaction to complete before returning
    #[serde(default)]
    pub wait_for_result: bool,
    #[serde(default)]
    pub wait_for_result_timeout: Option<u64>,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub result: Option<TransactionFinalizeResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionFinalizeResult {
    // TODO: we should not return the whole state but only the addresses and perhaps a hash of the state
    pub decision: QuorumDecision,
    pub finalize: FinalizeResult,
    pub qc: QuorumCertificate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRequest {
    pub payload_id: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstatesRequest {
    pub payload_id: Vec<u8>,
    pub shard_id: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionResultRequest {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionResultResponse {
    pub result: Option<FinalizeResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionQcsRequest {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionQcsResponse {
    pub qcs: Vec<QuorumCertificate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRecentTransactionsRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRecentTransactionsResponse {
    pub transactions: Vec<RecentTransaction>,
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
}
