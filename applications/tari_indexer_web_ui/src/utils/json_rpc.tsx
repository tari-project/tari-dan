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
  IndexerAddPeerRequest,
  IndexerAddPeerResponse,
  IndexerGetAllVnsResponse,
  IndexerGetCommsStatsResponse,
  IndexerGetConnectionsResponse,
  GetEpochManagerStatsResponse,
  IndexerGetIdentityResponse,
  GetNonFungibleCollectionsResponse,
  GetNonFungibleCountRequest,
  GetNonFungibleCountResponse,
  GetNonFungiblesRequest,
  GetNonFungiblesResponse,
  GetRelatedTransactionsRequest,
  GetRelatedTransactionsResponse,
  IndexerGetSubstateRequest,
  IndexerGetSubstateResponse,
  IndexerGetTransactionResultRequest,
  IndexerGetTransactionResultResponse,
  InspectSubstateRequest,
  InspectSubstateResponse,
  IndexerSubmitTransactionResponse,
} from "@tari-project/typescript-bindings";

async function jsonRpc(method: string, params: any = null) {
  let id = 0;
  id += 1;
  let address = "http://localhost:18300";
  try {
    address = await (await fetch("/json_rpc_address")).text();
    if (!address.startsWith("http")) {
      address = "http://" + address;
    }
  } catch (e) {
    console.warn("Failed to fetch address", e);
  }
  let response = await fetch(address, {
    method: "POST",
    body: JSON.stringify({
      method: method,
      jsonrpc: "2.0",
      id: id,
      params: params,
    }),
    headers: {
      "Content-Type": "application/json",
    },
  });
  let json = await response.json();
  if (json.error) {
    throw json.error;
  }
  return json.result;
}

export const getOpenRpcSchema = (): Promise<string> => jsonRpc("rpc.discover");
export const getIdentity = (): Promise<IndexerGetIdentityResponse> => jsonRpc("get_identity");
export const getAllVns = (epoch: number): Promise<IndexerGetAllVnsResponse> => jsonRpc("get_all_vns", { epoch });
export const addPeer = (request: IndexerAddPeerRequest): Promise<IndexerAddPeerResponse> => jsonRpc("add_peer", request);
export const getCommsStats = (): Promise<IndexerGetCommsStatsResponse> => jsonRpc("get_comms_stats");
export const getSubstate = (request: IndexerGetSubstateRequest): Promise<IndexerGetSubstateResponse> =>
  jsonRpc("get_substate", request);
export const inspectSubstate = (request: InspectSubstateRequest): Promise<InspectSubstateResponse> =>
  jsonRpc("inspect_substate", request);
export const getConnections = (): Promise<IndexerGetConnectionsResponse> => jsonRpc("get_connections");
export const getNonFungibleCollections = (): Promise<GetNonFungibleCollectionsResponse> =>
  jsonRpc("get_non_fungible_collections");
export const getNonFungibleCount = (request: GetNonFungibleCountRequest): Promise<GetNonFungibleCountResponse> =>
  jsonRpc("get_non_fungible_count", request);
export const getNonFungibles = (request: GetNonFungiblesRequest): Promise<GetNonFungiblesResponse> =>
  jsonRpc("get_non_fungibles", request);
export const submitTransaction = (request: GetNonFungiblesRequest): Promise<IndexerSubmitTransactionResponse> =>
  jsonRpc("submit_transaction", request);
export const getTransactionResult = (request: IndexerGetTransactionResultRequest): Promise<IndexerGetTransactionResultResponse> =>
  jsonRpc("get_transaction_result", request);
export const getSubstateTransactions = (
  request: GetRelatedTransactionsRequest,
): Promise<GetRelatedTransactionsResponse> => jsonRpc("get_substate_transactions", request);
export const getEpochManagerStats = (): Promise<GetEpochManagerStatsResponse> => jsonRpc("get_epoch_manager_stats");
