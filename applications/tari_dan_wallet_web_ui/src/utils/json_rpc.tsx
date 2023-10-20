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

import { Mutex } from "async-mutex";
import { json } from "react-router-dom";

let token: String | null = null;
let json_id = 0;
const mutex_token = new Mutex();
const mutex_id = new Mutex();

async function internalJsonRpc(method: string, token: any = null, params: any = null) {
  let id;
  await mutex_id.runExclusive(() => {
    id = json_id;
    json_id += 1;
  });
  let address = import.meta.env.VITE_DAEMON_JRPC_ADDRESS || "http://localhost:9000";
  try {
    address = await (await fetch('/json_rpc_address')).text();
    if (!address.startsWith("http")) {
      address = "http://" + address;
    }
  } catch { }
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

export async function jsonRpc(method: string, params: any = null) {
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

// The 'any' types are structs I don't define them here, but we can add them later.

// auth
export const authLogin = () => jsonRpc("auth.login");

// jwts
export const getAllTokens = () => jsonRpc("auth.get_all_jwt", []);
export const authRevokeToken = (token: string) => jsonRpc("auth.revoke", [token]);

// rpc
export const rpcDiscover = () => jsonRpc("rpc.discover");

// keys
export const keysCreate = () => jsonRpc("keys.create", []);
export const keysList = () => jsonRpc("keys.list", []);
export const keysSetActive = (index: number) => jsonRpc("keys.set_active", [index]);

// transactions
export const transactionsSubmit = (
  signingKeyIndex: number | undefined,
  instructions: any[],
  fee: number,
  inputs: any[],
  overrideInputs: boolean,
  newOutputs: number,
  specificNonFungibleOutputs: any[],
  newNonFungibleOutputs: any[],
  newNonFungibleIndexOutputs: any[],
  isDryRun: boolean,
  proofId: any | undefined
) =>
  jsonRpc("transactions.submit", [
    signingKeyIndex,
    instructions,
    fee,
    inputs,
    overrideInputs,
    newOutputs,
    specificNonFungibleOutputs,
    newNonFungibleOutputs,
    newNonFungibleIndexOutputs,
    isDryRun,
    proofId,
  ]);
export const transactionsGet = (hash: string) => jsonRpc("transactions.get", [hash]);
export const transactionsGetResult = (hash: string) => jsonRpc("transactions.get_result", [hash]);
export const transactionsWaitResult = (hash: string, timeoutSecs: number | null) =>
  jsonRpc("transactions.wait_result", [hash, timeoutSecs]);

// accounts
export const accountsClaimBurn = (account: string, claimProof: any, fee: number) =>
  // Fees are passed as strings because Amount is tagged
  jsonRpc("accounts.claim_burn", {
    account,
    claim_proof: claimProof,
    fee: fee,
  });
export const accountsCreate = (
  accountName: string | undefined,
  signingKeyIndex: number | undefined,
  customAccessRules: any | undefined,
  fee: number | undefined,
  is_default: boolean | false
) => jsonRpc("accounts.create", [accountName, signingKeyIndex, customAccessRules, fee, is_default]);
export const accountsCreateFreeTestCoins = (
  accountName: string | undefined,
  amount: number | undefined,
  fee: number | undefined
) => jsonRpc("accounts.create_free_test_coins", [{ Name: accountName }, amount, fee]);
export const accountsList = (offset: number, limit: number) => jsonRpc("accounts.list", [offset, limit]);
export const accountsGetBalances = (accountName: string) => jsonRpc("accounts.get_balances", [accountName]);
export const accountsInvoke = (accountName: string, method: string, args: any[]) =>
  jsonRpc("accounts.invoke", [accountName, method, args]);
export const accountsGet = (nameOrAddress: string) => jsonRpc("accounts.get", [nameOrAddress]);

// confidential
export const confidentialCreateTransferProof = (
  amount: number,
  source_accountName: string,
  resourceAddress: string,
  destinationAccount: string,
  destinationStealthPublicKey: string
) =>
  jsonRpc("confidential.create_transfer_proof", [
    amount,
    source_accountName,
    resourceAddress,
    destinationAccount,
    destinationStealthPublicKey,
  ]);
export const confidentialFinalize = (proofId: number) => jsonRpc("confidential.finalize", [proofId]);
export const confidentialCancel = (proofId: number) => jsonRpc("confidential.cancel", [proofId]);
export const confidentialCreateOutputProof = (amount: number) => jsonRpc("confidential.create_output_proof", [amount]);

export const getAllTransaction = (status: string | null | undefined, component: string | null | undefined) =>
  jsonRpc("transactions.get_all", [status, component]);

export const webrtc = (signalingServerToken: string, permissions: string, name: string) =>
  jsonRpc("webrtc.start", [signalingServerToken, permissions, name]);

// nfts
export const accountNFTsList = (offset: number, limit: number) => jsonRpc("nfts.list", [offset, limit]);

// settings
export const getIndexerUrl = () => jsonRpc("settings.get_indexer_url", []);
export const setIndexerUrl = (indexer_url: string) => jsonRpc("settings.set_indexer_url", [indexer_url]);
