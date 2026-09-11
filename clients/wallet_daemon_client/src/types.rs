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

use std::{collections::HashMap, fmt::Display, str::FromStr};

use hex::FromHexError;
use serde::{Deserialize, Serialize};
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult},
    confidential::MinotariBurnClaimProof,
    substate::{Substate, SubstateId},
};
use tari_ootle_address::OotleAddress;
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    SubstateAddress,
    SubstateRequirement,
    shard::Shard,
    substate_type::SubstateType,
};
use tari_ootle_template_metadata::{MetadataHash, TemplateMetadata};
use tari_ootle_transaction::{
    Instruction,
    PrunedTransaction,
    TransactionId,
    TransactionSignature,
    UnsignedTransaction,
};
use tari_ootle_wallet_sdk::{
    apis::{
        confidential_transfer::UtxoInputSelection,
        stealth_transfer::{BadgeUsage, TransferFeeParams, TransferOutput},
    },
    crypto::{memo::Memo, pay_to::PayTo},
    models::{
        Account,
        AddressBookEntry,
        AuthoredTemplateModel,
        BalanceChange,
        BalanceChangeSourceType,
        DerivedKeyIndex,
        EffectiveStatus,
        KeyBranch,
        KeyId,
        KeyType,
        NonFungibleToken,
        OutputStatus,
        StealthUtxoSpendKeyId,
        TransactionRequestId,
        TransactionStatus,
        WalletLockId,
        WalletTransaction,
    },
};
use tari_template_abi::{FunctionDef, TemplateDef, version::WasmAbiVersion};
use tari_template_lib_types::{
    Amount,
    ComponentAddress,
    ConfidentialOutputAddress,
    EncryptedData,
    NonFungibleId,
    ResourceAddress,
    ResourceType,
    TemplateAddress,
    UtxoAddress,
    UtxoId,
    ValidatorFeePoolAddress,
    VaultId,
    confidential::{ConfidentialOutputStatement, ConfidentialWithdrawProof},
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, Scalar32Bytes},
    stealth::{SpendAuthorization, StealthTransferStatement},
};
use time::PrimitiveDateTime;
use url::Url;
use webauthn_rs_proto::{
    PublicKeyCredential,
    PublicKeyCredentialCreationOptions,
    RegisterPublicKeyCredential,
    RequestChallengeResponse,
};
use zeroize::Zeroizing;

use crate::{
    ComponentAddressOrName,
    permissions::Permission,
    serialize::{opt_string_or_struct, string_or_struct},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct CallInstructionRequest {
    pub instructions: Vec<Instruction>,
    #[serde(deserialize_with = "string_or_struct")]
    pub fee_account: ComponentAddressOrName,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    /// Substates the instructions require as transaction inputs. Needed for any input that cannot be inferred
    /// from the instructions, e.g. a `ConfidentialOutput` named only by a commitment inside an opaque proof.
    #[serde(default)]
    pub inputs: Vec<SubstateRequirement>,
    /// If true, inputs inferred from the instructions are added to `inputs`.
    #[serde(default)]
    pub override_inputs: Option<bool>,
    #[serde(default)]
    pub new_outputs: Option<u8>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub proof_ids: Vec<WalletLockId>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub min_epoch: Option<u64>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub max_epoch: Option<u64>,
}

/// Create a transaction request for a separately-permissioned principal to
/// approve (issue #2343).
///
/// The transaction is stored verbatim and frozen: what the approver views is
/// what submit seals. The caller supplies a complete transaction (inputs
/// resolved, any out-of-band signatures collected); walletd does not alter it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestCreateRequest {
    /// The transaction, stored verbatim. Inputs must already be resolved: the
    /// request path does not detect them, so that a caller who has already
    /// collected signatures over these exact bytes is not invalidated. Use
    /// `transactions.detect_inputs` first if the caller cannot resolve them.
    pub transaction: UnsignedTransaction,
    /// Who pays and seals last.
    pub seal_signer: KeyId,
    /// Wallet keys walletd signs with at submit.
    #[serde(default)]
    pub other_signers: Vec<KeyId>,
    /// Signatures collected out of band (e.g. from an external co-signer) over
    /// `transaction` and `seal_signer`'s public key. Attached verbatim at
    /// submit. Their presence means the transaction is final, so nothing about
    /// it is altered after creation.
    #[serde(default)]
    pub signatures: Vec<TransactionSignature>,
    /// Locks holding this request's inputs. Their timeouts are extended to
    /// outlive the approval window. Stealth spend keys are derived from these
    /// at submit -- a caller cannot name them.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub lock_ids: Vec<WalletLockId>,
    /// How long a human has to approve, in seconds. Defaults to the daemon's
    /// configured window.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub ttl_secs: Option<u64>,
}

/// Resolve a transaction's inputs without submitting it.
///
/// Detection returns the **dependency closure** of everything the instructions
/// reference (e.g. an account component pulls in every vault it holds), which
/// is a superset of what execution will touch: the wallet cannot know intent
/// without executing, so it over-approximates. Narrowing the set is the
/// caller's job — every declared input adds transaction weight (fees) and must
/// be locked by consensus, so the caller pays for anything left in.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionDetectInputsRequest {
    pub transaction: UnsignedTransaction,
    /// If true (default), detected inputs omit versions so consensus resolves
    /// them; if false, walletd pins the versions it currently knows.
    #[serde(default = "return_true")]
    pub use_unversioned: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionDetectInputsResponse {
    /// The input transaction with the detected inputs merged in.
    pub transaction: UnsignedTransaction,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestCreateResponse {
    pub request_id: TransactionRequestId,
    /// Unix timestamp (seconds).
    pub expires_at: i64,
}

/// Net value movement for a stealth or confidential transfer, computed by the
/// wallet from the request's locks.
///
/// A stealth transfer's instructions are commitments and range proofs, so a
/// human cannot read the amount out of them. The wallet owns the masks and is
/// the only party that can state what leaves the account.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestValueSummary {
    pub resource_address: ResourceAddress,
    /// Total value of the inputs this request spends.
    pub inputs_total: u64,
    /// Total value returning to this wallet as change.
    pub change_total: u64,
    /// `inputs_total - change_total`: what actually leaves, before fees.
    pub amount_leaving: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestInfo {
    pub request_id: TransactionRequestId,
    /// The frozen transaction, decoded for display. These are the exact bytes
    /// an approval commits to.
    pub transaction: UnsignedTransaction,
    pub seal_signer: KeyId,
    pub other_signers: Vec<KeyId>,
    /// Admin-assigned name of the API key that created this request, or `None`
    /// for a wallet session. Display only -- nothing authorises on it.
    pub requested_by: Option<String>,
    pub status: EffectiveStatus,
    pub transaction_id: Option<TransactionId>,
    /// `None` for an ordinary transaction, whose instructions are readable.
    pub value_summary: Option<TransactionRequestValueSummary>,
    /// Unix timestamp (seconds).
    pub expires_at: i64,
    /// Unix timestamp (seconds). `None` until approved.
    pub approved_at: Option<i64>,
    /// Unix timestamp (seconds).
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestGetRequest {
    pub request_id: TransactionRequestId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestGetResponse {
    pub request: TransactionRequestInfo,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestListRequest {
    /// Only return requests whose effective status matches.
    #[serde(default)]
    pub status: Option<EffectiveStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestListResponse {
    pub requests: Vec<TransactionRequestInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestDecisionRequest {
    pub request_id: TransactionRequestId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestDecisionResponse {
    pub request_id: TransactionRequestId,
    pub status: EffectiveStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestSubmitRequest {
    pub request_id: TransactionRequestId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionRequestSubmitResponse {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionSubmitRequest {
    pub transaction: UnsignedTransaction,
    pub seal_signer: KeyId,
    pub other_signers: Vec<KeyId>,
    /// Signatures collected out of band, attached before walletd adds its own
    /// and seals.
    #[serde(default)]
    pub signatures: Vec<TransactionSignature>,
    /// Attempt to infer inputs and their dependencies from instructions. If false, the provided transaction must
    /// contain the required inputs.
    pub detect_inputs: bool,
    /// If true(default), detected inputs will omit versions allowing consensus to resolve input substates.
    /// If false, the wallet will try to determine versions for the inputs. These may be outdated if the substate has
    /// changed since detection.
    #[serde(default = "return_true")]
    pub detect_inputs_use_unversioned: bool,
    pub lock_ids: Vec<WalletLockId>,
}

const fn return_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionSubmitResponse {
    pub transaction_id: TransactionId,
}

pub type TransactionSubmitDryRunRequest = TransactionSubmitRequest;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionSubmitDryRunResponse {
    pub transaction_id: TransactionId,
    /// The minimum fee required to submit this transaction. Includes a +1 buffer over
    /// `total_fees_charged` to account for storage fee rounding differences between the dry run
    /// and actual submission (the vault balance changes with a different max_fee, which can shift
    /// `floor(total_bytes / 4)` by 1 at a rounding boundary). Non-refundable overcharge is
    /// subtracted since it won't recur with a tighter max_fee.
    pub required_fees: u64,
    pub result: ExecuteResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionSubmitManifestRequest {
    pub manifest: String,
    pub variables: HashMap<String, String>,
    /// The key used for the seal (owner) signature. If not provided, defaults to the default account's owner key.
    pub seal_signer_key_id: Option<KeyId>,
    /// Additional signing keys for accounts involved in the transaction (e.g. for multi-account manifests).
    #[serde(default)]
    pub signing_key_ids: Vec<KeyId>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    pub dry_run: bool,
    /// Blob payloads referenced from the manifest via `blob!(name)`. Keys are the names used
    /// in the manifest text; values are base64-encoded byte payloads in JSON.
    #[serde(default)]
    pub blobs: HashMap<String, tari_ootle_transaction::Blob>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionSubmitManifestResponse {
    pub transaction_id: TransactionId,
    /// The minimum fee required to submit this transaction. Only present for dry runs.
    /// Includes a +1 buffer over `total_fees_charged` to account for storage fee rounding
    /// differences between the dry run and actual submission.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub required_fees: Option<u64>,
    pub result: Option<ExecuteResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct PublishTemplateRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::base64")]
    pub binary: Vec<u8>,
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub fee_account: Option<ComponentAddressOrName>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    /// Attempt to infer inputs and their dependencies from instructions. If false, the provided transaction must
    /// contain the required inputs.
    pub detect_inputs: bool,
    pub dry_run: bool,
    /// Optional template metadata. Can be provided as raw JSON/CBOR (base64-encoded) for server-side
    /// hashing, or as a pre-computed hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PublishTemplateMetadata>,
}

/// Template metadata input for publishing. Either provide the metadata for server-side hashing,
/// or a pre-computed hash.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", content = "data")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum PublishTemplateMetadata {
    /// Inline template metadata object. The server CBOR-encodes it and computes the hash.
    Literal(Box<TemplateMetadata>),
    /// Pre-encoded CBOR metadata (base64-encoded). The server decodes and computes the hash.
    RawCbor(
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        #[serde(with = "ootle_serde::base64")]
        Vec<u8>,
    ),
    /// Pre-computed metadata hash (hex-encoded multihash).
    Hash(#[cfg_attr(feature = "ts", ts(type = "string"))] MetadataHash),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct PublishTemplateResponse {
    pub transaction_id: TransactionId,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub dry_run_fee: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionGetRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionGetResponse {
    /// Pruned transaction — blob commitments are present but raw blob bytes are omitted to
    /// keep API responses small. Use a separate endpoint to fetch blob payloads if required.
    pub transaction: PrunedTransaction,
    pub result: Option<FinalizeResult>,
    pub status: TransactionStatus,
    /// The estimated fee required for the transaction. For dry runs, this is the minimum fee
    /// that should be used as `max_fee` for the actual submission.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub final_fee: Option<u64>,
    pub invalid_reason: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub last_update_time: PrimitiveDateTime,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionGetAllRequest {
    pub status: Option<TransactionStatus>,
    /// Filter to transactions involving this account. Transactions are linked to the account(s) they
    /// involve at submission time, so this works even for stealth transactions.
    pub account: Option<ComponentAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionGetAllResponse {
    pub transactions: Vec<WalletTransaction>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionGetResultRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionGetResultResponse {
    pub transaction_id: TransactionId,
    pub status: TransactionStatus,
    pub result: Option<FinalizeResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionWaitResultRequest {
    pub transaction_id: TransactionId,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionWaitResultResponse {
    pub transaction_id: TransactionId,
    pub result: Option<FinalizeResult>,
    pub status: TransactionStatus,
    pub final_fee: u64,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransactionClaimBurnResponse {
    pub transaction_id: TransactionId,
    pub inputs: Vec<SubstateAddress>,
    pub outputs: Vec<SubstateAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct BurnProofsListRequest {
    /// Optional filter by account public key. Only proofs whose file name starts with this key
    /// will be returned. Proofs with file names that do not match the expected
    /// `{public_key}_{commitment}.json` format are always included.
    pub filter_by_public_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct BurnProofsListResponse {
    pub proofs: Vec<BurnProofFileInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct BurnProofFileInfo {
    pub file_name: String,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub value: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct BurnProofsGetRequest {
    pub file_name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct BurnProofsGetResponse {
    pub proof: ClaimBurnProofContents,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct KeysListRequest {
    pub branch: KeyBranch,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct KeysListResponse {
    /// (KeyId, public key, is_active)
    pub keys: Vec<(KeyId, RistrettoPublicKeyBytes, bool)>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct KeysSetActiveRequest {
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub index: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct KeysSetActiveResponse {
    pub public_key: RistrettoPublicKeyBytes,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct KeysCreateRequest {
    pub branch: KeyBranch,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub specific_index: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct KeysCreateResponse {
    pub id: u64,
    pub public_key: RistrettoPublicKeyBytes,
}

/// Imports an externally-generated secret key into the wallet's keystore. The `secret_key` is transmitted as hex.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeysImportRequest {
    pub secret_key: RistrettoSecretKey,
    pub key_type: KeyType,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeysImportResponse {
    pub key_id: KeyId,
    pub public_key: RistrettoPublicKeyBytes,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsCreateRequest {
    pub account_name: Option<String>,
    pub is_default: Option<bool>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub key_index: Option<DerivedKeyIndex>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsCreateResponse {
    pub account: Account,
    pub address: OotleAddress,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsCreateOrGetRequest {
    pub account: Option<ComponentAddressOrName>,
    pub is_default: Option<bool>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub key_index: Option<DerivedKeyIndex>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsCreateOrGetResponse {
    pub account: Account,
    pub address: OotleAddress,
    pub created: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsListRequest {
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountInfo {
    pub account: Account,
    pub address: OotleAddress,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsListResponse {
    pub accounts: Vec<AccountInfo>,
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsGetBalancesRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsGetBalancesResponse {
    pub address: ComponentAddress,
    pub balances: Vec<BalanceEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsGetBalanceChangesRequest {
    #[serde(deserialize_with = "string_or_struct")]
    pub account: ComponentAddressOrName,
    pub offset: u32,
    pub limit: u32,
    pub resource_address: Option<ResourceAddress>,
    pub transaction_id: Option<TransactionId>,
    pub source_type: Option<BalanceChangeSourceType>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsGetBalanceChangesResponse {
    pub changes: Vec<BalanceChange>,
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct BalanceEntry {
    pub vault_address: Option<VaultId>,
    pub resource_address: ResourceAddress,
    pub balance: Amount,
    pub resource_type: ResourceType,
    pub confidential_balance: Amount,
    pub token_symbol: Option<String>,
    pub divisibility: u8,
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
            ResourceType::Stealth => {
                format!("{} {} (stealth)", self.balance + self.confidential_balance, symbol)
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountGetRequest {
    #[serde(deserialize_with = "string_or_struct")]
    pub name_or_address: ComponentAddressOrName,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountGetDefaultRequest {
    // Intentionally empty. Fields may be added in the future.
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountGetByKeyIndexRequest {
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub key_index: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountGetResponse {
    pub account: Account,
    pub address: OotleAddress,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountSetDefaultRequest {
    #[serde(deserialize_with = "string_or_struct")]
    pub account: ComponentAddressOrName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountSetDefaultResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsRenameRequest {
    #[serde(deserialize_with = "string_or_struct")]
    pub account: ComponentAddressOrName,
    pub new_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsRenameResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsTransferRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    pub amount: Amount,
    pub resource_address: ResourceAddress,
    pub destination_public_key: RistrettoPublicKeyBytes,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    pub proof_from_badge_resource: Option<ResourceAddress>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsTransferResponse {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ProofsGenerateRequest {
    pub confidential_amount: Amount,
    pub reveal_amount: Amount,
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    pub resource_address: ResourceAddress,
    pub destination_address: OotleAddress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_memo: Option<Memo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ProofsGenerateResponse {
    pub proof_id: WalletLockId,
    pub proof: ConfidentialWithdrawProof,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ProofsFinalizeRequest {
    pub lock_id: WalletLockId,
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ProofsFinalizeResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ProofsCancelRequest {
    pub proof_id: WalletLockId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialCreateOutputProofRequest {
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub amount: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialCreateOutputProofResponse {
    pub proof: ConfidentialOutputStatement,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialTransferRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    pub amount: Amount,
    pub input_selection: UtxoInputSelection,
    pub resource_address: ResourceAddress,
    pub destination_address: OotleAddress,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    pub output_to_revealed: bool,
    pub proof_from_badge_resource: Option<ResourceAddress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_memo: Option<Memo>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialTransferResponse {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialViewVaultBalanceRequest {
    pub vault_id: VaultId,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub minimum_expected_value: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub maximum_expected_value: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub view_key_id: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialViewVaultBalanceResponse {
    #[cfg_attr(feature = "ts", ts(type = "Record<string, number | null>"))]
    pub balances: HashMap<PedersenCommitmentBytes, Option<u64>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ClaimBurnRequest {
    pub account: ComponentAddressOrName,
    pub claim_proof: ClaimBurnProof,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum ClaimBurnProof {
    Contents(Box<ClaimBurnProofContents>),
    FromFile { file_name: String },
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ClaimBurnProofContents {
    pub claim_proof: MinotariBurnClaimProof,
    pub encrypted_data: EncryptedData,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ClaimBurnResponse {
    pub transaction_id: TransactionId,
    /// The minimum fee required to submit this transaction. Only present for dry runs.
    /// Includes a +1 buffer over `total_fees_charged` to account for storage fee rounding
    /// differences between the dry run and actual submission.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub required_fees: Option<u64>,
    pub dry_run_result: Option<ExecuteResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ProofsCancelResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsCreateFreeTestCoinsRequest {
    pub account: ComponentAddressOrName,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsCreateFreeTestCoinsResponse {
    pub account: Account,
    pub transaction_id: TransactionId,
    pub amount: Amount,
    pub fee: u64,
    pub result: FinalizeResult,
    pub address: OotleAddress,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebRtcStart {
    pub jwt: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebRtcStartRequest {
    pub signaling_server_token: String,
    #[cfg_attr(feature = "ts", ts(type = "object"))]
    pub permissions: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebRtcStartResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthRefreshRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthRefreshResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub token: EncodedJwtString,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthLoginRequest {
    pub permissions: Vec<Permission>,
    pub credentials: AuthCredentials,
}

/// Credentials for the `auth.request` JSON-RPC entry point. Used by humans
/// authenticating via WebAuthN (or in `none` auth mode); agent automation
/// authenticates by sending the raw API key as the `Authorization: Bearer
/// …` header on every JSON-RPC call instead, with no `auth.request`
/// round-trip.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum AuthCredentials {
    /// Credentials for 'none' auth mode
    None,
    /// Credentials for WebAuthN auth mode. Contains the request from the client to finish the auth.
    WebAuthN(Box<WebauthnFinishAuthRequest>),
}

impl AuthCredentials {
    pub fn as_none(&self) -> Option<()> {
        match self {
            Self::None => Some(()),
            _ => None,
        }
    }

    pub fn as_webauthn(&self) -> Option<&WebauthnFinishAuthRequest> {
        match self {
            Self::WebAuthN(req) => Some(req),
            _ => None,
        }
    }
}

/// Represents a JWT token. The token is zeroized from memory on drop.
pub type EncodedJwtString = Zeroizing<String>;

/// Raw API key material. Zeroized on drop so a freed allocation does not
/// leave the plaintext behind in memory. Used for both the
/// `auth.request` ApiKey credential and the one-shot create response.
pub type EncodedApiKey = Zeroizing<String>;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthLoginResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub token: EncodedJwtString,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthRevokeTokenRequest {
    pub refresh_token_id: RefreshTokenHash,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct RefreshTokenHash(
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::hex")]
    [u8; 32],
);

impl RefreshTokenHash {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl FromStr for RefreshTokenHash {
    type Err = FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s, &mut bytes)?;
        Ok(Self(bytes))
    }
}

impl Display for RefreshTokenHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthRevokeTokenResponse {}

// -----------------------------------------------------------------------------
// API key management (issue #1957) — admin-only CRUD over long-lived
// credentials that AI agents and other automated clients use to authenticate
// without webauthn. Every endpoint that handles these structures requires
// the caller to already hold the `Admin` permission; non-admin sessions are
// rejected at the JSON-RPC layer.
// -----------------------------------------------------------------------------

/// Admin → daemon: mint a new long-lived API key with the supplied scopes.
///
/// `permissions` is the same textual form `Permissions::from_str`
/// accepts (e.g. `["accounts:read", "transactions:read"]`). `confirm_admin`
/// must be set to `true` if and only if the list contains the `admin`
/// permission — this is a deliberate speed-bump so the UI can render an
/// explicit warning before issuing a fully-privileged credential.
///
/// `expires_at` is an optional unix-seconds deadline. When set, the daemon's
/// active-row filter excludes the key once that timestamp has passed; the
/// agent receives the same opaque "invalid or revoked" error as for an
/// unknown or revoked key. `None` means the key never expires.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthCreateApiKeyRequest {
    pub name: String,
    pub permissions: Vec<String>,
    #[serde(default)]
    pub confirm_admin: bool,
    /// Unix timestamp (seconds) at which the key becomes unusable. `None`
    /// for a never-expiring key. Rejected at the handler if it lies in the
    /// past — refusing to mint an instantly-expired credential.
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub expires_at: Option<i64>,
}

/// Daemon → admin: response after successful key creation.
///
/// `api_key` is the RAW key material and is included in this response
/// EXACTLY ONCE; it cannot be retrieved again. The admin/UI must store it
/// immediately (clipboard / secrets manager). The daemon only persists a
/// SHA-256 hash, so a database leak does not expose this string.
#[derive(Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthCreateApiKeyResponse {
    pub id: i32,
    pub name: String,
    pub permissions: Vec<Permission>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub api_key: EncodedApiKey,
    /// Unix timestamp (seconds) of creation.
    pub created_at: i64,
}

// Manual Debug: do not leak the one-shot raw key into logs / error context
// if a caller ever `?`-bubbles a response or wraps it in `anyhow::Context`.
impl std::fmt::Debug for AuthCreateApiKeyResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthCreateApiKeyResponse")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("permissions", &self.permissions)
            .field("api_key", &"<redacted>")
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Admin → daemon: list API keys. By default returns only non-revoked
/// rows (the typical admin-UI view); set `include_revoked` to retrieve
/// the historical audit list. Expired keys are always included — their
/// `last_used_at` is useful audit context. The raw key material is
/// NEVER included in the response.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthListApiKeysRequest {
    #[serde(default)]
    pub include_revoked: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthListApiKeysResponse {
    pub keys: Vec<IssuedApiKey>,
}

/// Metadata returned by `auth.list_api_keys`. Mirrors the storage row
/// minus the hash (which the admin doesn't need to see) and the raw key
/// (which doesn't exist anywhere persistent).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct IssuedApiKey {
    pub id: i32,
    pub name: String,
    pub permissions: Vec<Permission>,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    /// Unix timestamp (seconds). `None` if the key has never been used to
    /// authenticate since creation.
    pub last_used_at: Option<i64>,
    /// Unix timestamp (seconds). `None` for an active key; non-null means
    /// the key has been revoked and is no longer usable.
    pub revoked_at: Option<i64>,
    /// Unix timestamp (seconds). `None` means the key does not expire.
    /// Once the timestamp has passed, the auth shim's active-row filter
    /// stops surfacing the key — the agent gets the same opaque error as
    /// for a revoked key.
    pub expires_at: Option<i64>,
}

/// Admin → daemon: revoke an API key by its id. The row is soft-deleted
/// (`revoked_at` stamped) so admin tooling can still see the historical
/// `last_used_at` of revoked credentials, but the storage layer filters
/// revoked rows out of authentication lookups so revocation takes effect
/// immediately.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthRevokeApiKeyRequest {
    pub id: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthRevokeApiKeyResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct MintFaucetNftRequest {
    pub account: ComponentAddressOrName,
    #[cfg_attr(feature = "ts", ts(type = "object"))]
    pub mutable_data: serde_json::Value,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub number_to_mint: u64,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub max_fee: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct MintFaucetNftResponse {
    pub transaction_id: TransactionId,
    pub finalize: FinalizeResult,
    pub fee: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct GetNftRequest {
    pub resource_address: ResourceAddress,
    pub nft_id: NonFungibleId,
}

pub type GetNftResponse = NonFungibleToken;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ListNftsRequest {
    #[serde(deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ListNftsResponse {
    pub nfts: Vec<NonFungibleToken>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthListSessionsRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthListSessionsResponse {
    pub sessions: Vec<AuthSessionInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthSessionInfo {
    pub id: RefreshTokenHash,
    pub permissions: Vec<Permission>,
    pub exp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct GetValidatorFeesRequest {
    pub account_or_key: AccountOrKeyId,
    pub shard_group: Option<ShardGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum AccountOrKeyId {
    /// Query by account. None signifies the default account.
    Account(Option<ComponentAddressOrName>),
    /// Query by key id.
    KeyId(KeyId),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct GetValidatorFeesResponse {
    pub fees: HashMap<Shard, FeePoolDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct FeePoolDetails {
    pub address: ValidatorFeePoolAddress,
    pub amount: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ClaimValidatorFeesRequest {
    #[serde(default, deserialize_with = "opt_string_or_struct")]
    pub account: Option<ComponentAddressOrName>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub claim_key_index: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    pub shards: Vec<Shard>,
    pub dry_run: bool,
    /// If true, claim into the account's revealed vault. If false (default), claim into a per-shard stealth UTXO
    /// addressed to the account's own owner key.
    #[serde(default)]
    pub output_to_revealed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ClaimValidatorFeesResponse {
    pub transaction_id: TransactionId,
    pub fee: u64,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SettingsSetRequest {
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[serde(default)]
    pub indexer_url: Option<Url>,
    #[serde(default)]
    pub advanced_ui_features: Option<AdvancedUiFeatures>,
    #[serde(default)]
    pub claimed_accounts: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SettingsSetResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SettingsGetResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub indexer_url: Url,
    pub network: NetworkInfo,
    pub advanced_ui_features: AdvancedUiFeatures,
    pub claimed_accounts: Vec<String>,
    /// The network's current epoch, as the wallet daemon sees it. Callers that build transactions
    /// themselves need it to choose a `max_epoch` the network will accept. `None` when the indexer
    /// is unreachable — settings must stay readable while the network is down, not least so the
    /// caller can see and correct the indexer URL.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub current_epoch: Option<Epoch>,
    /// How many epochs past `current_epoch` the wallet daemon stamps `max_epoch` when a caller does
    /// not choose one.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub default_transaction_validity_epochs: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AdvancedUiFeatures {
    pub enable_manifest: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct NetworkInfo {
    pub name: String,
    pub byte: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SubstatesListRequest {
    #[serde(default, deserialize_with = "ootle_serde::string::option::deserialize")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub filter_by_template: Option<TemplateAddress>,
    pub filter_by_type: Option<SubstateType>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub limit: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SubstatesListResponse {
    pub substates: Vec<WalletSubstateInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SubstatesGetRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub substate_id: SubstateId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SubstatesGetResponse {
    // NOTE either of these can be None, but never both (instead, NotFound error)
    pub local_record: Option<WalletSubstateInfo>,
    pub substate_from_remote: Option<Substate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WalletSubstateInfo {
    pub substate_id: SubstateId,
    pub parent_id: Option<SubstateId>,
    pub module_name: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
    pub template_address: Option<TemplateAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TemplatesGetRequest {
    pub template_address: TemplateAddress,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TemplatesGetResponse {
    pub template_definition: TemplateDef,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TemplatesListAuthoredRequest {
    pub author_public_key: Option<RistrettoPublicKeyBytes>,
    pub page: u32,
    pub page_size: u32,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthoredTemplate {
    pub author_public_key: RistrettoPublicKeyBytes,
    pub address: TemplateAddress,
    pub name: String,
    pub abi_version: WasmAbiVersion,
    pub functions: Vec<FunctionDef>,
}

impl From<AuthoredTemplateModel> for AuthoredTemplate {
    fn from(model: AuthoredTemplateModel) -> Self {
        AuthoredTemplate {
            author_public_key: model.author_public_key,
            address: model.address,
            name: model.name,
            abi_version: model.abi_version,
            functions: model.functions,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TemplatesListAuthoredResponse {
    pub templates: Vec<AuthoredTemplate>,
    pub total_templates: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthGetMethodRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    None,
    Webauthn,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AuthGetMethodResponse {
    pub method: AuthMethod,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnAlreadyRegisteredRequest {
    pub username: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnAlreadyRegisteredResponse {
    pub registered: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnStartRegisterRequest {
    pub username: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnStartRegisterResponse {
    /// Unique ID of the current registration Session.
    pub session_id: String,
    /// [`PublicKeyCredentialCreationOptions`] serialized as JSON
    #[cfg_attr(feature = "ts", ts(type = "object"))]
    pub public_key: PublicKeyCredentialCreationOptions,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnFinishRegisterRequest {
    /// Session ID received from [`WebauthnStartRegisterResponse`].
    pub session_id: String,
    /// [`RegisterPublicKeyCredential`]
    #[cfg_attr(feature = "ts", ts(type = "object"))]
    pub credential: RegisterPublicKeyCredential,
    /// Permissions requested by the client to be associated with the registered credential.
    pub requested_permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnFinishRegisterResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub token: EncodedJwtString,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnStartAuthRequest {
    pub username: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnStartAuthResponse {
    /// Session ID.
    pub session_id: String,
    /// [`RequestChallengeResponse`] serialized as JSON string.
    #[cfg_attr(feature = "ts", ts(type = "object"))]
    pub challenge: RequestChallengeResponse,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WebauthnFinishAuthRequest {
    /// Session ID received from [`WebauthnStartAuthResponse`].
    pub session_id: String,
    /// [`PublicKeyCredential`]
    #[cfg_attr(feature = "ts", ts(type = "object"))]
    pub credential: PublicKeyCredential,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WalletGetInfoRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct WalletGetInfoResponse {
    pub version: String,
    pub network: String,
    pub network_byte: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransferNftRequest {
    pub resource_address: ResourceAddress,
    pub nfts: Vec<NonFungibleId>,
    #[serde(deserialize_with = "string_or_struct")]
    pub fee_payer_account: ComponentAddressOrName,
    #[serde(deserialize_with = "string_or_struct")]
    pub source_account: ComponentAddressOrName,
    pub target_account_address: OotleAddress,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransferNftResponse {
    pub transaction_id: TransactionId,
    pub fee: u64,
    pub fee_refunded: u64,
    pub result: FinalizeResult,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsCreateStealthTransferStatementRequest {
    pub requests: Vec<TransferStatementRequest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransferStatementRequest {
    pub sender_account: ComponentAddressOrName,
    pub resource_address: ResourceAddress,
    pub input_selection: InputSelection,
    pub outputs: Vec<TransferOutput>,
}

impl TransferStatementRequest {
    pub fn total_output_amount(&self) -> Amount {
        self.outputs
            .iter()
            .map(|o| Amount::from(o.blinded_amount) + o.revealed_amount)
            .sum()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum InputSelection {
    FromBucket {
        revealed_amount: Amount,
    },
    Selection(UtxoInputSelection),
    /// Spend exactly the named wallet-owned stealth UTXOs, in the given order. The wallet resolves, validates and
    /// locks precisely these outputs via `StealthOutputsApi::lock_specific_outputs`, rather than choosing inputs to
    /// cover an amount.
    Specific {
        utxo_addresses: Vec<UtxoAddress>,
    },
}

impl InputSelection {
    pub fn as_selection(&self) -> Option<UtxoInputSelection> {
        match self {
            InputSelection::Selection(s) => Some(*s),
            InputSelection::FromBucket { .. } | InputSelection::Specific { .. } => None,
        }
    }

    pub fn as_from_bucket(&self) -> Option<Amount> {
        match self {
            InputSelection::FromBucket { revealed_amount } => Some(*revealed_amount),
            InputSelection::Selection(_) | InputSelection::Specific { .. } => None,
        }
    }

    /// The exact UTXOs to spend when this is an [`InputSelection::Specific`] selection.
    pub fn as_specific(&self) -> Option<&[UtxoAddress]> {
        match self {
            InputSelection::Specific { utxo_addresses } => Some(utxo_addresses),
            InputSelection::FromBucket { .. } | InputSelection::Selection(_) => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsCreateStealthTransferStatementResponse {
    pub statements: Vec<StealthTransferStatement>,
    pub lock_id: WalletLockId,
    pub signing_keys: Vec<KeyId>,
    /// Any signatures using a stealth spend key required to spend inputs provided in the statements.
    pub utxo_signers: Vec<StealthUtxoSpendKeyId>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthTransferRequest {
    pub owner_account: ComponentAddressOrName,
    pub fee_params: TransferFeeParams,
    pub input_selection: UtxoInputSelection,
    pub resource_address: ResourceAddress,
    #[serde(default, skip_serializing_if = "BadgeUsage::is_none")]
    pub badge_usage: BadgeUsage,
    pub transfers: Vec<StealthTransfer>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub max_fee: u64,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthTransfer {
    pub destination_address: OotleAddress,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    pub blinded_output_amount: u64,
    pub revealed_output_amount: Amount,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_memo: Option<Memo>,
    pub pay_to: PayTo,
    /// If set, the sender's Ootle address is attached as the output memo so the recipient can identify and save
    /// the sender as a contact. Replaces `output_memo` (the sender address takes precedence). Composes with
    /// `pay_ref`, which is embedded inside the SenderAddress memo when set.
    #[serde(default)]
    pub attach_sender_address: bool,
    /// Optional pay reference (UTF-8, max 64 bytes). When `attach_sender_address` is true it is embedded inside
    /// the SenderAddress memo; otherwise it builds a `PayRefAndBytes` memo combined with any `output_memo`. If
    /// unset, the destination address's bech32-embedded pay reference (if any) is used as a fallback for
    /// backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pay_ref: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthTransferResponse {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsAssociateStealthResourceRequest {
    pub account: ComponentAddressOrName,
    pub resource_address: ResourceAddress,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AccountsAssociateStealthResourceResponse {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthUtxosListRequest {
    pub resource_address: ResourceAddress,
    pub account_address: Option<ComponentAddress>,
    pub filter_by_status: Option<OutputStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthUtxosListResponse {
    pub utxos: Vec<UtxoInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct UtxoInfo {
    pub address: UtxoAddress,
    pub value: Amount,
    pub status: OutputStatus,
    pub memo: Option<Memo>,
    /// The sender's Ootle address, resolved from the memo when it is a `SenderAddress` variant, using the
    /// wallet's configured network. `None` for all other memo types.
    pub sender_address: Option<OotleAddress>,
    /// How this UTXO is authorised at spend time (key path, condition tree, or both).
    pub auth: SpendAuthorization,
    pub is_burnt: bool,
    pub is_frozen: bool,
    pub is_on_chain: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialOutputsListRequest {
    pub resource_address: ResourceAddress,
    pub account_address: Option<ComponentAddress>,
    pub filter_by_status: Option<OutputStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialOutputsListResponse {
    pub outputs: Vec<ConfidentialOutputInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct ConfidentialOutputInfo {
    /// Address of the output's own substate. It carries the commitment, which is the output's identity.
    pub address: ConfidentialOutputAddress,
    /// The vault holding this output. Membership of a vault's commitment list is what authorises the spend.
    pub vault_id: VaultId,
    /// The decrypted value. Zero for an output the wallet could not decrypt (`OutputStatus::Invalid`).
    pub value: Amount,
    pub status: OutputStatus,
    pub memo: Option<Memo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthUtxosDecryptValueRequest {
    pub resource_address: ResourceAddress,
    pub ids: Vec<UtxoId>,
    pub view_key_id: KeyId,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub minimum_expected_value: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub maximum_expected_value: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthUtxosDecryptValueResponse {
    pub values: HashMap<UtxoId, Option<u64>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthUtxosGetValueLookupInfoRequest {}

/// Describes the value lookup table configured for confidential/stealth balance decryption. When no file is
/// configured, `configured` is `false` and the remaining fields are `None`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct StealthUtxosGetValueLookupInfoResponse {
    pub configured: bool,
    pub path: Option<String>,
    pub format: Option<String>,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub prefix_len: Option<u8>,
    pub value_len: Option<u8>,
    pub num_records: Option<u64>,
}

// -------------------------------- Templates -------------------------------- //

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SignTemplateMetadataRequest {
    pub key_id: KeyId,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub template_address: TemplateAddress,
    /// The template metadata to sign. Provided as an inline JSON object.
    pub metadata: TemplateMetadata,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SignTemplateMetadataResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_nonce: RistrettoPublicKeyBytes,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub signature: Scalar32Bytes,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: RistrettoPublicKeyBytes,
    /// Hex-encoded canonical CBOR of the metadata (the signed payload).
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::hex")]
    pub metadata_cbor: Vec<u8>,
    /// The metadata hash derived from the CBOR encoding.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub metadata_hash: MetadataHash,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SwapPoolGetExchangeRateRequest {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub pool_address: ComponentAddress,
    /// If provided, the response will include the calculated swap input amount needed
    /// to receive at least this amount of TARI from the pool (with a slippage margin).
    #[serde(default)]
    pub desired_tari_output: Option<Amount>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SwapPoolGetExchangeRateResponse {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub resource_a: ResourceAddress,
    pub balance_a: Amount,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub resource_b: ResourceAddress,
    pub balance_b: Amount,
    /// The calculated input amount of the non-TARI token needed to receive at least
    /// `desired_tari_output` TARI from the pool. Only present when `desired_tari_output` was provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_input_amount: Option<Amount>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SwapPoolsListRequest {
    /// Filter pools to only those containing this exact resource pair (in any order).
    /// Both must be provided for filtering to take effect.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "[string, string] | null"))]
    pub resource_pair: Option<(ResourceAddress, ResourceAddress)>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub limit: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string | null"))]
    #[serde(default, deserialize_with = "ootle_serde::str_number::option::deserialize")]
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SwapPoolsListResponse {
    pub pools: Vec<SwapPoolInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct SwapPoolInfo {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub pool_address: ComponentAddress,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub resource_a: ResourceAddress,
    pub balance_a: Amount,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub resource_b: ResourceAddress,
    pub balance_b: Amount,
}

// -------------------------------- AddressBook -------------------------------- //

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookAddRequest {
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookAddResponse {
    pub entry: AddressBookEntry,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookListRequest {}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookListResponse {
    pub entries: Vec<AddressBookEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookGetRequest {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookGetResponse {
    pub entry: AddressBookEntry,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookUpdateRequest {
    pub name: String,
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookUpdateResponse {
    pub entry: AddressBookEntry,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookDeleteRequest {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct AddressBookDeleteResponse {}

#[cfg(test)]
mod tests {
    use tari_ootle_wallet_sdk::models::{BalanceChangeSource, BalanceChangeSourceType};

    use super::*;

    /// A JS client encodes every 64-bit field as a string, because JavaScript cannot hold an integer
    /// above `Number.MAX_SAFE_INTEGER` in a `number` without losing precision. Both encodings must be
    /// accepted, for any magnitude.
    #[test]
    fn requests_accept_string_encoded_64_bit_fields() {
        let request: KeysSetActiveRequest = serde_json::from_value(serde_json::json!({
            "index": "18446744073709551615",
        }))
        .unwrap();
        assert_eq!(request.index, u64::MAX);

        let request: AccountGetByKeyIndexRequest = serde_json::from_value(serde_json::json!({
            "key_index": "9007199254740993",
        }))
        .unwrap();
        assert_eq!(request.key_index, 9_007_199_254_740_993);

        // Numbers still work, and the two encodings agree.
        let request: KeysSetActiveRequest = serde_json::from_value(serde_json::json!({ "index": 7 })).unwrap();
        assert_eq!(request.index, 7);
    }

    #[test]
    fn optional_64_bit_fields_accept_string_number_null_and_absent() {
        let from_string: SwapPoolsListRequest =
            serde_json::from_value(serde_json::json!({ "limit": "9007199254740993", "offset": "0" })).unwrap();
        assert_eq!(from_string.limit, Some(9_007_199_254_740_993));
        assert_eq!(from_string.offset, Some(0));

        let from_number: SwapPoolsListRequest =
            serde_json::from_value(serde_json::json!({ "limit": 10, "offset": null })).unwrap();
        assert_eq!(from_number.limit, Some(10));
        assert_eq!(from_number.offset, None);

        let absent: SwapPoolsListRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(absent.limit, None);
        assert_eq!(absent.offset, None);
    }

    #[test]
    fn balance_change_request_accepts_string_account_and_optional_filters() {
        let transaction_id = TransactionId::default();
        let request: AccountsGetBalanceChangesRequest = serde_json::from_value(serde_json::json!({
            "account": "savings",
            "offset": 20,
            "limit": 10,
            "resource_address": null,
            "transaction_id": transaction_id.to_string(),
            "source_type": "Transaction",
        }))
        .unwrap();

        assert_eq!(request.account.name(), Some("savings"));
        assert_eq!(request.offset, 20);
        assert_eq!(request.limit, 10);
        assert_eq!(request.resource_address, None);
        assert_eq!(request.transaction_id, Some(transaction_id));
        assert_eq!(request.source_type, Some(BalanceChangeSourceType::Transaction));
    }

    #[test]
    fn balance_change_source_serializes_as_an_internally_tagged_union() {
        let transaction_id = TransactionId::default();
        assert_eq!(
            serde_json::to_value(BalanceChangeSource::Transaction { transaction_id }).unwrap(),
            serde_json::json!({
                "type": "Transaction",
                "transaction_id": transaction_id.to_string(),
            })
        );
        assert_eq!(
            serde_json::to_value(BalanceChangeSource::Scan).unwrap(),
            serde_json::json!({ "type": "Scan" })
        );
        assert_eq!(
            serde_json::to_value(BalanceChangeSource::Recovery).unwrap(),
            serde_json::json!({ "type": "Recovery" })
        );
    }

    fn sample_utxo_address() -> UtxoAddress {
        let resource: ResourceAddress = "resource_0000000000000000000000000000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        UtxoAddress::new(
            resource,
            PedersenCommitmentBytes::from_array([7u8; PedersenCommitmentBytes::length()]).into(),
        )
    }

    #[test]
    fn input_selection_specific_round_trips() {
        let utxo = sample_utxo_address();
        let selection = InputSelection::Specific {
            utxo_addresses: vec![utxo.clone()],
        };
        let json = serde_json::to_value(&selection).unwrap();
        // Exact externally tagged wire shape, consistent with the other variants.
        assert_eq!(
            json,
            serde_json::json!({ "Specific": { "utxo_addresses": [serde_json::to_value(&utxo).unwrap()] } })
        );

        let parsed: InputSelection = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.as_specific().unwrap().to_vec(), vec![utxo]);
        assert!(parsed.as_selection().is_none());
        assert!(parsed.as_from_bucket().is_none());
    }

    #[test]
    fn input_selection_from_bucket_serialization_is_unchanged() {
        let amount = Amount::from(5u64);
        let json = serde_json::to_value(InputSelection::FromBucket {
            revealed_amount: amount,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "FromBucket": { "revealed_amount": serde_json::to_value(amount).unwrap() } })
        );

        let parsed: InputSelection = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.as_from_bucket(), Some(amount));
        assert!(parsed.as_specific().is_none());
    }

    #[test]
    fn input_selection_selection_serialization_is_unchanged() {
        let json = serde_json::to_value(InputSelection::Selection(UtxoInputSelection::ConfidentialOnly)).unwrap();
        assert_eq!(json, serde_json::json!({ "Selection": "ConfidentialOnly" }));

        let parsed: InputSelection = serde_json::from_value(json).unwrap();
        assert!(matches!(
            parsed.as_selection(),
            Some(UtxoInputSelection::ConfidentialOnly)
        ));
        assert!(parsed.as_specific().is_none());
    }
}
