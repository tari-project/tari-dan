/*
 * //  Copyright 2024 The Tari Project
 * //  SPDX-License-Identifier: BSD-3-Clause
 */

import {
    AccountsGetBalancesRequest,
    AccountsGetBalancesResponse,
    TransactionSubmitRequest,
    TransactionSubmitResponse,
    AccountGetRequest,
    AccountGetResponse, AccountGetDefaultRequest
} from 'bindings/index';

export type { AccountsGetBalancesRequest, AccountsGetBalancesResponse, TransactionSubmitRequest, TransactionSubmitResponse, AccountGetRequest, AccountGetResponse, AccountGetDefaultRequest };

export function createClient(url: string): WalletDaemonClient {
    return new WalletDaemonClient(url);
}

export class WalletDaemonClient {
    private url: string;
    private id: number;

    constructor(url: string) {
        this.url = url;
        this.id = 0;
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

    async __invokeRpc(method: string, params: Object = null) {
        let id = this.id++;
        let response = await fetch(this.url, {
            method: "POST",
            body: JSON.stringify({
                method: method,
                jsonrpc: "2.0",
                id: id,
                params: params || {},
            }),
            headers: {
                "Content-Type": "application/json",
            },
        });
        let json = await response.json();
        if (json.error) {
            throw new Error(`${json.error.code}: ${json.error.message}`);
        }
        return json.result;
    }
}