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
  GetNetworkEconomicsResponse,
  GetNetworkSyncStateResponse,
  GetTransactionReceiptResponse,
  IndexerGetConnectionsResponse,
  IndexerGetIdentityResponse,
  IndexerGetSubstateResponse,
  IndexerGetTransactionResponse,
  IndexerGetTransactionResultRequest,
  IndexerGetTransactionResultResponse,
  ListRecentTransactionsRequest,
  ListRecentTransactionsResponse,
  ListTransactionReceiptsRequest,
  ListTransactionReceiptsResponse,
  ListValidatorsRequest,
  ListValidatorsResponse,
  ListWatchedSubstatesRequest,
  ListWatchedSubstatesResponse,
  ListWatchedTemplatesResponse,
  QueryTransactionEventsRequest,
  QueryTransactionEventsResponse,
  SubstateId,
  TransactionReceiptAddress,
} from "@tari-project/ootle-ts-bindings";

const CONFIGURED_API_ADDRESS = import.meta.env.VITE_INDEXER_API_ADDRESS || import.meta.env.VITE_API_ADDRESS;

const DEFAULT_API_ADDRESS = new URL(CONFIGURED_API_ADDRESS || "http://localhost:9000");

export async function getClientAddress(): Promise<URL> {
  // `/rest_api_address` is served by the indexer that also serves this UI. A build configured
  // with an explicit address is not hosted by an indexer, so there is nothing to ask.
  if (CONFIGURED_API_ADDRESS) {
    return DEFAULT_API_ADDRESS;
  }

  try {
    const resp = await fetch("/rest_api_address");
    if (resp.status === 200) {
      const url = await resp.text();
      return new URL(url);
    }
  } catch (e) {
    console.warn(e);
  }

  return DEFAULT_API_ADDRESS;
}

let clientInstance: IndexerClient | null = null;
let pendingClientInstance: Promise<IndexerClient> | null = null;
let outerAddress: URL | null = null;

export async function client() {
  if (clientInstance) {
    return Promise.resolve(clientInstance);
  }

  const getAddress = outerAddress ? Promise.resolve(outerAddress) : getClientAddress();

  pendingClientInstance = getAddress.then(async (addr) => {
    const client = IndexerClient.usingFetchTransport(addr.toString());
    outerAddress = addr;
    clientInstance = client;
    pendingClientInstance = null;
    return client;
  });

  return pendingClientInstance;
}

export const getIdentity = (): Promise<IndexerGetIdentityResponse> => client().then((c) => c.identityGet());

export const getConnections = (): Promise<IndexerGetConnectionsResponse> => client().then((c) => c.getConnections());

export const getNetworkStats = (): Promise<GetNetworkSyncStateResponse> => client().then((c) => c.networkStats());

export const getNetworkEconomics = (): Promise<GetNetworkEconomicsResponse> =>
  client().then((c) => c.getNetworkEconomics());

export const listValidators = (params: Partial<ListValidatorsRequest> = {}): Promise<ListValidatorsResponse> =>
  client().then((c) => c.listValidators(params));

export const getSubstate = (
  id: SubstateId,
  version?: number | null,
  local_search_only?: boolean,
): Promise<IndexerGetSubstateResponse> =>
  client().then((c) =>
    c.substatesGet(id, {
      version: version ?? null,
      local_search_only: local_search_only ?? false,
      include_proof: false,
    }),
  );
export const getTransaction = (transaction_id: string): Promise<IndexerGetTransactionResponse> =>
  client().then((c) => c.getTransaction(transaction_id));

export const getTransactionResult = (
  request: IndexerGetTransactionResultRequest,
): Promise<IndexerGetTransactionResultResponse> => client().then((c) => c.getTransactionResult(request.transaction_id));

export const listRecentTransactions = (
  request: ListRecentTransactionsRequest,
): Promise<ListRecentTransactionsResponse> => client().then((c) => c.listRecentTransactions(request));

export const queryTransactionEvents = (req: QueryTransactionEventsRequest): Promise<QueryTransactionEventsResponse> =>
  client().then((c) => c.queryTransactionEvents(req));

export const listTransactionReceipts = (
  request: ListTransactionReceiptsRequest,
): Promise<ListTransactionReceiptsResponse> => client().then((c) => c.listTransactionReceipts(request));

export const getTransactionReceipt = (address: TransactionReceiptAddress): Promise<GetTransactionReceiptResponse> =>
  client().then((c) => c.getTransactionReceipt(address));

export const getTemplateDefinition = (templateAddress: string): Promise<any> =>
  client().then((c) => c.templatesGet(templateAddress));

export const listWatchedTemplates = (): Promise<ListWatchedTemplatesResponse> =>
  client().then((c) => c.listWatchedTemplates());

export const listWatchedSubstates = (
  params: Partial<ListWatchedSubstatesRequest>,
): Promise<ListWatchedSubstatesResponse> => client().then((c) => c.listWatchedSubstates(params));
