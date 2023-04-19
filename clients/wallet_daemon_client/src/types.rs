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

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{serde_with, QuorumCertificate, ShardId};
use tari_dan_wallet_sdk::models::{Account, ConfidentialProofId, TransactionStatus, VersionedSubstateAddress};
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason},
    instruction::Instruction,
    instruction_result::InstructionResult,
    substate::SubstateAddress,
    TemplateAddress,
};
use tari_template_lib::{
    args::Arg,
    auth::AccessRules,
    models::{Amount, ComponentAddress, ConfidentialOutputProof, NonFungibleId, ResourceAddress},
    prelude::{ConfidentialWithdrawProof, ResourceType},
};
use tari_transaction::Transaction;

use crate::serialize::string_or_struct;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionSubmitRequest {
    pub signing_key_index: Option<u64>,
    pub fee_instructions: Vec<Instruction>,
    pub instructions: Vec<Instruction>,
    pub inputs: Vec<VersionedSubstateAddress>,
    pub override_inputs: bool,
    pub new_outputs: u8,
    pub specific_non_fungible_outputs: Vec<(ResourceAddress, NonFungibleId)>,
    pub new_resources: Vec<(TemplateAddress, String)>,
    pub new_non_fungible_outputs: Vec<(ResourceAddress, u8)>,
    pub new_non_fungible_index_outputs: Vec<(ResourceAddress, u64)>,
    pub is_dry_run: bool,
    pub proof_ids: Vec<ConfidentialProofId>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionSubmitResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub inputs: Vec<ShardId>,
    pub outputs: Vec<ShardId>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionGetRequest {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionClaimBurnRequest {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionGetResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub transaction: Transaction,
    pub result: Option<FinalizeResult>,
    pub status: TransactionStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionGetResultRequest {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionGetResultResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub result: Option<FinalizeResult>,
    // TODO: Always None
    pub qc: Option<QuorumCertificate<PublicKey>>,
    pub status: TransactionStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionWaitResultRequest {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionWaitResultResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub result: Option<FinalizeResult>,
    pub qcs: Vec<QuorumCertificate<PublicKey>>,
    pub status: TransactionStatus,
    pub transaction_failure: Option<RejectReason>,
    pub final_fee: Amount,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionClaimBurnResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub inputs: Vec<ShardId>,
    pub outputs: Vec<ShardId>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthLoginRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeysListRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeysListResponse {
    pub keys: Vec<(u64, PublicKey, bool)>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeysSetActiveRequest {
    pub index: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeysSetActiveResponse {
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeysCreateRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeysCreateResponse {
    pub id: u64,
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsCreateRequest {
    pub account_name: Option<String>,
    pub signing_key_index: Option<u64>,
    pub custom_access_rules: Option<AccessRules>,
    pub fee: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsCreateResponse {
    pub address: SubstateAddress,
    pub public_key: PublicKey,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsInvokeRequest {
    pub account_name: String,
    pub method: String,
    pub args: Vec<Arg>,
    pub fee: Amount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsInvokeResponse {
    pub result: Option<InstructionResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsListRequest {
    pub offset: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountInfo {
    pub account: Account,
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsListResponse {
    pub accounts: Vec<AccountInfo>,
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsGetBalancesRequest {
    pub account_name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsGetBalancesResponse {
    pub address: SubstateAddress,
    pub balances: Vec<BalanceEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BalanceEntry {
    pub vault_address: SubstateAddress,
    pub resource_address: ResourceAddress,
    pub balance: Amount,
    pub resource_type: ResourceType,
    pub confidential_balance: Amount,
    pub token_symbol: Option<String>,
}

impl BalanceEntry {
    pub fn to_balance_string(&self) -> String {
        let symbol = self.token_symbol.as_deref().unwrap_or_default();
        match self.resource_type {
            ResourceType::Fungible => {
                format!("{} {}", self.balance, symbol)
            },
            ResourceType::NonFungible => {
                format!("{} {} tokens", self.balance, symbol)
            },
            ResourceType::Confidential => {
                format!(
                    "{} revealed + {} blinded = {} {}",
                    self.balance,
                    self.confidential_balance,
                    self.balance + self.confidential_balance,
                    symbol
                )
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountByNameRequest {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountByNameResponse {
    pub account: Account,
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProofsGenerateRequest {
    pub amount: Amount,
    pub reveal_amount: Amount,
    pub source_account_name: String,
    #[serde(deserialize_with = "string_or_struct")]
    pub resource_address: ResourceAddress,
    // TODO: For now, we assume that this is obtained "somehow" from the destination account
    pub destination_public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProofsGenerateResponse {
    pub proof_id: ConfidentialProofId,
    pub proof: ConfidentialWithdrawProof,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProofsFinalizeRequest {
    pub proof_id: ConfidentialProofId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProofsFinalizeResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProofsCancelRequest {
    pub proof_id: ConfidentialProofId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConfidentialCreateOutputProofRequest {
    pub amount: Amount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConfidentialCreateOutputProofResponse {
    pub proof: ConfidentialOutputProof,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConfidentialTransferRequest {
    #[serde(deserialize_with = "string_or_struct")]
    pub account: ComponentAddress,
    pub amount: Amount,
    pub resource_address: ResourceAddress,
    pub destination_account: ComponentAddress,
    pub destination_public_key: PublicKey,
    pub fee: Amount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConfidentialTransferResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub fee: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClaimBurnRequest {
    #[serde(deserialize_with = "string_or_struct")]
    pub account: ComponentAddress,
    pub claim_proof: serde_json::Value,
    pub fee: Amount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClaimBurnResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub fee: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProofsCancelResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RevealFundsRequest {
    #[serde(deserialize_with = "string_or_struct")]
    /// Account with funds to reveal
    pub account: ComponentAddress,
    /// Amount to reveal
    pub amount_to_reveal: Amount,
    /// Pay fee from revealed funds. If false, previously revealed funds in the account are used.
    pub pay_fee_from_reveal: bool,
    /// The amount of fees to add to the transaction. Any fees not charged are refunded.
    pub fee: Amount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RevealFundsResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub fee: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsCreateFreeTestCoinsRequest {
    pub account_name: String,
    pub amount: Amount,
    pub fee: Amount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountsCreateFreeTestCoinsResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub amount: Amount,
    pub fee: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebRtcStart {
    pub jwt: String,
}
