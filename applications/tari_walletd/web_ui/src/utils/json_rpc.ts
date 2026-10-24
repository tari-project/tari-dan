//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

import { IndexerClient } from "@tari-project/indexer-client";
import type {
  AccountGetDefaultRequest,
  AccountGetRequest,
  AccountGetResponse,
  AccountsAssociateStealthResourceRequest,
  AccountsAssociateStealthResourceResponse,
  AccountsCreateFreeTestCoinsRequest,
  AccountsCreateFreeTestCoinsResponse,
  AccountsCreateRequest,
  AccountsCreateResponse,
  AccountSetDefaultRequest,
  AccountSetDefaultResponse,
  AccountsGetBalanceChangesRequest,
  AccountsGetBalanceChangesResponse,
  AccountsGetBalancesRequest,
  AccountsGetBalancesResponse,
  AccountsGetFaucetBalanceResponse,
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
  AuthGetMethodResponse,
  AuthListApiKeysRequest,
  AuthListApiKeysResponse,
  AuthListSessionsRequest,
  AuthListSessionsResponse,
  AuthRevokeApiKeyRequest,
  AuthRevokeApiKeyResponse,
  AuthRevokeTokenRequest,
  AuthRevokeTokenResponse,
  BurnProofsGetRequest,
  BurnProofsGetResponse,
  BurnProofsListRequest,
  BurnProofsListResponse,
  ClaimBurnRequest,
  ClaimBurnResponse,
  ClaimValidatorFeesRequest,
  ClaimValidatorFeesResponse,
  ConfidentialOutputsListRequest,
  ConfidentialOutputsListResponse,
  ConfidentialTransferRequest,
  ConfidentialTransferResponse,
  ConfidentialViewVaultBalanceRequest,
  ConfidentialViewVaultBalanceResponse,
  GetValidatorFeesRequest,
  GetValidatorFeesResponse,
  KeysCreateRequest,
  KeysCreateResponse,
  KeysListRequest,
  KeysListResponse,
  KeysSetActiveRequest,
  KeysSetActiveResponse,
  ListNftsRequest,
  ListNftsResponse,
  MintFaucetNftRequest,
  MintFaucetNftResponse,
  PublishTemplateRequest,
  PublishTemplateResponse,
  SettingsGetResponse,
  SettingsSetRequest,
  SettingsSetResponse,
  SigningRequestDecisionRequest,
  SigningRequestDecisionResponse,
  SigningRequestListRequest,
  SigningRequestListResponse,
  StealthTransferRequest,
  StealthTransferResponse,
  StealthUtxosDecryptValueRequest,
  StealthUtxosDecryptValueResponse,
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
  TransactionGetAllRequest,
  TransactionGetAllResponse,
  TransactionGetRequest,
  TransactionGetResponse,
  TransactionGetResultRequest,
  TransactionGetResultResponse,
  TransactionRequestDecisionRequest,
  TransactionRequestDecisionResponse,
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
  ValidatorSetGetStatusResponse,
  WalletGetInfoResponse,
  WebauthnAlreadyRegisteredResponse,
  WebauthnFinishRegisterRequest,
  WebauthnFinishRegisterResponse,
  WebauthnListPasskeysRequest,
  WebauthnListPasskeysResponse,
  WebauthnRemovePasskeyRequest,
  WebauthnRemovePasskeyResponse,
  WebauthnRenamePasskeyRequest,
  WebauthnRenamePasskeyResponse,
  WebauthnStartAuthResponse,
  WebauthnStartRegisterRequest,
  WebauthnStartRegisterResponse,
  WebRtcStartRequest,
  WebRtcStartResponse,
} from "@tari-project/ootle-ts-bindings";
import { WalletDaemonClient } from "@tari-project/wallet_jrpc_client";
import { jwtDecode } from "jwt-decode";

let clientInstance: WalletDaemonClient | null = null;
const WALLET_JRPC_URL =
  import.meta.env.VITE_DAEMON_JRPC_ADDRESS ||
  import.meta.env.VITE_JSON_RPC_ADDRESS ||
  import.meta.env.VITE_JRPC_ADDRESS ||
  "/json_rpc";

export const isValidJwt = (token?: string | null) => {
  if (!token) return false;
  try {
    const decodedToken = jwtDecode(token);
    if (!decodedToken.exp) {
      return true;
    }
    const currentTime = Date.now() / 1000;
    return decodedToken.exp >= currentTime;
  } catch (error) {
    console.warn("Failed to decode token:", error);
    return false;
  }
};

export async function getClientInstance(): Promise<WalletDaemonClient> {
  if (clientInstance) {
    return clientInstance;
  }

  // This returning a promise was because of the need to first fetch the JSON-RPC address. Not needed now, but we'll
  // keep it this way to avoid having to change a lot of code and, it is more future-proof.
  const client = WalletDaemonClient.usingFetchTransport(WALLET_JRPC_URL);
  clientInstance = client;
  return client;
}

// Alias
const client = getClientInstance;

export const authGetMethod = (): Promise<AuthGetMethodResponse> => client().then((c) => c.authGetMethod());

export const webauthnAlreadyRegistered = (): Promise<WebauthnAlreadyRegisteredResponse> =>
  client().then((c) => c.webauthnAlreadyRegistered({}));

export const webauthnStartRegistration = (
  request: WebauthnStartRegisterRequest,
): Promise<WebauthnStartRegisterResponse> => client().then((c) => c.webauthnStartRegistration(request));

export const webauthnFinishRegistration = (
  request: WebauthnFinishRegisterRequest,
): Promise<WebauthnFinishRegisterResponse> => client().then((c) => c.webauthnFinishRegistration(request));

export const webauthnStartAuth = (): Promise<WebauthnStartAuthResponse> =>
  client().then((c) => c.webauthnAuthStart({}));

export const webauthnListPasskeys = (
  request: WebauthnListPasskeysRequest = {},
): Promise<WebauthnListPasskeysResponse> => client().then((c) => c.webauthnListPasskeys(request));

export const webauthnRenamePasskey = (
  request: WebauthnRenamePasskeyRequest,
): Promise<WebauthnRenamePasskeyResponse> => client().then((c) => c.webauthnRenamePasskey(request));

export const webauthnRemovePasskey = (
  request: WebauthnRemovePasskeyRequest,
): Promise<WebauthnRemovePasskeyResponse> => client().then((c) => c.webauthnRemovePasskey(request));

export const authRevoke = (request: AuthRevokeTokenRequest): Promise<AuthRevokeTokenResponse> =>
  client().then((c) => c.authRevoke(request));
export const authGetAllJwt = (request: AuthListSessionsRequest): Promise<AuthListSessionsResponse> =>
  client().then((c) => c.authListSessions(request));

// API keys (issue #1957). All three require an Admin session; the daemon
// rejects non-admin calls with InsufficientPermissions.
export const authCreateApiKey = (request: AuthCreateApiKeyRequest): Promise<AuthCreateApiKeyResponse> =>
  client().then((c) => c.authCreateApiKey(request));
export const authListApiKeys = (request: AuthListApiKeysRequest): Promise<AuthListApiKeysResponse> =>
  client().then((c) => c.authListApiKeys(request));
export const authRevokeApiKey = (request: AuthRevokeApiKeyRequest): Promise<AuthRevokeApiKeyResponse> =>
  client().then((c) => c.authRevokeApiKey(request));

// Transaction requests (issue #2343). `create` and `approve` are separately
// permissioned, and `approve` takes an interactive user session: a requesting
// tool holding an API key cannot approve its own request.
export const transactionRequestsList = (
  request: TransactionRequestListRequest,
): Promise<TransactionRequestListResponse> => client().then((c) => c.transactionRequestsList(request));
export const transactionRequestsApprove = (
  request: TransactionRequestDecisionRequest,
): Promise<TransactionRequestDecisionResponse> => client().then((c) => c.transactionRequestsApprove(request));
export const transactionRequestsReject = (
  request: TransactionRequestDecisionRequest,
): Promise<TransactionRequestDecisionResponse> => client().then((c) => c.transactionRequestsReject(request));
export const transactionRequestsSubmit = (
  request: TransactionRequestSubmitRequest,
): Promise<TransactionRequestSubmitResponse> => client().then((c) => c.transactionRequestsSubmit(request));

// Signing requests. A tool holding `signing_requests:create` asks the wallet to
// co-sign; approving or rejecting takes an interactive session, which this UI is.
export const signingRequestsList = (request: SigningRequestListRequest): Promise<SigningRequestListResponse> =>
  client().then((c) => c.signingRequestsList(request));
export const signingRequestsApprove = (
  request: SigningRequestDecisionRequest,
): Promise<SigningRequestDecisionResponse> => client().then((c) => c.signingRequestsApprove(request));
export const signingRequestsReject = (
  request: SigningRequestDecisionRequest,
): Promise<SigningRequestDecisionResponse> => client().then((c) => c.signingRequestsReject(request));

// indexer
export const indexerGetNetworkInfo = (indexerUrl: string) =>
  IndexerClient.usingFetchTransport(indexerUrl).networkInfo();

// settings
export const settingsGet = (): Promise<SettingsGetResponse> => client().then((c) => c.settingsGet());
export const settingsSet = (request: SettingsSetRequest): Promise<SettingsSetResponse> =>
  client().then((c) => c.settingsSet(request));

// webrtc
export const webrtcStart = (request: WebRtcStartRequest): Promise<WebRtcStartResponse> =>
  client().then((c) => c.webrtcStart(request));

// keys
export const keysCreate = (request: KeysCreateRequest): Promise<KeysCreateResponse> =>
  client().then((c) => c.createKey(request));
export const keysList = (request: KeysListRequest): Promise<KeysListResponse> =>
  client().then((c) => c.listKeys(request));
export const keysSetActive = (request: KeysSetActiveRequest): Promise<KeysSetActiveResponse> =>
  client().then((c) => c.keysSetActive(request));

export const transactionsSubmit = (request: TransactionSubmitRequest): Promise<TransactionSubmitResponse> =>
  client().then((c) => c.submitTransaction(request));
export const submitTransactionDryRun = (request: TransactionSubmitRequest): Promise<TransactionSubmitDryRunResponse> =>
  client().then((c) => c.submitTransactionDryRun(request));
export const transactionsGet = (request: TransactionGetRequest): Promise<TransactionGetResponse> =>
  client().then((c) => c.transactionsGet(request));
export const transactionsGetResult = (request: TransactionGetResultRequest): Promise<TransactionGetResultResponse> =>
  client().then((c) => c.getTransactionResult(request));
export const transactionsWaitResult = (request: TransactionWaitResultRequest): Promise<TransactionWaitResultResponse> =>
  client().then((c) => c.waitForTransactionResult(request));
export const transactionsGetAll = (request: TransactionGetAllRequest): Promise<TransactionGetAllResponse> =>
  client().then((c) => c.transactionsList(request));

export const transactionsPublishTemplate = (request: PublishTemplateRequest): Promise<PublishTemplateResponse> =>
  client().then((c) => c.publishTemplate(request));

export const transactionsSubmitManifest = (
  request: TransactionSubmitManifestRequest,
): Promise<TransactionSubmitManifestResponse> => client().then((c) => c.submitTransactionManifest(request));

// burn proofs
export const burnProofsList = (request: BurnProofsListRequest): Promise<BurnProofsListResponse> =>
  client().then((c) => c.burnProofsList(request));
export const burnProofsGet = (request: BurnProofsGetRequest): Promise<BurnProofsGetResponse> =>
  client().then((c) => c.burnProofsGet(request));

// accounts
export const accountsClaimBurn = (request: ClaimBurnRequest): Promise<ClaimBurnResponse> =>
  client().then((c) => c.accountsClaimBurn(request));
export const accountsCreate = (request: AccountsCreateRequest): Promise<AccountsCreateResponse> =>
  client().then((c) => c.accountsCreate(request));
export const accountsRename = (request: AccountsRenameRequest): Promise<AccountsRenameResponse> =>
  client().then((c) => c.accountsRename(request));
export const accountsList = (request: AccountsListRequest): Promise<AccountsListResponse> =>
  client().then((c) => c.accountsList(request));
export const accountsGetBalances = (request: AccountsGetBalancesRequest): Promise<AccountsGetBalancesResponse> =>
  client().then((c) => c.accountsGetBalances(request));
export const accountsGetBalanceChanges = (
  request: AccountsGetBalanceChangesRequest,
): Promise<AccountsGetBalanceChangesResponse> => client().then((c) => c.accountsGetBalanceChanges(request));
export const accountsGet = (request: AccountGetRequest): Promise<AccountGetResponse> =>
  client().then((c) => c.accountsGet(request));

export const accountsTransfer = (request: AccountsTransferRequest): Promise<AccountsTransferResponse> =>
  client().then((c) => c.accountsTransfer(request));
export const accountsConfidentialTransfer = (
  request: ConfidentialTransferRequest,
): Promise<ConfidentialTransferResponse> => client().then((c) => c.confidentialTransfer(request));
export const accountsAssociateStealthResource = (
  request: AccountsAssociateStealthResourceRequest,
): Promise<AccountsAssociateStealthResourceResponse> =>
  client().then((c) => c.accountsAssociateStealthResource(request));
export const accountsStealthTransfer = (request: StealthTransferRequest): Promise<StealthTransferResponse> =>
  client().then((c) => c.stealthTransfer(request));
export const accountsSetDefault = (request: AccountSetDefaultRequest): Promise<AccountSetDefaultResponse> =>
  client().then((c) => c.accountsSetDefault(request));
export const accountsCreateFreeTestCoins = (
  request: AccountsCreateFreeTestCoinsRequest,
): Promise<AccountsCreateFreeTestCoinsResponse> => client().then((c) => c.createFreeTestCoins(request));
export const accountsGetFaucetBalance = (): Promise<AccountsGetFaucetBalanceResponse> =>
  client().then((c) => c.getFaucetBalance({}));
export const mintFaucetNfts = (request: MintFaucetNftRequest): Promise<MintFaucetNftResponse> =>
  client().then((c) => c.mintFaucetNfts(request));
export const accountsGetDefault = (request: AccountGetDefaultRequest): Promise<AccountGetResponse> =>
  client().then((c) => c.accountsGetDefault(request));

// confidential
export const confidentialViewVaultBalance = (
  request: ConfidentialViewVaultBalanceRequest,
): Promise<ConfidentialViewVaultBalanceResponse> => client().then((c) => c.viewVaultBalance(request));

export const stealthDecryptUtxoBalance = (
  request: StealthUtxosDecryptValueRequest,
): Promise<StealthUtxosDecryptValueResponse> => client().then((c) => c.stealthUtxosDecryptValue(request));

// nfts
export const nftList = (request: ListNftsRequest): Promise<ListNftsResponse> =>
  client().then((c) => c.nftsList(request));

export const nftTransfer = (request: TransferNftRequest): Promise<TransferNftResponse> =>
  client().then((c) => c.nftTransfer(request));

// validators

export const validatorsClaimFees = (request: ClaimValidatorFeesRequest): Promise<ClaimValidatorFeesResponse> =>
  client().then((c) => c.validatorsClaimFees(request));
export const validatorsGetFees = (request: GetValidatorFeesRequest): Promise<GetValidatorFeesResponse> =>
  client().then((c) => c.validatorsGetFees(request));

// substates
export const substatesGet = (request: SubstatesGetRequest): Promise<SubstatesGetResponse> =>
  client().then((c) => c.substatesGet(request));

export const substatesList = (request: SubstatesListRequest): Promise<SubstatesListResponse> =>
  client().then((c) => c.substatesList(request));

// templates
export const templatesGet = (request: TemplatesGetRequest): Promise<TemplatesGetResponse> =>
  client().then((c) => c.templatesGet(request));

export const templatesListAuthored = (request: TemplatesListAuthoredRequest): Promise<TemplatesListAuthoredResponse> =>
  client().then((c) => c.templatesListAuthored(request));

// info
export const walletGetInfo = (): Promise<WalletGetInfoResponse> => client().then((c) => c.walletGetInfo());

export const validatorSetGetStatus = (): Promise<ValidatorSetGetStatusResponse> =>
  client().then((c) => c.validatorSetGetStatus());

// utxos
export const stealthUtxosList = (request: StealthUtxosListRequest): Promise<StealthUtxosListResponse> =>
  client().then((c) => c.stealthUtxosList(request));

export const confidentialListOutputs = (
  request: ConfidentialOutputsListRequest,
): Promise<ConfidentialOutputsListResponse> => client().then((c) => c.confidentialListOutputs(request));

// swap pools
export const swapPoolGetExchangeRate = (
  request: SwapPoolGetExchangeRateRequest,
): Promise<SwapPoolGetExchangeRateResponse> => client().then((c) => c.swapPoolGetExchangeRate(request));

export const swapPoolsList = (request: SwapPoolsListRequest): Promise<SwapPoolsListResponse> =>
  client().then((c) => c.swapPoolsList(request));

// address book
export const addressBookAdd = (request: AddressBookAddRequest): Promise<AddressBookAddResponse> =>
  client().then((c) => c.addressBookAdd(request));

export const addressBookList = (): Promise<AddressBookListResponse> => client().then((c) => c.addressBookList());

export const addressBookGet = (request: AddressBookGetRequest): Promise<AddressBookGetResponse> =>
  client().then((c) => c.addressBookGet(request));

export const addressBookUpdate = (request: AddressBookUpdateRequest): Promise<AddressBookUpdateResponse> =>
  client().then((c) => c.addressBookUpdate(request));

export const addressBookDelete = (request: AddressBookDeleteRequest): Promise<AddressBookDeleteResponse> =>
  client().then((c) => c.addressBookDelete(request));
