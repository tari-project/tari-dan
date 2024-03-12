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

import {Mutex} from "async-mutex";
import type {
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
    AccountsInvokeRequest,
    AccountsInvokeResponse,
    AccountsListRequest,
    AccountsListResponse,
    AuthGetAllJwtRequest,
    AuthGetAllJwtResponse,
    AuthLoginAcceptRequest,
    AuthLoginAcceptResponse,
    AuthLoginDenyRequest,
    AuthLoginDenyResponse,
    AuthLoginRequest,
    AuthLoginResponse,
    AuthRevokeTokenRequest,
    AuthRevokeTokenResponse,
    CallInstructionRequest,
    ClaimBurnRequest,
    ClaimBurnResponse,
    ClaimValidatorFeesRequest,
    ClaimValidatorFeesResponse,
    ConfidentialCreateOutputProofRequest,
    ConfidentialCreateOutputProofResponse,
    ConfidentialTransferRequest,
    ConfidentialTransferResponse,
    GetAccountNftRequest,
    GetValidatorFeesRequest,
    GetValidatorFeesResponse,
    KeysCreateRequest,
    KeysCreateResponse,
    KeysListRequest,
    KeysListResponse,
    KeysSetActiveRequest,
    KeysSetActiveResponse,
    ListAccountNftRequest,
    ListAccountNftResponse,
    MintAccountNftRequest,
    MintAccountNftResponse,
    ProofsCancelRequest,
    ProofsCancelResponse,
    ProofsGenerateRequest,
    ProofsGenerateResponse,
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
    TransferRequest,
    TransferResponse,
    WebRtcStartRequest,
    WebRtcStartResponse,
} from "@tariproject/typescript-bindings/wallet-daemon-client";

import {
    NonFungibleToken
} from "@tariproject/typescript-bindings";

let token: String | null = null;
let json_id = 0;
let address = new URL("http://localhost:9000");
let isAddressSet = false;
const mutex_token = new Mutex();
const mutex_id = new Mutex();

async function internalJsonRpc(method: string, token: any = null, params: any = null) {
    let id;
    await mutex_id.runExclusive(() => {
        id = json_id;
        json_id += 1;
    });
    if (!isAddressSet) {
        try {
            let resp = await fetch("/json_rpc_address");
            if (resp.status === 200) {
                address = new URL(await resp.text());
            }
        } catch (e) {
            console.warn(e);
        }

        isAddressSet = true;
    }

    let headers: { [key: string]: string } = {
        "Content-Type": "application/json",
    };
    if (token) {
        headers["Authorization"] = `Bearer ${token}`;
    }
    let response = await fetch(address, {
        method: "POST",
        body: JSON.stringify({
            method: method,
            jsonrpc: "2.0",
            id: id,
            params: params,
        }),
        headers: headers,
    });
    let json = await response.json();
    if (json.error) {
        console.error(json.error);
        throw json.error;
    }
    return json.result;
}

async function jsonRpc(method: string, params: any = null) {
    await mutex_token.runExclusive(async () => {
        if (token === null) {
            let auth_response = await internalJsonRpc("auth.request", null, [["Admin"], null]);
            let auth_token = auth_response["auth_token"];
            let accept_response = await internalJsonRpc("auth.accept", null, [auth_token, auth_token]);
            token = accept_response.permissions_token;
        }
    });
    // This will fail if the token is expired
    return internalJsonRpc(method, token, params);
}

// auth
export const authLogin = (request: AuthLoginRequest): Promise<AuthLoginResponse> => jsonRpc("auth.request");
export const authAccept = (request: AuthLoginAcceptRequest): Promise<AuthLoginAcceptResponse> => jsonRpc("auth.accept");
export const authDeny = (request: AuthLoginDenyRequest): Promise<AuthLoginDenyResponse> => jsonRpc("auth.deny");
export const authRevoke = (request: AuthRevokeTokenRequest): Promise<AuthRevokeTokenResponse> => jsonRpc("auth.revoke");
export const authGetAllJwt = (request: AuthGetAllJwtRequest): Promise<AuthGetAllJwtResponse> =>
    jsonRpc("auth.get_all_jwt");

// settings
export const settingsGet = (): Promise<SettingsGetResponse> => jsonRpc("settings.get", []);
export const settingsSet = (request: SettingsSetRequest): Promise<SettingsSetResponse> =>
    jsonRpc("settings.set", request);

// webrtc
export const webrtcStart = (request: WebRtcStartRequest): Promise<WebRtcStartResponse> =>
    jsonRpc("webrtc.start", request);

// rpc
export const rpcDiscover = (): Promise<string> => jsonRpc("rpc.discover");

// keys
export const keysCreate = (request: KeysCreateRequest): Promise<KeysCreateResponse> => jsonRpc("keys.create", request);
export const keysList = (request: KeysListRequest): Promise<KeysListResponse> => jsonRpc("keys.list", request);
export const keysSetActive = (request: KeysSetActiveRequest): Promise<KeysSetActiveResponse> =>
    jsonRpc("keys.set_active", request);

export const transactionsSubmitInstruction = (request: CallInstructionRequest): Promise<TransactionSubmitResponse> =>
    jsonRpc("transactions.submit_instruction", request);
export const transactionsSubmit = (request: TransactionSubmitRequest): Promise<TransactionSubmitResponse> =>
    jsonRpc("transactions.submit", request);
export const transactionsGet = (request: TransactionGetRequest): Promise<TransactionGetResponse> =>
    jsonRpc("transactions.get", request);
export const transactionsGetResult = (request: TransactionGetResultRequest): Promise<TransactionGetResultResponse> =>
    jsonRpc("transactions.get_result", request);
export const transactionsWaitResult = (request: TransactionWaitResultRequest): Promise<TransactionWaitResultResponse> =>
    jsonRpc("transactions.wait_result", request);
export const transactionsGetAll = (request: TransactionGetAllRequest): Promise<TransactionGetAllResponse> =>
    jsonRpc("transactions.get_all", request);

// accounts

export const accountsRevealFunds = (request: RevealFundsRequest): Promise<RevealFundsResponse> =>
    jsonRpc("accounts.reveal_funds", request);
export const accountsClaimBurn = (request: ClaimBurnRequest): Promise<ClaimBurnResponse> =>
    jsonRpc("accounts.claim_burn", request);
export const accountsCreate = (request: AccountsCreateRequest): Promise<AccountsCreateResponse> =>
    jsonRpc("accounts.create", request);
export const accountsList = (request: AccountsListRequest): Promise<AccountsListResponse> =>
    jsonRpc("accounts.list", request);
export const accountsGetBalances = (request: AccountsGetBalancesRequest): Promise<AccountsGetBalancesResponse> =>
    jsonRpc("accounts.get_balances", request);
export const accountsInvoke = (request: AccountsInvokeRequest): Promise<AccountsInvokeResponse> =>
    jsonRpc("accounts.invoke", request);
export const accountsGet = (request: AccountGetRequest): Promise<AccountGetResponse> =>
    jsonRpc("accounts.get", request);
export const accountsGetDefault = (request: AccountGetDefaultRequest): Promise<AccountGetResponse> =>
    jsonRpc("accounts.get_default", request);
export const accountsTransfer = (request: TransferRequest): Promise<TransferResponse> =>
    jsonRpc("accounts.transfer", request);
export const accountsConfidentialTransfer = (
    request: ConfidentialTransferRequest,
): Promise<ConfidentialTransferResponse> => jsonRpc("accounts.confidential_transfer", request);
export const accountsSetDefault = (request: AccountSetDefaultRequest): Promise<AccountSetDefaultResponse> =>
    jsonRpc("accounts.set_default", request);
export const accountsCreateFreeTestCoins = (
    request: AccountsCreateFreeTestCoinsRequest,
): Promise<AccountsCreateFreeTestCoinsResponse> => jsonRpc("accounts.create_free_test_coins", request);

// confidential
export const confidentialCreateTransferProof = (request: ProofsGenerateRequest): Promise<ProofsGenerateResponse> =>
    jsonRpc("confidential.create_transfer_proof", request);
export const confidentialFinalize = (request: ProofsCancelRequest): Promise<ProofsCancelResponse> =>
    jsonRpc("confidential.finalize", request);
export const confidentialCancel = (request: ProofsCancelRequest): Promise<ProofsCancelResponse> =>
    jsonRpc("confidential.cancel", request);
export const confidentialCreateOutputProof = (
    request: ConfidentialCreateOutputProofRequest,
): Promise<ConfidentialCreateOutputProofResponse> => jsonRpc("confidential.create_output_proof", request);

// nfts
export const nftMintAccountNft = (request: MintAccountNftRequest): Promise<MintAccountNftResponse> =>
    jsonRpc("nfts.mint_account_nft", request);
export const nftGet = (request: GetAccountNftRequest): Promise<NonFungibleToken> => jsonRpc("nfts.get", request);
export const nftList = (request: ListAccountNftRequest): Promise<ListAccountNftResponse> =>
    jsonRpc("nfts.list", request);

// validators
export const validatorsGetFeeSummary = (request: GetValidatorFeesRequest): Promise<GetValidatorFeesResponse> =>
    jsonRpc("validators.get_fee_summary", request);
export const validatorsClaimFees = (request: ClaimValidatorFeesRequest): Promise<ClaimValidatorFeesResponse> =>
    jsonRpc("validators.claim_fees", request);
