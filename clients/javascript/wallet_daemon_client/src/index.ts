/*
 * //  Copyright 2024 The Tari Project
 * //  SPDX-License-Identifier: BSD-3-Clause
 */

import type {
  AccountGetByKeyIndexRequest,
  AccountGetDefaultRequest,
  AccountGetRequest,
  AccountGetResponse,
  AccountsAssociateStealthResourceRequest,
  AccountsAssociateStealthResourceResponse,
  AccountsCreateFreeTestCoinsRequest,
  AccountsCreateFreeTestCoinsResponse,
  AccountsCreateOrGetRequest,
  AccountsCreateOrGetResponse,
  AccountsCreateRequest,
  AccountsCreateResponse,
  AccountsCreateStealthTransferStatementRequest,
  AccountsCreateStealthTransferStatementResponse,
  AccountSetDefaultRequest,
  AccountSetDefaultResponse,
  AccountsExportViewKeyRequest,
  AccountsExportViewKeyResponse,
  AccountsGetBalanceChangesRequest,
  AccountsGetBalanceChangesResponse,
  AccountsGetBalancesRequest,
  AccountsGetBalancesResponse,
  AccountsGetFaucetBalanceRequest,
  AccountsGetFaucetBalanceResponse,
  AccountsImportViewOnlyRequest,
  AccountsImportViewOnlyResponse,
  AccountsListRequest,
  AccountsListResponse,
  AccountsRenameRequest,
  AccountsRenameResponse,
  AccountsTransferRequest,
  AccountsTransferResponse,
  AddressBookAddRequest,
  AddressBookAddResponse,
  AddressBookDeleteRequest,
  AddressBookDeleteResponse,
  AddressBookGetRequest,
  AddressBookGetResponse,
  AddressBookListResponse,
  AddressBookUpdateRequest,
  AddressBookUpdateResponse,
  AuthCreateApiKeyRequest,
  AuthCreateApiKeyResponse,
  AuthCredentials,
  AuthGetMethodResponse,
  AuthListApiKeysRequest,
  AuthListApiKeysResponse,
  AuthListSessionsRequest,
  AuthListSessionsResponse,
  AuthLoginRequest,
  AuthLoginResponse,
  AuthRefreshResponse,
  AuthRevokeApiKeyRequest,
  AuthRevokeApiKeyResponse,
  AuthRevokeTokenRequest,
  AuthRevokeTokenResponse,
  BurnProofsGetRequest,
  BurnProofsGetResponse,
  BurnProofsListRequest,
  BurnProofsListResponse,
  CallInstructionRequest,
  ClaimBurnRequest,
  ClaimBurnResponse,
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
  GetNftRequest,
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
  ListNftsRequest,
  ListNftsResponse,
  MintFaucetNftRequest,
  MintFaucetNftResponse,
  NonFungibleToken,
  Permission,
  ProofsCancelRequest,
  ProofsCancelResponse,
  ProofsFinalizeRequest,
  ProofsFinalizeResponse,
  ProofsGenerateRequest,
  ProofsGenerateResponse,
  PublishTemplateRequest,
  PublishTemplateResponse,
  rejectReasonToString,
  ResourcesGetManyRequest,
  ResourcesGetManyResponse,
  ResourcesGetRequest,
  ResourcesGetResponse,
  SettingsGetResponse,
  SettingsSetRequest,
  SettingsSetResponse,
  SigningRequestCreateRequest,
  SigningRequestCreateResponse,
  SigningRequestDecisionRequest,
  SigningRequestDecisionResponse,
  SigningRequestGetRequest,
  SigningRequestGetResponse,
  SigningRequestListRequest,
  SigningRequestListResponse,
  SignTemplateMetadataRequest,
  SignTemplateMetadataResponse,
  StealthTransferRequest,
  StealthTransferResponse,
  StealthUtxosDecryptValueRequest,
  StealthUtxosDecryptValueResponse,
  StealthUtxosGetValueLookupInfoRequest,
  StealthUtxosGetValueLookupInfoResponse,
  StealthUtxosListRequest,
  StealthUtxosListResponse,
  stringToSubstateId,
  substateIdToString,
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
  TransactionSubmitDryRunResponse,
  TransactionSubmitManifestRequest,
  TransactionSubmitManifestResponse,
  TransactionSubmitRequest,
  TransactionSubmitResponse,
  TransactionWaitResultRequest,
  TransactionWaitResultResponse,
  TransferNftRequest,
  TransferNftResponse,
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
  WebRtcStartRequest,
  WebRtcStartResponse,
} from "@tari-project/ootle-ts-bindings";
import { FetchRpcTransport, FetchRpcTransportOptions, RpcErrorResponse, RpcResponse, RpcTransport } from "./transports";

export * as transports from "./transports";

export { rejectReasonToString, stringToSubstateId, substateIdToString };

/**
 * Prefix the daemon puts on every API key it mints. A bearer carrying it is
 * resolved against the api_keys table rather than decoded as a JWT.
 */
const API_KEY_PREFIX = "tw_";

/** True if `token` is an API key rather than a session JWT. */
export function isApiKey(token: string): boolean {
  return token.startsWith(API_KEY_PREFIX);
}

export class WalletDaemonClient<T extends RpcTransport = FetchRpcTransport> {
  private token: string | null;
  private transport: T;
  private id: number;
  private reauthEnabled: boolean = true;
  private hasWarnedApiKeyReauth: boolean = false;

  constructor(transport: T) {
    this.token = null;
    this.transport = transport;
    this.id = 0;
  }

  public static new<T extends RpcTransport>(transport: T): WalletDaemonClient<T> {
    return new WalletDaemonClient(transport);
  }

  public static usingFetchTransport(url: string, options?: FetchRpcTransportOptions): WalletDaemonClient {
    return WalletDaemonClient.new(FetchRpcTransport.new(url, options));
  }

  public setReauthenticationEnabled(enabled: boolean) {
    this.reauthEnabled = enabled;
    this.warnIfApiKeyReauth();
  }

  public isAuthenticated() {
    return Boolean(this.token);
  }

  public setToken(token: string) {
    this.token = token;
    this.warnIfApiKeyReauth();
  }

  private clearToken() {
    this.token = null;
    this.hasWarnedApiKeyReauth = false;
  }

  /**
   * Reauthentication is inert for an API key: the key is validated against the
   * api_keys table on every call rather than expiring into a refreshable
   * session, and `auth.refresh` consumes the login cookie that an API-key
   * holder never receives. Warn once so a 401 from a revoked or under-scoped
   * key isn't mistaken for a refresh that failed.
   *
   * Called from both setters so the warning fires whichever order they run in.
   */
  private warnIfApiKeyReauth() {
    if (this.hasWarnedApiKeyReauth || !this.reauthEnabled || !this.token || !isApiKey(this.token)) {
      return;
    }
    this.hasWarnedApiKeyReauth = true;
    console.warn(
      "WalletDaemonClient: reauthentication is enabled but the credential is an API key. API keys cannot be " +
        "refreshed; a 401 means the key is revoked, expired, or lacks the required permission.",
    );
  }

  public getToken(): string | null {
    return this.token;
  }

  public getTransport(): T {
    return this.transport;
  }

  public authGetMethod(): Promise<AuthGetMethodResponse> {
    return this.sendRequest("auth.method", {});
  }

  public authListSessions(params: AuthListSessionsRequest): Promise<AuthListSessionsResponse> {
    return this.sendRequest("auth.list_sessions", params);
  }

  public async authRequest(permissions: Permission[], credentials: AuthCredentials): Promise<string> {
    let request: AuthLoginRequest = {
      permissions: permissions,
      credentials,
    };
    let resp = await this.sendRequest<AuthLoginResponse>("auth.request", request);
    return resp.token;
  }

  public authRevoke(params: AuthRevokeTokenRequest): Promise<AuthRevokeTokenResponse> {
    return this.sendRequest("auth.revoke", params);
  }

  public authRefresh(): Promise<AuthRefreshResponse> {
    return this.sendRequest("auth.refresh");
  }

  // ---------------------------------------------------------------------------
  // API key management (issue #1957). All three methods require the active
  // session to hold the `Admin` permission; the daemon rejects non-admin calls
  // at the JSON-RPC layer.
  // ---------------------------------------------------------------------------

  /**
   * Mint a new long-lived API key with the supplied permission scopes.
   *
   * The response contains the raw key material exactly once. The caller MUST
   * persist it immediately — the daemon only stores a SHA-256 hash and the
   * key cannot be retrieved later.
   *
   * If the granted permissions include `Admin`, the request must set
   * `confirm_admin: true`.
   */
  public transactionRequestsCreate(params: TransactionRequestCreateRequest): Promise<TransactionRequestCreateResponse> {
    return this.sendRequest("transaction_requests.create", params);
  }

  public transactionRequestsGet(params: TransactionRequestGetRequest): Promise<TransactionRequestGetResponse> {
    return this.sendRequest("transaction_requests.get", params);
  }

  public transactionRequestsList(
    params: TransactionRequestListRequest = { status: null },
  ): Promise<TransactionRequestListResponse> {
    return this.sendRequest("transaction_requests.list", params);
  }

  public transactionRequestsApprove(
    params: TransactionRequestDecisionRequest,
  ): Promise<TransactionRequestDecisionResponse> {
    return this.sendRequest("transaction_requests.approve", params);
  }

  public transactionRequestsReject(
    params: TransactionRequestDecisionRequest,
  ): Promise<TransactionRequestDecisionResponse> {
    return this.sendRequest("transaction_requests.reject", params);
  }

  public transactionRequestsSubmit(params: TransactionRequestSubmitRequest): Promise<TransactionRequestSubmitResponse> {
    return this.sendRequest("transaction_requests.submit", params);
  }

  public signingRequestsCreate(params: SigningRequestCreateRequest): Promise<SigningRequestCreateResponse> {
    return this.sendRequest("signing_requests.create", params);
  }

  public signingRequestsGet(params: SigningRequestGetRequest): Promise<SigningRequestGetResponse> {
    return this.sendRequest("signing_requests.get", params);
  }

  public signingRequestsList(
    params: SigningRequestListRequest = { status: null },
  ): Promise<SigningRequestListResponse> {
    return this.sendRequest("signing_requests.list", params);
  }

  /** Requires an interactive wallet session; the daemon refuses an API key. */
  public signingRequestsApprove(params: SigningRequestDecisionRequest): Promise<SigningRequestDecisionResponse> {
    return this.sendRequest("signing_requests.approve", params);
  }

  /** Requires an interactive wallet session; the daemon refuses an API key. */
  public signingRequestsReject(params: SigningRequestDecisionRequest): Promise<SigningRequestDecisionResponse> {
    return this.sendRequest("signing_requests.reject", params);
  }

  public authCreateApiKey(params: AuthCreateApiKeyRequest): Promise<AuthCreateApiKeyResponse> {
    return this.sendRequest("auth.create_api_key", params);
  }

  /**
   * List API keys. By default returns only active (non-revoked) keys; pass
   * `{ include_revoked: true }` to retrieve the full audit history.
   * Expired keys are always included. Never returns raw key material.
   */
  public authListApiKeys(
    params: AuthListApiKeysRequest = { include_revoked: false },
  ): Promise<AuthListApiKeysResponse> {
    return this.sendRequest("auth.list_api_keys", params);
  }

  /**
   * Revoke an API key by id. Revocation is immediate — the daemon's auth
   * lookup filter excludes revoked rows so any in-flight request using the
   * revoked key will fail.
   */
  public authRevokeApiKey(params: AuthRevokeApiKeyRequest): Promise<AuthRevokeApiKeyResponse> {
    return this.sendRequest("auth.revoke_api_key", params);
  }

  /**
   * Authenticate this client using an API key string. Agents submit the raw
   * key as the `Authorization: Bearer …` header on every JSON-RPC call —
   * the daemon resolves the `tw_`-prefixed bearer against the api_keys
   * table on each call, so there is no `auth.request` round-trip and no
   * JWT exchange. Equivalent to `setToken(apiKey)` but reads better at the
   * call site.
   */
  public authenticateWithApiKey(apiKey: string): void {
    this.setToken(apiKey);
  }

  public walletGetInfo(): Promise<WalletGetInfoResponse> {
    return this.sendRequest("wallet.get_info", {} as WalletGetInfoRequest);
  }

  public validatorSetGetStatus(): Promise<ValidatorSetGetStatusResponse> {
    return this.sendRequest("validator_set.get_status", {} as ValidatorSetGetStatusRequest);
  }

  public accountsCreate(params: AccountsCreateRequest): Promise<AccountsCreateResponse> {
    return this.sendRequest("accounts.create", params);
  }

  public accountsImportViewOnly(params: AccountsImportViewOnlyRequest): Promise<AccountsImportViewOnlyResponse> {
    return this.sendRequest("accounts.import_view_only", params);
  }

  public accountsExportViewKey(params: AccountsExportViewKeyRequest): Promise<AccountsExportViewKeyResponse> {
    return this.sendRequest("accounts.export_view_key", params);
  }

  public accountsRename(params: AccountsRenameRequest): Promise<AccountsRenameResponse> {
    return this.sendRequest("accounts.rename", params);
  }

  public burnProofsList(params: BurnProofsListRequest): Promise<BurnProofsListResponse> {
    return this.sendRequest("burn_proofs.list", params);
  }

  public burnProofsGet(params: BurnProofsGetRequest): Promise<BurnProofsGetResponse> {
    return this.sendRequest("burn_proofs.get", params);
  }

  public accountsClaimBurn(params: ClaimBurnRequest): Promise<ClaimBurnResponse> {
    return this.sendRequest("accounts.claim_burn", params);
  }

  public accountsAssociateStealthResource(
    params: AccountsAssociateStealthResourceRequest,
  ): Promise<AccountsAssociateStealthResourceResponse> {
    return this.sendRequest("accounts.associate_stealth_resource", params);
  }

  public accountsGetBalances(params: AccountsGetBalancesRequest): Promise<AccountsGetBalancesResponse> {
    return this.sendRequest("accounts.get_balances", params);
  }

  public accountsGetBalanceChanges(
    params: AccountsGetBalanceChangesRequest,
  ): Promise<AccountsGetBalanceChangesResponse> {
    return this.sendRequest("accounts.get_balance_changes", params);
  }

  public accountsList(params: AccountsListRequest): Promise<AccountsListResponse> {
    return this.sendRequest("accounts.list", params);
  }

  public accountsGet(params: AccountGetRequest): Promise<AccountGetResponse> {
    return this.sendRequest("accounts.get", params);
  }

  public accountsGetByKeyIndex(params: AccountGetByKeyIndexRequest): Promise<AccountGetResponse> {
    return this.sendRequest("accounts.get_by_key_index", params);
  }

  public accountsCreateOrGet(params: AccountsCreateOrGetRequest): Promise<AccountsCreateOrGetResponse> {
    return this.sendRequest("accounts.create_or_get", params);
  }

  public accountsTransfer(params: AccountsTransferRequest): Promise<AccountsTransferResponse> {
    return this.sendRequest("accounts.transfer", params);
  }

  public confidentialTransfer(params: ConfidentialTransferRequest): Promise<ConfidentialTransferResponse> {
    return this.sendRequest("accounts.confidential_transfer", params);
  }

  public stealthTransfer(params: StealthTransferRequest): Promise<StealthTransferResponse> {
    return this.sendRequest("accounts.stealth_transfer", params);
  }

  public accountsCreateStealthTransferStatement(
    params: AccountsCreateStealthTransferStatementRequest,
  ): Promise<AccountsCreateStealthTransferStatementResponse> {
    return this.sendRequest("accounts.create_stealth_transfer_statement", params);
  }

  public accountsGetDefault(params: AccountGetDefaultRequest): Promise<AccountGetResponse> {
    return this.sendRequest("accounts.get_default", params);
  }

  public accountsSetDefault(params: AccountSetDefaultRequest): Promise<AccountSetDefaultResponse> {
    return this.sendRequest("accounts.set_default", params);
  }

  public submitTransaction(params: TransactionSubmitRequest): Promise<TransactionSubmitResponse> {
    return this.sendRequest("transactions.submit", params);
  }

  public submitInstruction(params: CallInstructionRequest): Promise<TransactionSubmitResponse> {
    return this.sendRequest("transactions.submit_instruction", params);
  }

  public submitTransactionDryRun(params: TransactionSubmitRequest): Promise<TransactionSubmitDryRunResponse> {
    return this.sendRequest("transactions.submit_dry_run", params);
  }

  public transactionsDetectInputs(params: TransactionDetectInputsRequest): Promise<TransactionDetectInputsResponse> {
    return this.sendRequest("transactions.detect_inputs", params);
  }

  public submitTransactionManifest(
    params: TransactionSubmitManifestRequest,
  ): Promise<TransactionSubmitManifestResponse> {
    return this.sendRequest("transactions.submit_manifest", params);
  }

  public publishTemplate(params: PublishTemplateRequest): Promise<PublishTemplateResponse> {
    return this.sendRequest("transactions.publish_template", params);
  }

  public substatesGet(params: SubstatesGetRequest): Promise<SubstatesGetResponse> {
    return this.sendRequest("substates.get", params);
  }

  public substatesList(params: SubstatesListRequest): Promise<SubstatesListResponse> {
    return this.sendRequest("substates.list", params);
  }

  public swapPoolGetExchangeRate(params: SwapPoolGetExchangeRateRequest): Promise<SwapPoolGetExchangeRateResponse> {
    return this.sendRequest("swap_pools.get_exchange_rate", params);
  }

  public swapPoolsList(params: SwapPoolsListRequest): Promise<SwapPoolsListResponse> {
    return this.sendRequest("swap_pools.list", params);
  }

  public transactionsList(params: TransactionGetAllRequest): Promise<TransactionGetAllResponse> {
    return this.sendRequest("transactions.list", params);
  }

  public transactionsGet(params: TransactionGetRequest): Promise<TransactionGetResponse> {
    return this.sendRequest("transactions.get", params);
  }

  public getTransactionResult(params: TransactionGetResultRequest): Promise<TransactionGetResultResponse> {
    return this.sendRequest("transactions.get_result", params);
  }

  public waitForTransactionResult(params: TransactionWaitResultRequest): Promise<TransactionWaitResultResponse> {
    return this.sendRequest("transactions.wait_result", params);
  }

  public templatesGet(params: TemplatesGetRequest): Promise<TemplatesGetResponse> {
    return this.sendRequest("templates.get", params);
  }

  public templatesListAuthored(params: TemplatesListAuthoredRequest): Promise<TemplatesListAuthoredResponse> {
    return this.sendRequest("templates.list_authored", params);
  }

  public templatesSignMetadata(params: SignTemplateMetadataRequest): Promise<SignTemplateMetadataResponse> {
    return this.sendRequest("templates.sign_metadata", params);
  }

  public createFreeTestCoins(params: AccountsCreateFreeTestCoinsRequest): Promise<AccountsCreateFreeTestCoinsResponse> {
    return this.sendRequest("accounts.create_free_test_coins", params);
  }

  public getFaucetBalance(params: AccountsGetFaucetBalanceRequest): Promise<AccountsGetFaucetBalanceResponse> {
    return this.sendRequest("accounts.get_faucet_balance", params);
  }

  public createKey(params: KeysCreateRequest): Promise<KeysCreateResponse> {
    return this.sendRequest("keys.create", params);
  }

  public keysSetActive(params: KeysSetActiveRequest): Promise<KeysSetActiveResponse> {
    return this.sendRequest("keys.set_active", params);
  }

  public listKeys(params: KeysListRequest): Promise<KeysListResponse> {
    return this.sendRequest("keys.list", params);
  }

  public importKey(params: KeysImportRequest): Promise<KeysImportResponse> {
    return this.sendRequest("keys.import", params);
  }

  public viewVaultBalance(params: ConfidentialViewVaultBalanceRequest): Promise<ConfidentialViewVaultBalanceResponse> {
    return this.sendRequest("confidential.view_vault_balance", params);
  }

  public confidentialCreateTransferProof(params: ProofsGenerateRequest): Promise<ProofsGenerateResponse> {
    return this.sendRequest("confidential.create_transfer_proof", params);
  }

  public confidentialFinalize(params: ProofsFinalizeRequest): Promise<ProofsFinalizeResponse> {
    return this.sendRequest("confidential.finalize", params);
  }

  public confidentialCancel(params: ProofsCancelRequest): Promise<ProofsCancelResponse> {
    return this.sendRequest("confidential.cancel", params);
  }

  public confidentialCreateOutputProof(
    params: ConfidentialCreateOutputProofRequest,
  ): Promise<ConfidentialCreateOutputProofResponse> {
    return this.sendRequest("confidential.create_output_proof", params);
  }

  public nftsList(params: ListNftsRequest): Promise<ListNftsResponse> {
    return this.sendRequest("nfts.list", params);
  }

  public nftsGet(params: GetNftRequest): Promise<NonFungibleToken> {
    return this.sendRequest("nfts.get", params);
  }

  public nftTransfer(params: TransferNftRequest): Promise<TransferNftResponse> {
    return this.sendRequest("nfts.transfer", params);
  }

  public mintFaucetNfts(params: MintFaucetNftRequest): Promise<MintFaucetNftResponse> {
    return this.sendRequest("nfts.mint_faucet_nft", params);
  }

  public validatorsClaimFees(params: ClaimValidatorFeesRequest): Promise<ClaimValidatorFeesResponse> {
    return this.sendRequest("validators.claim_fees", params);
  }

  public validatorsGetFees(params: GetValidatorFeesRequest): Promise<GetValidatorFeesResponse> {
    return this.sendRequest("validators.get_fees", params);
  }

  public webrtcStart(params: WebRtcStartRequest): Promise<WebRtcStartResponse> {
    return this.sendRequest("webrtc.start", params);
  }

  public settingsGet(): Promise<SettingsGetResponse> {
    return this.sendRequest("settings.get");
  }

  public settingsSet(params: SettingsSetRequest): Promise<SettingsSetResponse> {
    return this.sendRequest("settings.set", params);
  }

  public webauthnAlreadyRegistered(
    params: WebauthnAlreadyRegisteredRequest,
  ): Promise<WebauthnAlreadyRegisteredResponse> {
    return this.sendRequest("webauthn.already_registered", params);
  }

  public webauthnStartRegistration(params: WebauthnStartRegisterRequest): Promise<WebauthnStartRegisterResponse> {
    return this.sendRequest("webauthn.reg_start", params);
  }

  public webauthnFinishRegistration(params: WebauthnFinishRegisterRequest): Promise<WebauthnFinishRegisterResponse> {
    return this.sendRequest("webauthn.reg_finish", params);
  }

  public webauthnAuthStart(params: WebauthnStartAuthRequest): Promise<WebauthnStartAuthResponse> {
    return this.sendRequest("webauthn.auth_start", params);
  }

  public webauthnListPasskeys(params: WebauthnListPasskeysRequest = {}): Promise<WebauthnListPasskeysResponse> {
    return this.sendRequest("webauthn.list_passkeys", params);
  }

  public webauthnRenamePasskey(params: WebauthnRenamePasskeyRequest): Promise<WebauthnRenamePasskeyResponse> {
    return this.sendRequest("webauthn.rename_passkey", params);
  }

  public webauthnRemovePasskey(params: WebauthnRemovePasskeyRequest): Promise<WebauthnRemovePasskeyResponse> {
    return this.sendRequest("webauthn.remove_passkey", params);
  }

  public stealthUtxosList(params: StealthUtxosListRequest): Promise<StealthUtxosListResponse> {
    return this.sendRequest("stealth_utxos.list", params);
  }

  public confidentialListOutputs(params: ConfidentialOutputsListRequest): Promise<ConfidentialOutputsListResponse> {
    return this.sendRequest("confidential.list_outputs", params);
  }

  public stealthUtxosDecryptValue(params: StealthUtxosDecryptValueRequest): Promise<StealthUtxosDecryptValueResponse> {
    return this.sendRequest("stealth_utxos.decrypt_value", params);
  }

  public stealthUtxosGetValueLookupInfo(
    params: StealthUtxosGetValueLookupInfoRequest,
  ): Promise<StealthUtxosGetValueLookupInfoResponse> {
    return this.sendRequest("stealth_utxos.get_value_lookup_info", params);
  }

  public resourcesGet(params: ResourcesGetRequest): Promise<ResourcesGetResponse> {
    return this.sendRequest("resources.get", params);
  }

  public resourcesGetMany(params: ResourcesGetManyRequest): Promise<ResourcesGetManyResponse> {
    return this.sendRequest("resources.get_many", params);
  }

  // Address book

  public addressBookAdd(params: AddressBookAddRequest): Promise<AddressBookAddResponse> {
    return this.sendRequest("address_book.add", params);
  }

  public addressBookList(): Promise<AddressBookListResponse> {
    return this.sendRequest("address_book.list", {});
  }

  public addressBookGet(params: AddressBookGetRequest): Promise<AddressBookGetResponse> {
    return this.sendRequest("address_book.get", params);
  }

  public addressBookUpdate(params: AddressBookUpdateRequest): Promise<AddressBookUpdateResponse> {
    return this.sendRequest("address_book.update", params);
  }

  public addressBookDelete(params: AddressBookDeleteRequest): Promise<AddressBookDeleteResponse> {
    return this.sendRequest("address_book.delete", params);
  }

  async sendRequest<R>(method: string, params: object = null): Promise<R> {
    const id = this.id++;
    let response = (await this.transport.sendRequest<any>(
      {
        method,
        jsonrpc: "2.0",
        id,
        params: params || {},
      },
      { token: this.token, timeout_millis: null },
    )) as RpcResponse<R>;

    if (!this.reauthEnabled) {
      if (response.error) {
        throw new Error(`RPC Error ${response.error.code}: ${response.error.message}`, {
          cause: {
            method,
            ...response.error,
          } as RpcError,
        });
      }
      return response.result;
    }

    // If we get an unauthorized error, try refreshing the token and retrying the request once
    if (response?.error && response.error.code === 401) {
      const origError = new Error(`RPC Error ${response.error.code}: ${response.error.message}`, {
        cause: {
          method,
          ...response.error,
        } as RpcError,
      });
      // Refresh failed with 401. No point in trying it again
      if (method.startsWith("auth.")) {
        console.warn("Token refresh failed");
        this.clearToken();
        throw origError;
      }
      // An API key has no refresh grant behind it, so a 401 is final. The key
      // is the caller's configured credential rather than a stale access
      // token, so it is kept for the caller to revoke or replace.
      if (this.token && isApiKey(this.token)) {
        throw origError;
      }
      const id = this.id++;
      // `auth.refresh` is authorised by the HttpOnly refresh cookie set at
      // login, never by the bearer token, so this call deliberately carries no
      // token. It only succeeds for a same-origin browser caller whose
      // transport sends cookies.
      const refreshResp = (await this.transport.sendRequest<any>({
        method: "auth.refresh",
        jsonrpc: "2.0",
        id,
        params: {},
      })) as RpcResponse<AuthLoginResponse>;

      if (refreshResp.error) {
        console.debug("Refresh resp", refreshResp);
        this.clearToken();
        // Throw the original 401 error instead of the refresh error
        throw origError;
      }

      console.debug("Token refreshed successfully. Retrying original request.");
      this.token = refreshResp.result.token;

      // Retry the original request with the new token
      response = (await this.transport.sendRequest<any>(
        {
          method,
          jsonrpc: "2.0",
          id,
          params: params || {},
        },
        { token: this.token, timeout_millis: null },
      )) as RpcResponse<R>;
    }

    if (response.error) {
      throw new Error(`RPC Error ${response.error.code}: ${response.error.message}`, {
        cause: {
          method,
          ...response.error,
        } as RpcError,
      });
    }

    return response.result;
  }
}

export type RpcError = RpcErrorResponse & {
  method: string;
};

export function isAuthError(error: any): error is RpcError {
  return error instanceof Error && (error as any).cause?.code === 401;
}
