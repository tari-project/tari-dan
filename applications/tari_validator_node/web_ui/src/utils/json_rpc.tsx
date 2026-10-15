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
  ClearDiagnosticEventsRequest,
  ClearDiagnosticEventsResponse,
  GetBlockRequest,
  GetBlockResponse,
  GetBlocksCountResponse,
  GetBlocksRequest,
  GetBlocksResponse,
  GetDiagnosticEventsRequest,
  GetDiagnosticEventsResponse,
  GetEpochManagerStatsResponse,
  GetFilteredBlocksCountRequest,
  GetMempoolStatsResponse,
  GetShardKeyRequest,
  GetShardKeyResponse,
  GetStateRequest,
  GetStateResponse,
  GetTemplateRequest,
  GetTemplateResponse,
  GetTransactionRequest,
  GetTransactionResponse,
  GetTxPoolResponse,
  ListBlocksRequest,
  ListBlocksResponse,
  VNAddPeerRequest,
  VNGetAllVnsRequest,
  VNGetAllVnsResponse,
  VnGetCommitteeRequest,
  VnGetCommitteeResponse,
  VNGetConnectionsResponse,
  VNGetIdentityResponse,
  VNGetSubstateRequest,
  VNGetSubstateResponse,
  VNGetTransactionResultRequest,
  VNGetTransactionResultResponse,
  VNSubmitTransactionRequest,
  VNSubmitTransactionResponse,
} from "@tari-project/ootle-ts-bindings";

// The validator serves this UI from its JSON-RPC listener, so the same origin is the default.
// A Vite dev server sets VITE_JSON_RPC_ADDRESS to reach a validator started with --enable-permissive-cors.
const JSON_RPC_ADDRESS: string =
  import.meta.env.VITE_JRPC_ADDRESS || import.meta.env.VITE_JSON_RPC_ADDRESS || "/json_rpc";

async function jsonRpc(method: string, params: any = null) {
  let id = 0;
  id += 1;
  let response = await fetch(new URL(JSON_RPC_ADDRESS, window.location.origin), {
    method: "POST",
    body: JSON.stringify({
      method: method,
      jsonrpc: "2.0",
      id,
      params: params,
    }),
    headers: {
      "Content-Type": "application/json",
    },
  });
  let json = await response.json();
  console.log(method, json);
  if (json.error) {
    throw json.error;
  }
  return json.result;
}

// Transaction
export const submitTransaction = (request: VNSubmitTransactionRequest): Promise<VNSubmitTransactionResponse> =>
  jsonRpc("submit_transaction", request);
export const getTransaction = (request: GetTransactionRequest): Promise<GetTransactionResponse> =>
  jsonRpc("get_transaction", request);
export const getTransactionResult = (request: VNGetTransactionResultRequest): Promise<VNGetTransactionResultResponse> =>
  jsonRpc("get_transaction_result", request);
export const getState = (request: GetStateRequest): Promise<GetStateResponse> => jsonRpc("get_state", request);
export const getSubstate = (request: VNGetSubstateRequest): Promise<VNGetSubstateResponse> =>
  jsonRpc("get_substate", request);
export const listBlocks = (request: ListBlocksRequest): Promise<ListBlocksResponse> => jsonRpc("list_blocks", request);
export const getTxPool = (): Promise<GetTxPoolResponse> => jsonRpc("get_tx_pool");

// Blocks
export const getBlock = (request: GetBlockRequest): Promise<GetBlockResponse> => jsonRpc("get_block", request);
export const getBlocks = (request: GetBlocksRequest): Promise<GetBlocksResponse> => jsonRpc("get_blocks", request);
export const getFilteredBlocksCount = (request: GetFilteredBlocksCountRequest): Promise<GetBlocksCountResponse> =>
  jsonRpc("get_filtered_blocks_count", request);

// Template
export const getTemplate = (request: GetTemplateRequest): Promise<GetTemplateResponse> =>
  jsonRpc("get_template", request);

// Validator Node
export const getIdentity = (): Promise<VNGetIdentityResponse> => jsonRpc("get_identity");

// Diagnostics
export const getDiagnosticEvents = (request: GetDiagnosticEventsRequest): Promise<GetDiagnosticEventsResponse> =>
  jsonRpc("get_diagnostic_events", request);
export const clearDiagnosticEvents = (request: ClearDiagnosticEventsRequest): Promise<ClearDiagnosticEventsResponse> =>
  jsonRpc("clear_diagnostic_events", request);

export const getMempoolStats = (): Promise<GetMempoolStatsResponse> => jsonRpc("get_mempool_stats");
export const getEpochManagerStats = (): Promise<GetEpochManagerStatsResponse> => jsonRpc("get_epoch_manager_stats");
export const getShardKey = (request: GetShardKeyRequest): Promise<GetShardKeyResponse> =>
  jsonRpc("get_shard_key", request);
export const getCommittee = (request: VnGetCommitteeRequest): Promise<VnGetCommitteeResponse> =>
  jsonRpc("get_committee", request);
export const getAllVns = (request: VNGetAllVnsRequest): Promise<VNGetAllVnsResponse> => jsonRpc("get_all_vns", request);
// export const getFees = (request: VNGetValidatorFeesRequest): Promise<VNGetValidatorFeesResponse> =>
//   jsonRpc("get_fees", request);

// Comms
export const addPeer = (request: VNAddPeerRequest) => jsonRpc("add_peer", request);
export const getConnections = (): Promise<VNGetConnectionsResponse> => jsonRpc("get_connections");
