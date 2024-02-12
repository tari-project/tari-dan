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

use std::{collections::HashMap, time::Duration};

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, SubstateAddress};
use tari_dan_wallet_sdk::{
    apis::jwt::Claims,
    models::{Account, ConfidentialProofId, TransactionStatus},
};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult},
    instruction::Instruction,
    instruction_result::InstructionResult,
    serde_with,
    substate::SubstateId,
};
use tari_template_lib::{
    args::Arg,
    auth::ComponentAccessRules,
    models::{Amount, ConfidentialOutputProof, NonFungibleId, ResourceAddress},
    prelude::{ComponentAddress, ConfidentialWithdrawProof, ResourceType},
};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    serialize::{opt_string_or_struct, string_or_struct},
    ComponentAddressOrName,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct CallInstructionRequest {
    pub instructions: Vec<Instruction>,
    #[serde(deserialize_with = "string_or_struct")]
    pub fee_account: ComponentAddressOrName,
    #[serde(default, deserialize_with = "opt_string_or_struct")]
    pub dump_outputs_into: Option<ComponentAddressOrName>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_fee: u64,
    #[serde(default)]
    pub inputs: Vec<SubstateRequirement>,
    #[serde(default)]
    pub override_inputs: Option<bool>,
    #[serde(default)]
    pub new_outputs: Option<u8>,
    #[serde(default)]
    pub is_dry_run: bool,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub proof_ids: Vec<ConfidentialProofId>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub min_epoch: Option<u64>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub max_epoch: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionSubmitRequest {
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub signing_key_index: Option<u64>,
    pub fee_instructions: Vec<Instruction>,
    pub instructions: Vec<Instruction>,
    pub inputs: Vec<SubstateRequirement>,
    pub override_inputs: bool,
    pub is_dry_run: bool,
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub proof_ids: Vec<ConfidentialProofId>,
    pub min_epoch: Option<Epoch>,
    pub max_epoch: Option<Epoch>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionSubmitResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub inputs: Vec<SubstateRequirement>,
    pub result: Option<ExecuteResult>,
    #[cfg_attr(feature = "ts", ts(type = "Array<any> | null"))]
    pub json_result: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionGetRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionGetResponse {
    pub transaction: Transaction,
    pub result: Option<FinalizeResult>,
    pub status: TransactionStatus,
    pub last_update_time: NaiveDateTime,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionGetAllRequest {
    pub status: Option<TransactionStatus>,
    pub component: Option<ComponentAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionGetAllResponse {
    pub transactions: Vec<(Transaction, Option<FinalizeResult>, TransactionStatus, NaiveDateTime)>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionGetResultRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionGetResultResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub status: TransactionStatus,
    pub result: Option<FinalizeResult>,
    #[cfg_attr(feature = "ts", ts(type = "Array<any> | null"))]
    pub json_result: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionWaitResultRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionWaitResultResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub result: Option<FinalizeResult>,
    #[cfg_attr(feature = "ts", ts(type = "Array<any> | null"))]
    pub json_result: Option<Vec<Value>>,
    pub status: TransactionStatus,
    pub final_fee: Amount,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionClaimBurnResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub inputs: Vec<SubstateAddress>,
    pub outputs: Vec<SubstateAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct KeysListRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct KeysListResponse {
    /// (index, public key, is_active)
    #[cfg_attr(feature = "ts", ts(type = "Array<[number, string, boolean]>"))]
    pub keys: Vec<(u64, PublicKey, bool)>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct KeysSetActiveRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub index: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct KeysSetActiveResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct KeysCreateRequest {
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub specific_index: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct KeysCreateResponse {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: u64,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsCreateRequest {
    pub account_name: Option<String>,
    pub custom_access_rules: Option<ComponentAccessRules>,
    pub max_fee: Option<Amount>,
    pub is_default: bool,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub key_id: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsCreateResponse {
    pub address: SubstateId,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsInvokeRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    pub method: String,
    pub args: Vec<Arg>,
    pub max_fee: Option<Amount>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsInvokeResponse {
    pub result: Option<InstructionResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsListRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub offset: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub limit: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountInfo {
    pub account: Account,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsListResponse {
    pub accounts: Vec<AccountInfo>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsGetBalancesRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsGetBalancesResponse {
    pub address: SubstateId,
    pub balances: Vec<BalanceEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct BalanceEntry {
    pub vault_address: SubstateId,
    #[serde(with = "serde_with::string")]
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
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountGetRequest {
    #[serde(deserialize_with = "string_or_struct")]
    pub name_or_address: ComponentAddressOrName,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountGetDefaultRequest {
    // Intentionally empty. Fields may be added in the future.
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountGetResponse {
    pub account: Account,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountSetDefaultRequest {
    #[serde(deserialize_with = "string_or_struct")]
    pub account: ComponentAddressOrName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountSetDefaultResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransferRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    pub amount: Amount,
    pub resource_address: ResourceAddress,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub destination_public_key: PublicKey,
    pub max_fee: Option<Amount>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransferResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub fee: Amount,
    pub fee_refunded: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ProofsGenerateRequest {
    pub amount: Amount,
    pub reveal_amount: Amount,
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    // TODO: #[serde(deserialize_with = "string_or_struct")]
    pub resource_address: ResourceAddress,
    // TODO: For now, we assume that this is obtained "somehow" from the destination account
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub destination_public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ProofsGenerateResponse {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub proof_id: ConfidentialProofId,
    pub proof: ConfidentialWithdrawProof,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ProofsFinalizeRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub proof_id: ConfidentialProofId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ProofsFinalizeResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ProofsCancelRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub proof_id: ConfidentialProofId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialCreateOutputProofRequest {
    pub amount: Amount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialCreateOutputProofResponse {
    pub proof: ConfidentialOutputProof,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialTransferRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    pub amount: Amount,
    pub resource_address: ResourceAddress,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub destination_public_key: PublicKey,
    pub max_fee: Option<Amount>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialTransferResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub fee: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ClaimBurnRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub claim_proof: serde_json::Value,
    pub max_fee: Option<Amount>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub key_id: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ClaimBurnResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub fee: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ProofsCancelResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct RevealFundsRequest {
    /// Account with funds to reveal
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    /// Amount to reveal
    pub amount_to_reveal: Amount,
    /// Pay fee from revealed funds. If false, previously revealed funds in the account are used.
    pub pay_fee_from_reveal: bool,
    /// The amount of fees to add to the transaction. Any fees not charged are refunded.
    pub max_fee: Option<Amount>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct RevealFundsResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub fee: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsCreateFreeTestCoinsRequest {
    pub account: Option<ComponentAddressOrName>,
    pub amount: Amount,
    pub max_fee: Option<Amount>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub key_id: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountsCreateFreeTestCoinsResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub amount: Amount,
    pub fee: Amount,
    pub result: FinalizeResult,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct WebRtcStart {
    pub jwt: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct WebRtcStartRequest {
    pub signaling_server_token: String,
    pub permissions: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct WebRtcStartResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthLoginRequest {
    pub permissions: Vec<String>,
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number} | null"))]
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthLoginResponse {
    pub auth_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthLoginAcceptRequest {
    pub auth_token: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthLoginAcceptResponse {
    pub permissions_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthLoginDenyRequest {
    pub auth_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthLoginDenyResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthRevokeTokenRequest {
    pub permission_token_id: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthRevokeTokenResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct MintAccountNftRequest {
    pub account: ComponentAddressOrName,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub metadata: serde_json::Value,
    pub mint_fee: Option<Amount>,
    pub create_account_nft_fee: Option<Amount>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct MintAccountNftResponse {
    pub nft_id: NonFungibleId,
    pub resource_address: ResourceAddress,
    pub result: FinalizeResult,
    pub fee: Amount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct GetAccountNftRequest {
    pub nft_id: NonFungibleId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AccountNftInfo {
    #[cfg_attr(feature = "ts", ts(type = "any"))]
    pub metadata: serde_json::Value,
    pub is_burned: bool,
}

// #[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub type GetAccountNftResponse = AccountNftInfo;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ListAccountNftRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub limit: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub offset: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ListAccountNftResponse {
    pub nfts: Vec<AccountNftInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthGetAllJwtRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct AuthGetAllJwtResponse {
    pub jwt: Vec<Claims>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct GetValidatorFeesRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub validator_public_key: PublicKey,
    // TODO: We'll probably pass in a range of epochs and get non-zero amounts for each epoch in range
    pub epoch: Epoch,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct GetValidatorFeesResponse {
    pub fee_summary: HashMap<Epoch, Amount>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ClaimValidatorFeesRequest {
    #[serde(default, deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    pub max_fee: Option<Amount>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub validator_public_key: PublicKey,
    pub epoch: Epoch,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ClaimValidatorFeesResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_id: TransactionId,
    pub fee: Amount,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct SettingsSetRequest {
    pub indexer_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct SettingsSetResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct SettingsGetResponse {
    pub indexer_url: String,
}
