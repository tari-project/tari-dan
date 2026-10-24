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
mod component_address;
pub use component_address::*;
pub mod error;
pub mod permissions;
pub mod serialize;
pub mod types;

use std::borrow::Borrow;

use json::Value;
use reqwest::{
    IntoUrl,
    Url,
    header::{self, AUTHORIZATION, HeaderMap},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json as json;
use serde_json::json;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_ootle_wallet_sdk::models::{KeyBranch, KeyType};
use types::{
    AccountsCreateFreeTestCoinsRequest,
    AccountsCreateFreeTestCoinsResponse,
    AccountsGetFaucetBalanceRequest,
    AccountsGetFaucetBalanceResponse,
    AccountsTransferRequest,
    AccountsTransferResponse,
    AuthCreateApiKeyRequest,
    AuthCreateApiKeyResponse,
    AuthListApiKeysRequest,
    AuthListApiKeysResponse,
    AuthLoginRequest,
    AuthLoginResponse,
    AuthRefreshRequest,
    AuthRevokeApiKeyRequest,
    AuthRevokeApiKeyResponse,
    CallInstructionRequest,
    ClaimBurnRequest,
    ClaimBurnResponse,
    GetNftRequest,
    GetNftResponse,
    ListNftsRequest,
    ListNftsResponse,
    MintFaucetNftRequest,
    MintFaucetNftResponse,
    ProofsCancelRequest,
    ProofsCancelResponse,
    ProofsFinalizeRequest,
    ProofsFinalizeResponse,
    ProofsGenerateRequest,
    ProofsGenerateResponse,
    TransferNftRequest,
    TransferNftResponse,
    WebRtcStartRequest,
    WebRtcStartResponse,
};

use crate::{
    error::WalletDaemonClientError,
    types::{
        AccountGetByKeyIndexRequest,
        AccountGetDefaultRequest,
        AccountGetRequest,
        AccountGetResponse,
        AccountSetDefaultRequest,
        AccountSetDefaultResponse,
        AccountsAssociateStealthResourceRequest,
        AccountsAssociateStealthResourceResponse,
        AccountsCreateOrGetRequest,
        AccountsCreateOrGetResponse,
        AccountsCreateRequest,
        AccountsCreateResponse,
        AccountsCreateStealthTransferStatementRequest,
        AccountsCreateStealthTransferStatementResponse,
        AccountsExportViewKeyRequest,
        AccountsExportViewKeyResponse,
        AccountsGetBalanceChangesRequest,
        AccountsGetBalanceChangesResponse,
        AccountsGetBalancesRequest,
        AccountsGetBalancesResponse,
        AccountsImportViewOnlyRequest,
        AccountsImportViewOnlyResponse,
        AccountsListRequest,
        AccountsListResponse,
        AccountsRenameRequest,
        AccountsRenameResponse,
        AddressBookAddRequest,
        AddressBookAddResponse,
        AddressBookDeleteRequest,
        AddressBookDeleteResponse,
        AddressBookGetRequest,
        AddressBookGetResponse,
        AddressBookListRequest,
        AddressBookListResponse,
        AddressBookUpdateRequest,
        AddressBookUpdateResponse,
        AuthGetMethodRequest,
        AuthGetMethodResponse,
        AuthListSessionsRequest,
        AuthListSessionsResponse,
        AuthRevokeTokenRequest,
        AuthRevokeTokenResponse,
        BurnProofsGetRequest,
        BurnProofsGetResponse,
        BurnProofsListRequest,
        BurnProofsListResponse,
        ClaimValidatorFeesRequest,
        ClaimValidatorFeesResponse,
        ConfidentialCreateOutputProofRequest,
        ConfidentialCreateOutputProofResponse,
        ConfidentialOutputsListRequest,
        ConfidentialOutputsListResponse,
        ConfidentialTransferRequest,
        ConfidentialTransferResponse,
        ConfidentialViewVaultBalanceRequest,
        ConfidentialViewVaultBalanceResponse,
        EncodedJwtString,
        GetValidatorFeesRequest,
        GetValidatorFeesResponse,
        KeysCreateRequest,
        KeysCreateResponse,
        KeysImportRequest,
        KeysImportResponse,
        KeysListRequest,
        KeysListResponse,
        KeysSetActiveRequest,
        KeysSetActiveResponse,
        PublishTemplateRequest,
        PublishTemplateResponse,
        ResourcesGetManyRequest,
        ResourcesGetManyResponse,
        ResourcesGetRequest,
        ResourcesGetResponse,
        SettingsGetResponse,
        SettingsSetRequest,
        SettingsSetResponse,
        SignTemplateMetadataRequest,
        SignTemplateMetadataResponse,
        SigningRequestCreateRequest,
        SigningRequestCreateResponse,
        SigningRequestDecisionRequest,
        SigningRequestDecisionResponse,
        SigningRequestGetRequest,
        SigningRequestGetResponse,
        SigningRequestListRequest,
        SigningRequestListResponse,
        StealthTransferRequest,
        StealthTransferResponse,
        StealthUtxosDecryptValueRequest,
        StealthUtxosDecryptValueResponse,
        StealthUtxosGetValueLookupInfoRequest,
        StealthUtxosGetValueLookupInfoResponse,
        StealthUtxosListRequest,
        StealthUtxosListResponse,
        SubstatesGetRequest,
        SubstatesGetResponse,
        SubstatesListRequest,
        SubstatesListResponse,
        SwapPoolGetExchangeRateRequest,
        SwapPoolGetExchangeRateResponse,
        SwapPoolsListRequest,
        SwapPoolsListResponse,
        TemplatesGetRequest,
        TemplatesGetResponse,
        TemplatesListAuthoredRequest,
        TemplatesListAuthoredResponse,
        TransactionDetectInputsRequest,
        TransactionDetectInputsResponse,
        TransactionGetAllRequest,
        TransactionGetAllResponse,
        TransactionGetRequest,
        TransactionGetResponse,
        TransactionGetResultRequest,
        TransactionGetResultResponse,
        TransactionRequestCreateRequest,
        TransactionRequestCreateResponse,
        TransactionRequestDecisionRequest,
        TransactionRequestDecisionResponse,
        TransactionRequestGetRequest,
        TransactionRequestGetResponse,
        TransactionRequestListRequest,
        TransactionRequestListResponse,
        TransactionRequestSubmitRequest,
        TransactionRequestSubmitResponse,
        TransactionSubmitDryRunRequest,
        TransactionSubmitDryRunResponse,
        TransactionSubmitManifestRequest,
        TransactionSubmitManifestResponse,
        TransactionSubmitRequest,
        TransactionSubmitResponse,
        TransactionWaitResultRequest,
        TransactionWaitResultResponse,
        ValidatorSetGetStatusRequest,
        ValidatorSetGetStatusResponse,
        WalletGetInfoRequest,
        WalletGetInfoResponse,
        WebauthnAlreadyRegisteredRequest,
        WebauthnAlreadyRegisteredResponse,
        WebauthnFinishRegisterRequest,
        WebauthnFinishRegisterResponse,
        WebauthnListPasskeysRequest,
        WebauthnListPasskeysResponse,
        WebauthnRemovePasskeyRequest,
        WebauthnRemovePasskeyResponse,
        WebauthnRenamePasskeyRequest,
        WebauthnRenamePasskeyResponse,
        WebauthnStartAuthRequest,
        WebauthnStartAuthResponse,
        WebauthnStartRegisterRequest,
        WebauthnStartRegisterResponse,
    },
};

#[derive(Debug, Clone)]
pub struct WalletDaemonClient {
    client: reqwest::Client,
    endpoint: Url,
    request_id: i64,
    token: Option<EncodedJwtString>,
}

impl WalletDaemonClient {
    pub fn connect<T: IntoUrl>(endpoint: T, token: Option<EncodedJwtString>) -> Result<Self, WalletDaemonClientError> {
        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = HeaderMap::with_capacity(1);
                headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
                headers
            })
            // Enable cookie storage so that HttpOnly refresh token cookies set by the server
            // are automatically stored and sent back on subsequent requests (e.g. auth.refresh).
            .cookie_store(true)
            .build()?;

        Ok(Self {
            client,
            endpoint: endpoint.into_url()?,
            request_id: 0,
            token,
        })
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub fn set_auth_token(&mut self, token: EncodedJwtString) -> &mut Self {
        self.token = Some(token);
        self
    }

    /// Returns general information about the wallet, including the wallet's public key and network.
    pub async fn get_wallet_info(&mut self) -> Result<WalletGetInfoResponse, WalletDaemonClientError> {
        self.send_request("wallet.get_info", &WalletGetInfoRequest {}).await
    }

    /// The validator sets the wallet verifies the network's proofs against.
    pub async fn get_validator_set_status(&mut self) -> Result<ValidatorSetGetStatusResponse, WalletDaemonClientError> {
        self.send_request("validator_set.get_status", &ValidatorSetGetStatusRequest {})
            .await
    }

    /// Derives the next key for the given [`KeyBranch`].
    pub async fn create_key(&mut self, branch: KeyBranch) -> Result<KeysCreateResponse, WalletDaemonClientError> {
        self.send_request("keys.create", &KeysCreateRequest {
            branch,
            specific_index: None,
        })
        .await
    }

    /// Derives a key at a specific index for the given [`KeyBranch`].
    pub async fn create_specific_key(
        &mut self,
        branch: KeyBranch,
        index: u64,
    ) -> Result<KeysCreateResponse, WalletDaemonClientError> {
        self.send_request("keys.create", &KeysCreateRequest {
            branch,
            specific_index: Some(index),
        })
        .await
    }

    /// Imports an externally-generated secret key into the wallet's keystore, returning the [`KeyId`] it can be
    /// referenced by.
    pub async fn import_key(
        &mut self,
        secret_key: RistrettoSecretKey,
        key_type: KeyType,
    ) -> Result<KeysImportResponse, WalletDaemonClientError> {
        self.send_request("keys.import", &KeysImportRequest { secret_key, key_type })
            .await
    }

    /// Sets the active key index used for signing transactions.
    pub async fn set_active_key(&mut self, index: u64) -> Result<KeysSetActiveResponse, WalletDaemonClientError> {
        self.send_request("keys.set_active", &KeysSetActiveRequest { index })
            .await
    }

    /// Lists all derived keys for the given [`KeyBranch`].
    pub async fn list_keys(&mut self, branch: KeyBranch) -> Result<KeysListResponse, WalletDaemonClientError> {
        self.send_request("keys.list", &KeysListRequest { branch }).await
    }

    /// Fetches a transaction by ID.
    pub async fn get_transaction<T: Borrow<TransactionGetRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionGetResponse, WalletDaemonClientError> {
        self.send_request("transactions.get", request.borrow()).await
    }

    /// Lists transactions with pagination.
    pub async fn get_transactions_all<T: Borrow<TransactionGetAllRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionGetAllResponse, WalletDaemonClientError> {
        self.send_request("transactions.list", request.borrow()).await
    }

    /// Fetches the finalized result of a transaction without blocking.
    pub async fn get_transaction_result<T: Borrow<TransactionGetResultRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionGetResultResponse, WalletDaemonClientError> {
        self.send_request("transactions.get_result", request.borrow()).await
    }

    /// Blocks until the transaction is finalized or the timeout is reached.
    pub async fn wait_transaction_result<T: Borrow<TransactionWaitResultRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionWaitResultResponse, WalletDaemonClientError> {
        self.send_request("transactions.wait_result", request.borrow()).await
    }

    /// Submits a transaction to the network for processing.
    pub async fn submit_transaction<T: Borrow<TransactionSubmitRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionSubmitResponse, WalletDaemonClientError> {
        self.send_request("transactions.submit", request.borrow()).await
    }

    /// Submits a transaction as a dry run without committing it to the network.
    pub async fn submit_transaction_dry_run<T: Borrow<TransactionSubmitDryRunRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionSubmitDryRunResponse, WalletDaemonClientError> {
        self.send_request("transactions.submit_dry_run", request.borrow()).await
    }

    /// Resolves a transaction's inputs without submitting it, returning the transaction with the
    /// detected inputs merged in.
    pub async fn detect_transaction_inputs<T: Borrow<TransactionDetectInputsRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionDetectInputsResponse, WalletDaemonClientError> {
        self.send_request("transactions.detect_inputs", request.borrow()).await
    }

    /// Creates a transaction request for a separately-permissioned principal to approve. The
    /// transaction is stored verbatim and frozen; what the approver views is what submit seals.
    pub async fn create_transaction_request<T: Borrow<TransactionRequestCreateRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionRequestCreateResponse, WalletDaemonClientError> {
        self.send_request("transaction_requests.create", request.borrow()).await
    }

    pub async fn get_transaction_request<T: Borrow<TransactionRequestGetRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionRequestGetResponse, WalletDaemonClientError> {
        self.send_request("transaction_requests.get", request.borrow()).await
    }

    pub async fn list_transaction_requests<T: Borrow<TransactionRequestListRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionRequestListResponse, WalletDaemonClientError> {
        self.send_request("transaction_requests.list", request.borrow()).await
    }

    /// Approves a pending transaction request. Requires an interactive user session: the daemon refuses an API key.
    pub async fn approve_transaction_request<T: Borrow<TransactionRequestDecisionRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionRequestDecisionResponse, WalletDaemonClientError> {
        self.send_request("transaction_requests.approve", request.borrow())
            .await
    }

    pub async fn reject_transaction_request<T: Borrow<TransactionRequestDecisionRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionRequestDecisionResponse, WalletDaemonClientError> {
        self.send_request("transaction_requests.reject", request.borrow()).await
    }

    /// Seals and submits an approved transaction request.
    pub async fn submit_transaction_request<T: Borrow<TransactionRequestSubmitRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionRequestSubmitResponse, WalletDaemonClientError> {
        self.send_request("transaction_requests.submit", request.borrow()).await
    }

    pub async fn create_signing_request<T: Borrow<SigningRequestCreateRequest>>(
        &mut self,
        request: T,
    ) -> Result<SigningRequestCreateResponse, WalletDaemonClientError> {
        self.send_request("signing_requests.create", request.borrow()).await
    }

    pub async fn get_signing_request<T: Borrow<SigningRequestGetRequest>>(
        &mut self,
        request: T,
    ) -> Result<SigningRequestGetResponse, WalletDaemonClientError> {
        self.send_request("signing_requests.get", request.borrow()).await
    }

    pub async fn list_signing_requests<T: Borrow<SigningRequestListRequest>>(
        &mut self,
        request: T,
    ) -> Result<SigningRequestListResponse, WalletDaemonClientError> {
        self.send_request("signing_requests.list", request.borrow()).await
    }

    /// Approve a signing request. Requires an interactive wallet session: an
    /// API key is refused.
    pub async fn approve_signing_request<T: Borrow<SigningRequestDecisionRequest>>(
        &mut self,
        request: T,
    ) -> Result<SigningRequestDecisionResponse, WalletDaemonClientError> {
        self.send_request("signing_requests.approve", request.borrow()).await
    }

    pub async fn reject_signing_request<T: Borrow<SigningRequestDecisionRequest>>(
        &mut self,
        request: T,
    ) -> Result<SigningRequestDecisionResponse, WalletDaemonClientError> {
        self.send_request("signing_requests.reject", request.borrow()).await
    }

    /// Submits a single instruction for execution as a transaction.
    pub async fn submit_instruction<T: Borrow<CallInstructionRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionSubmitResponse, WalletDaemonClientError> {
        self.send_request("transactions.submit_instruction", request.borrow())
            .await
    }

    /// Submits a transaction manifest for execution.
    pub async fn submit_manifest<T: Borrow<TransactionSubmitManifestRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionSubmitManifestResponse, WalletDaemonClientError> {
        self.send_request("transactions.submit_manifest", request.borrow())
            .await
    }

    /// Creates a new account with an optional name and key index.
    pub async fn create_account<T: Borrow<AccountsCreateRequest>>(
        &mut self,
        request: T,
    ) -> Result<AccountsCreateResponse, WalletDaemonClientError> {
        self.send_request("accounts.create", request.borrow()).await
    }

    /// Adds an account from its public key and view secret key. The account can receive and view stealth outputs but
    /// cannot spend them.
    pub async fn accounts_import_view_only<T: Borrow<AccountsImportViewOnlyRequest>>(
        &mut self,
        request: T,
    ) -> Result<AccountsImportViewOnlyResponse, WalletDaemonClientError> {
        self.send_request("accounts.import_view_only", request.borrow()).await
    }

    /// Returns the account public key, view secret key and birthday epoch needed to import the account as view-only
    /// into another wallet.
    pub async fn accounts_export_view_key(
        &mut self,
        account: ComponentAddressOrName,
    ) -> Result<AccountsExportViewKeyResponse, WalletDaemonClientError> {
        self.send_request("accounts.export_view_key", &AccountsExportViewKeyRequest { account })
            .await
    }

    /// Returns an existing account by name, or creates it if it does not exist.
    pub async fn create_or_get_account<T: Borrow<AccountsCreateOrGetRequest>>(
        &mut self,
        request: T,
    ) -> Result<AccountsCreateOrGetResponse, WalletDaemonClientError> {
        self.send_request("accounts.create_or_get", request.borrow()).await
    }

    /// Associates a stealth resource address with an account for tracking stealth outputs.
    pub async fn associate_stealth_resource<T: Borrow<AccountsAssociateStealthResourceRequest>>(
        &mut self,
        request: T,
    ) -> Result<AccountsAssociateStealthResourceResponse, WalletDaemonClientError> {
        self.send_request("accounts.associate_stealth_resource", request.borrow())
            .await
    }

    /// Returns the balances for all vaults in an account, optionally refreshing from the network.
    pub async fn get_account_balances<T: Borrow<AccountsGetBalancesRequest>>(
        &mut self,
        request: T,
    ) -> Result<AccountsGetBalancesResponse, WalletDaemonClientError> {
        self.send_request("accounts.get_balances", request.borrow()).await
    }

    /// Returns paginated balance changes for an account, optionally filtered by resource or transaction.
    pub async fn get_account_balance_changes<T: Borrow<AccountsGetBalanceChangesRequest>>(
        &mut self,
        request: T,
    ) -> Result<AccountsGetBalanceChangesResponse, WalletDaemonClientError> {
        self.send_request("accounts.get_balance_changes", request.borrow())
            .await
    }

    /// Returns unclaimed validator fees for the given shard range.
    pub async fn get_validator_fees<T: Borrow<GetValidatorFeesRequest>>(
        &mut self,
        request: T,
    ) -> Result<GetValidatorFeesResponse, WalletDaemonClientError> {
        self.send_request("validators.get_fees", request.borrow()).await
    }

    /// Claims accumulated validator fees for the given shard range.
    pub async fn claim_validator_fees<T: Borrow<ClaimValidatorFeesRequest>>(
        &mut self,
        request: T,
    ) -> Result<ClaimValidatorFeesResponse, WalletDaemonClientError> {
        self.send_request("validators.claim_fees", request.borrow()).await
    }

    /// Lists accounts with pagination.
    pub async fn list_accounts(
        &mut self,
        offset: u32,
        limit: u32,
    ) -> Result<AccountsListResponse, WalletDaemonClientError> {
        self.send_request("accounts.list", &AccountsListRequest { offset, limit })
            .await
    }

    /// Fetches an account by name or component address.
    pub async fn accounts_get(
        &mut self,
        name_or_address: ComponentAddressOrName,
    ) -> Result<AccountGetResponse, WalletDaemonClientError> {
        self.send_request("accounts.get", &AccountGetRequest { name_or_address })
            .await
    }

    /// Fetches an account by its owner key index.
    pub async fn accounts_get_by_key_index(
        &mut self,
        key_index: u64,
    ) -> Result<AccountGetResponse, WalletDaemonClientError> {
        self.send_request("accounts.get_by_key_index", &AccountGetByKeyIndexRequest { key_index })
            .await
    }

    /// Returns the default account.
    pub async fn accounts_get_default(&mut self) -> Result<AccountGetResponse, WalletDaemonClientError> {
        self.send_request("accounts.get_default", &AccountGetDefaultRequest {})
            .await
    }

    /// Sets the default account used for operations when no account is specified.
    pub async fn accounts_set_default(
        &mut self,
        account: ComponentAddressOrName,
    ) -> Result<AccountSetDefaultResponse, WalletDaemonClientError> {
        self.send_request("accounts.set_default", &AccountSetDefaultRequest { account })
            .await
    }

    /// Renames an account.
    pub async fn accounts_rename(
        &mut self,
        account: ComponentAddressOrName,
        new_name: String,
    ) -> Result<AccountsRenameResponse, WalletDaemonClientError> {
        self.send_request("accounts.rename", &AccountsRenameRequest { account, new_name })
            .await
    }

    /// Transfers resources from one account to a destination public key.
    pub async fn accounts_transfer<T: Borrow<AccountsTransferRequest>>(
        &mut self,
        req: T,
    ) -> Result<AccountsTransferResponse, WalletDaemonClientError> {
        self.send_request("accounts.transfer", req.borrow()).await
    }

    /// Performs a confidential (shielded) transfer between accounts.
    pub async fn accounts_confidential_transfer<T: Borrow<ConfidentialTransferRequest>>(
        &mut self,
        req: T,
    ) -> Result<ConfidentialTransferResponse, WalletDaemonClientError> {
        self.send_request("accounts.confidential_transfer", req.borrow()).await
    }

    /// Performs a stealth transfer, sending resources to a one-time stealth address.
    pub async fn accounts_stealth_transfer<T: Borrow<StealthTransferRequest>>(
        &mut self,
        req: T,
    ) -> Result<StealthTransferResponse, WalletDaemonClientError> {
        self.send_request("accounts.stealth_transfer", req.borrow()).await
    }

    /// Creates a stealth transfer statement that can be shared with the recipient for claiming.
    pub async fn accounts_create_stealth_transfer_statement<
        T: Borrow<AccountsCreateStealthTransferStatementRequest>,
    >(
        &mut self,
        req: T,
    ) -> Result<AccountsCreateStealthTransferStatementResponse, WalletDaemonClientError> {
        self.send_request("accounts.create_stealth_transfer_statement", req.borrow())
            .await
    }

    /// Lists the available burn proof files from the configured burn proofs directory.
    pub async fn list_burn_proofs<T: Borrow<BurnProofsListRequest>>(
        &mut self,
        req: T,
    ) -> Result<BurnProofsListResponse, WalletDaemonClientError> {
        self.send_request("burn_proofs.list", req.borrow()).await
    }

    /// Gets the full contents of a specific burn proof file.
    pub async fn get_burn_proof<T: Borrow<BurnProofsGetRequest>>(
        &mut self,
        req: T,
    ) -> Result<BurnProofsGetResponse, WalletDaemonClientError> {
        self.send_request("burn_proofs.get", req.borrow()).await
    }

    /// Claims a burn transaction, converting burned Minotari into Ootle funds.
    pub async fn claim_burn<T: Borrow<ClaimBurnRequest>>(
        &mut self,
        req: T,
    ) -> Result<ClaimBurnResponse, WalletDaemonClientError> {
        self.send_request("accounts.claim_burn", req.borrow()).await
    }

    /// Generates a confidential transfer proof for use in a confidential transaction.
    pub async fn confidential_create_transfer_proof<T: Borrow<ProofsGenerateRequest>>(
        &mut self,
        req: T,
    ) -> Result<ProofsGenerateResponse, WalletDaemonClientError> {
        self.send_request("confidential.create_transfer_proof", req.borrow())
            .await
    }

    /// Cancels a previously generated confidential transfer proof.
    pub async fn cancel_transfer_proof<T: Borrow<ProofsCancelRequest>>(
        &mut self,
        req: T,
    ) -> Result<ProofsCancelResponse, WalletDaemonClientError> {
        self.send_request("confidential.cancel", req.borrow()).await
    }

    /// Finalizes a confidential transfer proof, making it ready for submission.
    pub async fn finalize_transfer_proof<T: Borrow<ProofsFinalizeRequest>>(
        &mut self,
        req: T,
    ) -> Result<ProofsFinalizeResponse, WalletDaemonClientError> {
        self.send_request("confidential.finalize", req.borrow()).await
    }

    /// Creates a confidential output proof for a confidential deposit or withdrawal.
    pub async fn create_confidential_output_proof<T: Borrow<ConfidentialCreateOutputProofRequest>>(
        &mut self,
        req: T,
    ) -> Result<ConfidentialCreateOutputProofResponse, WalletDaemonClientError> {
        self.send_request("confidential.create_output_proof", req.borrow())
            .await
    }

    /// Creates free test coins for an account. Only available on test networks.
    pub async fn create_free_test_coins<T: Borrow<AccountsCreateFreeTestCoinsRequest>>(
        &mut self,
        req: T,
    ) -> Result<AccountsCreateFreeTestCoinsResponse, WalletDaemonClientError> {
        self.send_request("accounts.create_free_test_coins", req.borrow()).await
    }

    /// Returns the testnet faucet's balance and the amount a claim takes from it.
    pub async fn get_faucet_balance<T: Borrow<AccountsGetFaucetBalanceRequest>>(
        &mut self,
        req: T,
    ) -> Result<AccountsGetFaucetBalanceResponse, WalletDaemonClientError> {
        self.send_request("accounts.get_faucet_balance", req.borrow()).await
    }

    /// Mints one or more NFTs from the faucet into an account.
    pub async fn mint_faucet_nft<T: Borrow<MintFaucetNftRequest>>(
        &mut self,
        req: T,
    ) -> Result<MintFaucetNftResponse, WalletDaemonClientError> {
        self.send_request("nfts.mint_faucet_nft", req.borrow()).await
    }

    /// Fetches a specific NFT by its resource address and ID.
    pub async fn get_account_nft<T: Borrow<GetNftRequest>>(
        &mut self,
        req: T,
    ) -> Result<GetNftResponse, WalletDaemonClientError> {
        self.send_request("nfts.get", req.borrow()).await
    }

    /// Lists NFTs held by an account with pagination.
    pub async fn list_account_nfts<T: Borrow<ListNftsRequest>>(
        &mut self,
        req: T,
    ) -> Result<ListNftsResponse, WalletDaemonClientError> {
        self.send_request("nfts.list", req.borrow()).await
    }

    /// Transfers an NFT from one account to another.
    pub async fn transfer_nft<T: Borrow<TransferNftRequest>>(
        &mut self,
        req: T,
    ) -> Result<TransferNftResponse, WalletDaemonClientError> {
        self.send_request("nfts.transfer", req.borrow()).await
    }

    /// Decrypts and returns the confidential balance of a vault using the provided view key.
    /// The view key must correspond to the public view key of the Resource the vault holds.
    /// If the resource is not confidential, or has no view key configured, this will return an error.
    pub async fn view_vault_balance<T: Borrow<ConfidentialViewVaultBalanceRequest>>(
        &mut self,
        req: T,
    ) -> Result<ConfidentialViewVaultBalanceResponse, WalletDaemonClientError> {
        self.send_request("confidential.view_vault_balance", req.borrow()).await
    }

    /// Requests the current required authentication method to use when authenticating with the wallet daemon.
    pub async fn get_auth_method(&mut self) -> Result<AuthGetMethodResponse, WalletDaemonClientError> {
        self.send_request("auth.method", &AuthGetMethodRequest {}).await
    }

    /// Requests a JWT authentication token with the specified permissions.
    pub async fn auth_request<T: Borrow<AuthLoginRequest>>(
        &mut self,
        req: T,
    ) -> Result<AuthLoginResponse, WalletDaemonClientError> {
        self.send_request("auth.request", req.borrow()).await
    }

    /// Refreshes an authentication token using the refresh token cookie.
    pub async fn auth_refresh(&mut self) -> Result<AuthLoginResponse, WalletDaemonClientError> {
        self.send_request("auth.refresh", &AuthRefreshRequest {}).await
    }

    /// Revokes an active JWT token, invalidating further use.
    pub async fn auth_revoke<T: Borrow<AuthRevokeTokenRequest>>(
        &mut self,
        req: T,
    ) -> Result<AuthRevokeTokenResponse, WalletDaemonClientError> {
        self.send_request("auth.revoke", req.borrow()).await
    }

    /// Lists all active auth sessions.
    pub async fn auth_list_sessions<T: Borrow<AuthListSessionsRequest>>(
        &mut self,
        req: T,
    ) -> Result<AuthListSessionsResponse, WalletDaemonClientError> {
        self.send_request("auth.list_sessions", req.borrow()).await
    }

    /// Mint a new long-lived API key with the supplied permission scopes.
    ///
    /// Requires the current session to hold the `Admin` permission. The raw
    /// key string is returned in the response EXACTLY ONCE — the caller
    /// must persist it immediately (clipboard, secrets manager, env file
    /// for the agent) because the daemon only stores a SHA-256 hash.
    ///
    /// If the granted permissions include `Admin`, the request must set
    /// `confirm_admin: true`. This is a deliberate speed-bump so a UI
    /// checkbox can be the gate rather than the request payload alone.
    pub async fn auth_create_api_key<T: Borrow<AuthCreateApiKeyRequest>>(
        &mut self,
        req: T,
    ) -> Result<AuthCreateApiKeyResponse, WalletDaemonClientError> {
        self.send_request("auth.create_api_key", req.borrow()).await
    }

    /// List all API keys issued by this wallet, active and revoked. Returns
    /// only metadata — never the raw key material (the daemon does not
    /// store it).
    pub async fn auth_list_api_keys<T: Borrow<AuthListApiKeysRequest>>(
        &mut self,
        req: T,
    ) -> Result<AuthListApiKeysResponse, WalletDaemonClientError> {
        self.send_request("auth.list_api_keys", req.borrow()).await
    }

    /// Revoke an API key by id. Revocation is immediate — subsequent
    /// authentication attempts using the revoked key will fail at the
    /// storage layer's active-row filter.
    pub async fn auth_revoke_api_key<T: Borrow<AuthRevokeApiKeyRequest>>(
        &mut self,
        req: T,
    ) -> Result<AuthRevokeApiKeyResponse, WalletDaemonClientError> {
        self.send_request("auth.revoke_api_key", req.borrow()).await
    }

    /// Initiates a WebRTC signalling session with the wallet daemon.
    pub async fn webrtc_start<T: Borrow<WebRtcStartRequest>>(
        &mut self,
        req: T,
    ) -> Result<WebRtcStartResponse, WalletDaemonClientError> {
        self.send_request("webrtc.start", req.borrow()).await
    }

    /// Publishes a WASM template to the network.
    pub async fn publish_template<T: Borrow<PublishTemplateRequest>>(
        &mut self,
        request: T,
    ) -> Result<PublishTemplateResponse, WalletDaemonClientError> {
        self.send_request("transactions.publish_template", request.borrow())
            .await
    }

    /// Signs template metadata with the specified key, returning the signature and CBOR-encoded metadata.
    pub async fn sign_template_metadata<T: Borrow<SignTemplateMetadataRequest>>(
        &mut self,
        request: T,
    ) -> Result<SignTemplateMetadataResponse, WalletDaemonClientError> {
        self.send_request("templates.sign_metadata", request.borrow()).await
    }

    /// Lists stealth UTXOs for an account, with optional resource filtering.
    pub async fn stealth_utxos_list<T: Borrow<StealthUtxosListRequest>>(
        &mut self,
        request: T,
    ) -> Result<StealthUtxosListResponse, WalletDaemonClientError> {
        self.send_request("stealth_utxos.list", request.borrow()).await
    }

    pub async fn confidential_list_outputs<T: Borrow<ConfidentialOutputsListRequest>>(
        &mut self,
        request: T,
    ) -> Result<ConfidentialOutputsListResponse, WalletDaemonClientError> {
        self.send_request("confidential.list_outputs", request.borrow()).await
    }

    /// Decrypts the value of a stealth UTXO using the provided view key.
    /// The view key must correspond to the public view key of the Resource the UTXO represents.
    /// If the resource is not stealth, or has no view key configured, this will return an error.
    pub async fn stealth_utxos_decrypt_value<T: Borrow<StealthUtxosDecryptValueRequest>>(
        &mut self,
        request: T,
    ) -> Result<StealthUtxosDecryptValueResponse, WalletDaemonClientError> {
        self.send_request("stealth_utxos.decrypt_value", request.borrow()).await
    }

    /// Returns information about the value lookup table configured for confidential/stealth balance
    /// decryption (coverage range, format and size).
    pub async fn stealth_utxos_get_value_lookup_info<T: Borrow<StealthUtxosGetValueLookupInfoRequest>>(
        &mut self,
        request: T,
    ) -> Result<StealthUtxosGetValueLookupInfoResponse, WalletDaemonClientError> {
        self.send_request("stealth_utxos.get_value_lookup_info", request.borrow())
            .await
    }

    /// Returns the wallet daemon's current settings.
    pub async fn get_settings(&mut self) -> Result<SettingsGetResponse, WalletDaemonClientError> {
        self.send_request("settings.get", &()).await
    }

    /// Updates the wallet daemon's settings.
    pub async fn set_settings<T: Borrow<SettingsSetRequest>>(
        &mut self,
        req: T,
    ) -> Result<SettingsSetResponse, WalletDaemonClientError> {
        self.send_request("settings.set", req.borrow()).await
    }

    /// Fetches a substate by its address from the network.
    pub async fn get_substate<T: Borrow<SubstatesGetRequest>>(
        &mut self,
        req: T,
    ) -> Result<SubstatesGetResponse, WalletDaemonClientError> {
        self.send_request("substates.get", req.borrow()).await
    }

    /// Lists substates owned by an account or matching a filter.
    pub async fn list_substates<T: Borrow<SubstatesListRequest>>(
        &mut self,
        req: T,
    ) -> Result<SubstatesListResponse, WalletDaemonClientError> {
        self.send_request("substates.list", req.borrow()).await
    }

    /// Gets the exchange rate for a swap pool by fetching its vault balances.
    pub async fn swap_pool_get_exchange_rate<T: Borrow<SwapPoolGetExchangeRateRequest>>(
        &mut self,
        req: T,
    ) -> Result<SwapPoolGetExchangeRateResponse, WalletDaemonClientError> {
        self.send_request("swap_pools.get_exchange_rate", req.borrow()).await
    }

    /// Lists swap pools known to the indexer with their current exchange rates.
    pub async fn swap_pools_list<T: Borrow<SwapPoolsListRequest>>(
        &mut self,
        req: T,
    ) -> Result<SwapPoolsListResponse, WalletDaemonClientError> {
        self.send_request("swap_pools.list", req.borrow()).await
    }

    /// Fetches a resource this wallet has stored.
    pub async fn resources_get<T: Borrow<ResourcesGetRequest>>(
        &mut self,
        req: T,
    ) -> Result<ResourcesGetResponse, WalletDaemonClientError> {
        self.send_request("resources.get", req.borrow()).await
    }

    /// Fetches the resources this wallet has stored out of the given addresses.
    pub async fn resources_get_many<T: Borrow<ResourcesGetManyRequest>>(
        &mut self,
        req: T,
    ) -> Result<ResourcesGetManyResponse, WalletDaemonClientError> {
        self.send_request("resources.get_many", req.borrow()).await
    }

    /// Fetches a template by its address, including its function definitions.
    pub async fn get_template<T: Borrow<TemplatesGetRequest>>(
        &mut self,
        req: T,
    ) -> Result<TemplatesGetResponse, WalletDaemonClientError> {
        self.send_request("templates.get", req.borrow()).await
    }

    /// Lists templates authored by the wallet's public key.
    pub async fn list_authored_templates<T: Borrow<TemplatesListAuthoredRequest>>(
        &mut self,
        req: T,
    ) -> Result<TemplatesListAuthoredResponse, WalletDaemonClientError> {
        self.send_request("templates.list_authored", req.borrow()).await
    }

    /// Checks whether a WebAuthn credential has already been registered.
    pub async fn webauthn_already_registered<T: Borrow<WebauthnAlreadyRegisteredRequest>>(
        &mut self,
        req: T,
    ) -> Result<WebauthnAlreadyRegisteredResponse, WalletDaemonClientError> {
        self.send_request("webauthn.already_registered", req.borrow()).await
    }

    /// Starts a WebAuthn credential registration flow, returning a challenge.
    pub async fn webauthn_start_registration<T: Borrow<WebauthnStartRegisterRequest>>(
        &mut self,
        req: T,
    ) -> Result<WebauthnStartRegisterResponse, WalletDaemonClientError> {
        self.send_request("webauthn.reg_start", req.borrow()).await
    }

    /// Completes a WebAuthn credential registration flow.
    pub async fn webauthn_finish_registration<T: Borrow<WebauthnFinishRegisterRequest>>(
        &mut self,
        req: T,
    ) -> Result<WebauthnFinishRegisterResponse, WalletDaemonClientError> {
        self.send_request("webauthn.reg_finish", req.borrow()).await
    }

    /// Starts a WebAuthn authentication flow, returning a challenge.
    pub async fn webauthn_start_auth<T: Borrow<WebauthnStartAuthRequest>>(
        &mut self,
        req: T,
    ) -> Result<WebauthnStartAuthResponse, WalletDaemonClientError> {
        self.send_request("webauthn.auth_start", req.borrow()).await
    }

    /// Lists all WebAuthn passkeys registered in this wallet (requires `Admin` scope).
    pub async fn webauthn_list_passkeys<T: Borrow<WebauthnListPasskeysRequest>>(
        &mut self,
        req: T,
    ) -> Result<WebauthnListPasskeysResponse, WalletDaemonClientError> {
        self.send_request("webauthn.list_passkeys", req.borrow()).await
    }

    /// Renames a registered WebAuthn passkey (requires `Admin` scope).
    pub async fn webauthn_rename_passkey<T: Borrow<WebauthnRenamePasskeyRequest>>(
        &mut self,
        req: T,
    ) -> Result<WebauthnRenamePasskeyResponse, WalletDaemonClientError> {
        self.send_request("webauthn.rename_passkey", req.borrow()).await
    }

    /// Removes a registered WebAuthn passkey (requires `Admin` scope). Refuses to remove the last passkey.
    pub async fn webauthn_remove_passkey<T: Borrow<WebauthnRemovePasskeyRequest>>(
        &mut self,
        req: T,
    ) -> Result<WebauthnRemovePasskeyResponse, WalletDaemonClientError> {
        self.send_request("webauthn.remove_passkey", req.borrow()).await
    }

    // Address book

    /// Adds a new entry to the address book.
    pub async fn address_book_add<T: Borrow<AddressBookAddRequest>>(
        &mut self,
        req: T,
    ) -> Result<AddressBookAddResponse, WalletDaemonClientError> {
        self.send_request("address_book.add", req.borrow()).await
    }

    /// Lists all address book entries.
    pub async fn address_book_list(&mut self) -> Result<AddressBookListResponse, WalletDaemonClientError> {
        self.send_request("address_book.list", &AddressBookListRequest {}).await
    }

    /// Gets a specific address book entry by name.
    pub async fn address_book_get<T: Borrow<AddressBookGetRequest>>(
        &mut self,
        req: T,
    ) -> Result<AddressBookGetResponse, WalletDaemonClientError> {
        self.send_request("address_book.get", req.borrow()).await
    }

    /// Updates an existing address book entry.
    pub async fn address_book_update<T: Borrow<AddressBookUpdateRequest>>(
        &mut self,
        req: T,
    ) -> Result<AddressBookUpdateResponse, WalletDaemonClientError> {
        self.send_request("address_book.update", req.borrow()).await
    }

    /// Deletes an address book entry by name.
    pub async fn address_book_delete<T: Borrow<AddressBookDeleteRequest>>(
        &mut self,
        req: T,
    ) -> Result<AddressBookDeleteResponse, WalletDaemonClientError> {
        self.send_request("address_book.delete", req.borrow()).await
    }

    fn next_request_id(&mut self) -> i64 {
        self.request_id += 1;
        self.request_id
    }

    async fn jrpc_call<T: Serialize>(&mut self, method: &str, params: &T) -> Result<Value, WalletDaemonClientError> {
        let request_json = json!(
            {
                "jsonrpc": "2.0",
                "id": self.next_request_id(),
                "method": method,
                "params": params,
            }
        );
        let mut builder = self.client.post(self.endpoint.clone());
        if let Some(token) = &self.token {
            // If we don't have the token and the method is anything else than "auth.login" it will fail.
            builder = builder.header(AUTHORIZATION, format!("Bearer {}", token.as_str()));
        }
        let resp = builder.body(request_json.to_string()).send().await?;
        let val = resp.json().await?;
        jsonrpc_result(val)
    }

    async fn send_request<T: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: &T,
    ) -> Result<R, WalletDaemonClientError> {
        let params = json::to_value(params).map_err(|e| WalletDaemonClientError::SerializeRequest {
            source: e,
            method: method.to_string(),
        })?;
        let resp = self.jrpc_call(method, &params).await?;
        match serde_json::from_value(resp) {
            Ok(r) => Ok(r),
            Err(e) => Err(WalletDaemonClientError::DeserializeResponse {
                source: e,
                method: method.to_string(),
            }),
        }
    }
}

fn jsonrpc_result(val: json::Value) -> Result<json::Value, WalletDaemonClientError> {
    if let Some(err) = val.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown error")
            .to_string();
        return Err(WalletDaemonClientError::from_error_code(code, message));
    }

    let result = val
        .get("result")
        .ok_or_else(|| WalletDaemonClientError::InvalidResponse {
            message: "Missing result field".to_string(),
        })?;
    Ok(result.clone())
}
