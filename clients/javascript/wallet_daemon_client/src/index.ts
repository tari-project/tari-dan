/*
 * //  Copyright 2024 The Tari Project
 * //  SPDX-License-Identifier: BSD-3-Clause
 */

import {
  AccountGetDefaultRequest,
  AccountGetRequest,
  AccountGetResponse,
  AccountsGetBalancesRequest,
  AccountsGetBalancesResponse,
  TransactionGetResultRequest,
  TransactionGetResultResponse,
  TemplatesGetResponse,
  TemplatesGetRequest,
  SubstatesGetResponse,
  SubstatesGetRequest,
  SubstatesListRequest,
  SubstatesListResponse,
  TransactionSubmitRequest,
  TransactionSubmitResponse,
  TransactionWaitResultRequest,
  TransactionWaitResultResponse,
  AccountsCreateFreeTestCoinsRequest,
  AccountsCreateFreeTestCoinsResponse,
  ComponentAddressOrName,
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
} from "@tariproject/typescript-bindings/index";
import { FetchRpcTransport, RpcTransport } from "./transports";

export * as transports from "./transports";

export { substateIdToString, stringToSubstateId, rejectReasonToString };

export type {
  ComponentAddressOrName,
  AccountsCreateFreeTestCoinsRequest,
  AccountsCreateFreeTestCoinsResponse,
  AccountGetDefaultRequest,
  AccountGetRequest,
  AccountGetResponse,
  AccountsGetBalancesRequest,
  AccountsGetBalancesResponse,
  TemplateDef,
  FunctionDef,
  Type,
  ArgDef,
  TransactionGetResultRequest,
  TransactionGetResultResponse,
  TemplatesGetResponse,
  TemplatesGetRequest,
  TransactionStatus,
  SubstatesGetResponse,
  SubstatesGetRequest,
  SubstateId,
  FinalizeResult,
  Arg,
  SubstateType,
  Instruction,
  SubstatesListRequest,
  SubstatesListResponse,
  TransactionSubmitRequest,
  TransactionSubmitResponse,
  TransactionWaitResultRequest,
  TransactionWaitResultResponse,
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

  static new(transport: RpcTransport): WalletDaemonClient {
    return new WalletDaemonClient(transport);
  }

  static usingFetchTransport(url: string): WalletDaemonClient {
    return WalletDaemonClient.new(FetchRpcTransport.new(url));
  }

  getTransport() {
    return this.transport;
  }

  setToken(token: string) {
    this.token = token;
  }

  async authRequest(permissions: string[]): Promise<string> {
    // TODO: Exchange some secret credentials for a JWT
    let resp = await this.__invokeRpc("auth.request", { permissions });
    return resp.auth_token;
  }

  async authAccept(adminToken: string, name: string): Promise<string> {
    let resp = await this.__invokeRpc("auth.accept", { auth_token: adminToken, name });
    this.token = resp.permissions_token;
    return this.token;
  }

  accountsGetBalances(params: AccountsGetBalancesRequest): Promise<AccountsGetBalancesResponse> {
    return this.__invokeRpc("accounts.get_balances", params);
  }

  accountsGet(params: AccountGetRequest): Promise<AccountGetResponse> {
    return this.__invokeRpc("accounts.get", params);
  }

  accountsGetDefault(params: AccountGetDefaultRequest): Promise<AccountGetResponse> {
    return this.__invokeRpc("accounts.get_default", params);
  }

  submitTransaction(params: TransactionSubmitRequest): Promise<TransactionSubmitResponse> {
    return this.__invokeRpc("transactions.submit", params);
  }

  substatesGet(params: SubstatesGetRequest): Promise<SubstatesGetResponse> {
    return this.__invokeRpc("substates.get", params);
  }

  substatesList(params: SubstatesListRequest): Promise<SubstatesListResponse> {
    return this.__invokeRpc("substates.list", params);
  }

  getTransactionResult(params: TransactionGetResultRequest): Promise<TransactionWaitResultResponse> {
    return this.__invokeRpc("transactions.get_result", params);
  }

  waitForTransactionResult(params: TransactionWaitResultRequest): Promise<TransactionWaitResultResponse> {
    return this.__invokeRpc("transactions.wait_result", params);
  }

  templatesGet(params: TemplatesGetRequest): Promise<TemplatesGetResponse> {
    return this.__invokeRpc("templates.get", params);
  }

  createFreeTestCoins(params: AccountsCreateFreeTestCoinsRequest): Promise<AccountsCreateFreeTestCoinsResponse> {
    return this.__invokeRpc("accounts.create_free_test_coins", params);
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
