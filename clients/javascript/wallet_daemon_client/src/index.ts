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
    SubstatesGetRequest,
    SubstatesGetResponse,
    SubstatesListRequest,
    SubstatesListResponse,
    TransactionSubmitRequest,
    TransactionSubmitResponse,
    TransactionGetResultRequest,
    TransactionGetResultResponse,
    TransactionWaitResultRequest,
    TransactionWaitResultResponse, TemplatesGetRequest, TemplatesGetResponse,
} from '@tarilabs/typescript-bindings/wallet-daemon-client';

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
} from '@tarilabs/typescript-bindings/index';

export type {
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
    TransactionWaitResultResponse
};

export function createClient(url: string): WalletDaemonClient {
    return new WalletDaemonClient(url);
}

export class WalletDaemonClient {
    private token: string | null;
    private url: string;
    private id: number;

    constructor(url: string) {
        this.token = null;
        this.url = url;
        this.id = 0;
    }

    setToken(token: string) {
        this.token = token;
    }

    async authRequest(permissions: string[]): Promise<string> {
        // TODO: Exchange some secret credentials for a JWT
        let resp = await this.__invokeRpc("auth.request", {permissions});
        return resp.auth_token;
    }

    async authAccept(adminToken: string, name: string): Promise<string> {
        let resp = await this.__invokeRpc("auth.accept", {auth_token: adminToken, name});
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

    async __invokeRpc(method: string, params: object = null) {
        const id = this.id++;
        const headers = {
            "Content-Type": "application/json",
        };
        if (this.token) {
            headers["Authorization"] = `Bearer ${this.token}`;
        }
        const response = await fetch(this.url, {
            method: "POST",
            body: JSON.stringify({
                method: method,
                jsonrpc: "2.0",
                id: id,
                params: params || {},
            }),
            headers,
        });
        const json = await response.json();
        if (json.error) {
            throw new Error(`${json.error.code}: ${json.error.message}`);
        }
        return json.result;
    }
}