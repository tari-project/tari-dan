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

import type {
  AccountGetRequest,
  AccountGetResponse,
  AccountSetDefaultRequest,
  AccountSetDefaultResponse,
  AccountsCreateFreeTestCoinsRequest,
  AccountsCreateFreeTestCoinsResponse,
  AccountsCreateRequest,
  AccountsCreateResponse,
  AccountsGetBalancesRequest,
  AccountsGetBalancesResponse,
  AccountsListRequest,
  AccountsListResponse,
  AuthGetAllJwtRequest,
  AuthGetAllJwtResponse,
  AuthRevokeTokenRequest,
  AuthRevokeTokenResponse,
  ClaimBurnRequest,
  ClaimBurnResponse,
  ClaimValidatorFeesRequest,
  ClaimValidatorFeesResponse,
  ConfidentialTransferRequest,
  ConfidentialTransferResponse,
  ConfidentialViewVaultBalanceRequest,
  ConfidentialViewVaultBalanceResponse,
  KeysCreateRequest,
  KeysCreateResponse,
  KeysListRequest,
  KeysListResponse,
  KeysSetActiveRequest,
  KeysSetActiveResponse,
  ListAccountNftRequest,
  ListAccountNftResponse,
  RevealFundsRequest,
  RevealFundsResponse,
  SettingsGetResponse,
  SettingsSetRequest,
  SettingsSetResponse,
  TransactionGetAllRequest,
  TransactionGetAllResponse,
  TransactionGetRequest,
  TransactionGetResponse,
  TransactionGetResultRequest,
  TransactionGetResultResponse,
  TransactionSubmitRequest,
  TransactionSubmitResponse,
  TransactionWaitResultRequest,
  TransactionWaitResultResponse,
  WebRtcStartRequest,
  WebRtcStartResponse,
  AccountsTransferRequest,
  AccountsTransferResponse,
  SubstatesGetRequest,
  SubstatesGetResponse,
  TemplatesGetResponse,
  SubstatesListRequest,
  SubstatesListResponse,
} from "@tariproject/typescript-bindings/wallet-daemon-client";
import { AccountGetDefaultRequest, TemplatesGetRequest, WalletDaemonClient } from "@tariproject/wallet_daemon_client";

let clientInstance: WalletDaemonClient | null = null;
let pendingClientInstance: Promise<WalletDaemonClient> | null = null;
let outerAddress: URL | null = null;
const DEFAULT_WALLET_ADDRESS = new URL(import.meta.env.VITE_DAEMON_JRPC_ADDRESS || "http://localhost:9000");

export async function getClientAddress(): Promise<URL> {
  try {
    let resp = await fetch("/json_rpc_address");
    if (resp.status === 200) {
      return new URL(await resp.text());
    }
  } catch (e) {
    console.warn(e);
  }

  return DEFAULT_WALLET_ADDRESS;
}

async function client() {
  if (pendingClientInstance) {
    return pendingClientInstance;
  }
  if (clientInstance) {
    if (!clientInstance.isAuthenticated()) {
      return authenticateClient(clientInstance).then(() => clientInstance!);
    }
    return Promise.resolve(clientInstance);
  }

  const getAddress = !outerAddress ? getClientAddress() : Promise.resolve(DEFAULT_WALLET_ADDRESS);

  pendingClientInstance = getAddress.then(async (addr) => {
    const client = WalletDaemonClient.usingFetchTransport(addr.toString());
    await authenticateClient(client);
    outerAddress = addr;
    clientInstance = client;
    pendingClientInstance = null;
    return client;
  });
  return pendingClientInstance;
}

async function authenticateClient(client: WalletDaemonClient) {
  const auth_token = await client.authRequest(["Admin"]);
  await client.authAccept(auth_token, auth_token);
}

export const authRevoke = (request: AuthRevokeTokenRequest): Promise<AuthRevokeTokenResponse> =>
  client().then((c) => c.authRevoke(request));
export const authGetAllJwt = (request: AuthGetAllJwtRequest): Promise<AuthGetAllJwtResponse> =>
  client().then((c) => c.authGetAllJwt(request));

// settings
export const settingsGet = (): Promise<SettingsGetResponse> => client().then((c) => c.settingsGet());
export const settingsSet = (request: SettingsSetRequest): Promise<SettingsSetResponse> =>
  client().then((c) => c.settingsSet(request));

// webrtc
export const webrtcStart = (request: WebRtcStartRequest): Promise<WebRtcStartResponse> =>
  client().then((c) => c.webrtcStart(request));

// rpc
export const rpcDiscover = (): Promise<string> => client().then((c) => c.rpcDiscover());

// keys
export const keysCreate = (request: KeysCreateRequest): Promise<KeysCreateResponse> =>
  client().then((c) => c.createKey(request));
export const keysList = (request: KeysListRequest): Promise<KeysListResponse> =>
  client().then((c) => c.listKeys(request));
export const keysSetActive = (request: KeysSetActiveRequest): Promise<KeysSetActiveResponse> =>
  client().then((c) => c.keysSetActive(request));

export const transactionsSubmit = (request: TransactionSubmitRequest): Promise<TransactionSubmitResponse> =>
  client().then((c) => c.submitTransaction(request));
export const transactionsGet = (request: TransactionGetRequest): Promise<TransactionGetResponse> =>
  client().then((c) => c.transactionsGet(request));
export const transactionsGetResult = (request: TransactionGetResultRequest): Promise<TransactionGetResultResponse> =>
  client().then((c) => c.getTransactionResult(request));
export const transactionsWaitResult = (request: TransactionWaitResultRequest): Promise<TransactionWaitResultResponse> =>
  client().then((c) => c.waitForTransactionResult(request));
export const transactionsGetAll = (request: TransactionGetAllRequest): Promise<TransactionGetAllResponse> =>
  client().then((c) => c.transactionsList(request));

// accounts

export const accountsRevealFunds = (request: RevealFundsRequest): Promise<RevealFundsResponse> =>
  client().then((c) => c.accountsRevealFunds(request));
export const accountsClaimBurn = (request: ClaimBurnRequest): Promise<ClaimBurnResponse> =>
  client().then((c) => c.accountsClaimBurn(request));
export const accountsCreate = (request: AccountsCreateRequest): Promise<AccountsCreateResponse> =>
  client().then((c) => c.accountsCreate(request));
export const accountsList = (request: AccountsListRequest): Promise<AccountsListResponse> =>
  client().then((c) => c.accountsList(request));
export const accountsGetBalances = (request: AccountsGetBalancesRequest): Promise<AccountsGetBalancesResponse> =>
  client().then((c) => c.accountsGetBalances(request));
export const accountsGet = (request: AccountGetRequest): Promise<AccountGetResponse> =>
  client().then((c) => c.accountsGet(request));
export const accountsTransfer = (request: AccountsTransferRequest): Promise<AccountsTransferResponse> =>
  client().then((c) => c.accountsTransfer(request));
export const accountsConfidentialTransfer = (
  request: ConfidentialTransferRequest,
): Promise<ConfidentialTransferResponse> => client().then((c) => c.confidentialTransfer(request));
export const accountsSetDefault = (request: AccountSetDefaultRequest): Promise<AccountSetDefaultResponse> =>
  client().then((c) => c.accountsSetDefault(request));
export const accountsCreateFreeTestCoins = (
  request: AccountsCreateFreeTestCoinsRequest,
): Promise<AccountsCreateFreeTestCoinsResponse> => client().then((c) => c.createFreeTestCoins(request));
export const accountsGetDefault = (request: AccountGetDefaultRequest): Promise<AccountGetResponse> =>
  client().then((c) => c.accountsGetDefault(request));

// confidential
export const confidentialViewVaultBalance = (
  request: ConfidentialViewVaultBalanceRequest,
): Promise<ConfidentialViewVaultBalanceResponse> => client().then((c) => c.viewVaultBalance(request));

// nfts
export const nftList = (request: ListAccountNftRequest): Promise<ListAccountNftResponse> =>
  client().then((c) => c.nftsList(request));

// validators

export const validatorsClaimFees = (request: ClaimValidatorFeesRequest): Promise<ClaimValidatorFeesResponse> =>
  client().then((c) => c.validatorsClaimFees(request));

// substates
export const substatesGet = (request: SubstatesGetRequest): Promise<SubstatesGetResponse> =>
  client().then((c) => c.substatesGet(request));

export const substatesList = (request: SubstatesListRequest): Promise<SubstatesListResponse> =>
  client().then((c) => c.substatesList(request));

// templates
export const templatesGet = (request: TemplatesGetRequest): Promise<TemplatesGetResponse> =>
  client().then((c) => c.templatesGet(request));
