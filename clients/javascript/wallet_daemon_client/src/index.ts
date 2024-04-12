/*
 * //  Copyright 2024 The Tari Project
 * //  SPDX-License-Identifier: BSD-3-Clause
 */

import {
  AccountGetDefaultRequest,
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
  AccountsTransferRequest,
  AccountsTransferResponse,
  AuthGetAllJwtRequest,
  AuthGetAllJwtResponse,
  AuthRevokeTokenRequest,
  AuthRevokeTokenResponse,
  ClaimBurnRequest,
  ClaimBurnResponse,
  ClaimValidatorFeesRequest,
  ClaimValidatorFeesResponse,
  ComponentAddressOrName,
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
  SubstatesGetRequest,
  SubstatesGetResponse,
  SubstatesListRequest,
  SubstatesListResponse,
  TemplatesGetRequest,
  TemplatesGetResponse,
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
} from "@tariproject/typescript-bindings/wallet-daemon-client";

import {
  Arg,
  FinalizeResult,
  TemplateDef,
  FunctionDef,
  Type,
  ArgDef,
  Instruction,
  SubstateType,
  TransactionStatus,
  SubstateId,
  substateIdToString,
  stringToSubstateId,
  rejectReasonToString,
} from "@tariproject/typescript-bindings";
import { FetchRpcTransport, RpcTransport } from "./transports";

export * as transports from "./transports";

export { substateIdToString, stringToSubstateId, rejectReasonToString };

export type {
  AccountGetDefaultRequest,
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
  AccountsTransferRequest,
  AccountsTransferResponse,
  AuthGetAllJwtRequest,
  AuthGetAllJwtResponse,
  AuthRevokeTokenRequest,
  AuthRevokeTokenResponse,
  ClaimBurnRequest,
  ClaimBurnResponse,
  ClaimValidatorFeesRequest,
  ClaimValidatorFeesResponse,
  ComponentAddressOrName,
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
  SubstatesGetRequest,
  SubstatesGetResponse,
  SubstatesListRequest,
  SubstatesListResponse,
  TemplatesGetRequest,
  TemplatesGetResponse,
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
  Arg,
  FinalizeResult,
  TemplateDef,
  FunctionDef,
  Type,
  ArgDef,
  Instruction,
  SubstateType,
  TransactionStatus,
  SubstateId,
};

export class WalletDaemonClient {
  private token: string | null;
  private transport: RpcTransport;
  private id: number;

  constructor(transport: RpcTransport) {
    this.token = null;
    this.transport = transport;
    this.id = 0;
  }

  public static new(transport: RpcTransport): WalletDaemonClient {
    return new WalletDaemonClient(transport);
  }

  public static usingFetchTransport(url: string): WalletDaemonClient {
    return WalletDaemonClient.new(FetchRpcTransport.new(url));
  }

  getTransport() {
    return this.transport;
  }

  public isAuthenticated() {
    return this.token !== null;
  }

  public setToken(token: string) {
    this.token = token;
  }

  public authGetAllJwt(params: AuthGetAllJwtRequest): Promise<AuthGetAllJwtResponse> {
    return this.__invokeRpc("auth.get_all_jwt", params);
  }

  public async authRequest(permissions: string[]): Promise<string> {
    // TODO: Exchange some secret credentials for a JWT
    let resp = await this.__invokeRpc("auth.request", { permissions });
    return resp.auth_token;
  }

  public async authAccept(adminToken: string, name: string): Promise<string> {
    let resp = await this.__invokeRpc("auth.accept", { auth_token: adminToken, name });
    this.token = resp.permissions_token;
    return this.token;
  }

  public authRevoke(params: AuthRevokeTokenRequest): Promise<AuthRevokeTokenResponse> {
    return this.__invokeRpc("auth.revoke", params);
  }

  public accountsCreate(params: AccountsCreateRequest): Promise<AccountsCreateResponse> {
    return this.__invokeRpc("accounts.create", params);
  }

  public accountsClaimBurn(params: ClaimBurnRequest): Promise<ClaimBurnResponse> {
    return this.__invokeRpc("accounts.claim_burn", params);
  }

  public accountsRevealFunds(params: RevealFundsRequest): Promise<RevealFundsResponse> {
    return this.__invokeRpc("accounts.reveal_funds", params);
  }

  public accountsGetBalances(params: AccountsGetBalancesRequest): Promise<AccountsGetBalancesResponse> {
    return this.__invokeRpc("accounts.get_balances", params);
  }

  public accountsList(params: AccountsListRequest): Promise<AccountsListResponse> {
    return this.__invokeRpc("accounts.list", params);
  }

  public accountsGet(params: AccountGetRequest): Promise<AccountGetResponse> {
    return this.__invokeRpc("accounts.get", params);
  }

  public accountsTransfer(params: AccountsTransferRequest): Promise<AccountsTransferResponse> {
    return this.__invokeRpc("accounts.transfer", params);
  }

  public confidentialTransfer(params: ConfidentialTransferRequest): Promise<ConfidentialTransferResponse> {
    return this.__invokeRpc("accounts.confidential_transfer", params);
  }

  public accountsGetDefault(params: AccountGetDefaultRequest): Promise<AccountGetResponse> {
    return this.__invokeRpc("accounts.get_default", params);
  }

  public accountsSetDefault(params: AccountSetDefaultRequest): Promise<AccountSetDefaultResponse> {
    return this.__invokeRpc("accounts.set_default", params);
  }


  public submitTransaction(params: TransactionSubmitRequest): Promise<TransactionSubmitResponse> {
    return this.__invokeRpc("transactions.submit", params);
  }

  public substatesGet(params: SubstatesGetRequest): Promise<SubstatesGetResponse> {
    return this.__invokeRpc("substates.get", params);
  }

  public substatesList(params: SubstatesListRequest): Promise<SubstatesListResponse> {
    return this.__invokeRpc("substates.list", params);
  }

  public transactionsList(params: TransactionGetAllRequest): Promise<TransactionGetAllResponse> {
    return this.__invokeRpc("transactions.get_all", params);
  }

  public transactionsGet(params: TransactionGetRequest): Promise<TransactionGetResponse> {
    return this.__invokeRpc("transactions.get", params);
  }

  public getTransactionResult(params: TransactionGetResultRequest): Promise<TransactionWaitResultResponse> {
    return this.__invokeRpc("transactions.get_result", params);
  }

  public waitForTransactionResult(params: TransactionWaitResultRequest): Promise<TransactionWaitResultResponse> {
    return this.__invokeRpc("transactions.wait_result", params);
  }

  public templatesGet(params: TemplatesGetRequest): Promise<TemplatesGetResponse> {
    return this.__invokeRpc("templates.get", params);
  }

  public createFreeTestCoins(params: AccountsCreateFreeTestCoinsRequest): Promise<AccountsCreateFreeTestCoinsResponse> {
    return this.__invokeRpc("accounts.create_free_test_coins", params);
  }

  public createKey(params: KeysCreateRequest): Promise<KeysCreateResponse> {
    return this.__invokeRpc("keys.create", params);
  }

  public keysSetActive(params: KeysSetActiveRequest): Promise<KeysSetActiveResponse> {
    return this.__invokeRpc("keys.set_active", params);
  }

  public listKeys(params: KeysListRequest): Promise<KeysListResponse> {
    return this.__invokeRpc("keys.list", params);
  }

  public viewVaultBalance(params: ConfidentialViewVaultBalanceRequest): Promise<ConfidentialViewVaultBalanceResponse> {
    return this.__invokeRpc("confidential.view_vault_balance", params);
  }

  public nftsList(params: ListAccountNftRequest): Promise<ListAccountNftResponse> {
    return this.__invokeRpc("nfts.list", params);
  }

  public validatorsClaimFees(params: ClaimValidatorFeesRequest): Promise<ClaimValidatorFeesResponse> {
    return this.__invokeRpc("validators.claim_fees", params);
  }

  public rpcDiscover(): Promise<string> {
    return this.__invokeRpc("rpc.discover", {});
  }

  public webrtcStart(params: WebRtcStartRequest): Promise<WebRtcStartResponse> {
    return this.__invokeRpc("webrtc.start", params);
  }

  public settingsGet(): Promise<SettingsGetResponse> {
    return this.__invokeRpc("settings.get");
  }

  public settingsSet(params: SettingsSetRequest): Promise<SettingsSetResponse> {
    return this.__invokeRpc("settings.set", params);
  }

  async __invokeRpc(method: string, params: object = null) {
    const id = this.id++;
    const response = await this.transport.sendRequest<any>(
      {
        method,
        jsonrpc: "2.0",
        id: id,
        params: params || {},
      },
      { token: this.token, timeout_millis: null },
    );

    return response;
  }
}
