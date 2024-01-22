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

import { GetNetworkCommitteesResponse } from "./interfaces";

async function jsonRpc(method: string, params: any = null) {
  let id = 0;
  id += 1;
  let address = "http://127.0.0.1:18010";
  try {
    address = await (await fetch("/json_rpc_address")).text();
    if (!address.startsWith("http")) {
      address = "http://" + address;
    }
  } catch {}
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
  console.log(method, json);
  if (json.error) {
    throw json.error;
  }
  return json.result;
}
async function getIdentity() {
  return await jsonRpc("get_identity");
}
async function getEpochManagerStats() {
  return await jsonRpc("get_epoch_manager_stats");
}
async function getCommsStats() {
  return await jsonRpc("get_comms_stats");
}
async function getMempoolStats() {
  return await jsonRpc("get_mempool_stats");
}
async function getShardKey(height: number, public_key: string) {
  return await jsonRpc("get_shard_key", [height, public_key]);
}
async function getCommittee(epoch: number, substate_address: string) {
  return await jsonRpc("get_committee", { epoch, substate_address });
}
async function getAllVns(epoch: number) {
  return await jsonRpc("get_all_vns", epoch);
}
async function getNetworkCommittees(): Promise<GetNetworkCommitteesResponse> {
  return await jsonRpc("get_network_committees", {});
}
async function getConnections() {
  return await jsonRpc("get_connections");
}
async function addPeer(public_key: string, addresses: string[]) {
  return await jsonRpc("add_peer", {
    public_key,
    addresses,
    wait_for_dial: false,
  });
}
async function registerValidatorNode(feeClaimPublicKeyHex: string) {
  return await jsonRpc("register_validator_node", {
    fee_claim_public_key: feeClaimPublicKeyHex,
  });
}
async function getRecentTransactions() {
  return await jsonRpc("get_recent_transactions");
}
async function getTransaction(transaction_id: string) {
  return await jsonRpc("get_transaction", { transaction_id });
}
async function getFees(start_epoch: number, end_epoch: number, claim_leader_public_key: string) {
  return await jsonRpc("get_fees", [[start_epoch, end_epoch], claim_leader_public_key]);
}
async function getUpSubstates(transaction_id: string) {
  return await jsonRpc("get_substates_created_by_transaction", {
    transaction_id,
  });
}
async function getDownSubstates(transaction_id: string) {
  return await jsonRpc("get_substates_destroyed_by_transaction", {
    transaction_id,
  });
}
async function getTemplates(limit: number) {
  return await jsonRpc("get_templates", [limit]);
}
async function getTemplate(address: string) {
  return await jsonRpc("get_template", [address]);
}

async function listBlocks(block_id: string | null, limit: number) {
  return await jsonRpc("list_blocks", [block_id, limit]);
}

async function getBlock(block_id: string) {
  return await jsonRpc("get_block", [block_id]);
}

async function getBlocksCount() {
  return await jsonRpc("get_blocks_count");
}

export {
  getAllVns,
  getBlock,
  listBlocks,
  getBlocksCount,
  getCommittee,
  getCommsStats,
  getConnections,
  addPeer,
  getEpochManagerStats,
  getIdentity,
  getMempoolStats,
  getRecentTransactions,
  getShardKey,
  getTemplate,
  getTemplates,
  getTransaction,
  getFees,
  getUpSubstates,
  getDownSubstates,
  getNetworkCommittees,
  registerValidatorNode,
};
