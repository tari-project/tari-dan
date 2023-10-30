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

async function jsonRpc(method: string, params: any = null) {
  let id = 0;
  id += 1;
  let address = 'http://localhost:3333';
  try {
    address = await (await fetch('/json_rpc_address')).text();
    if (!address.startsWith("http")) {
      address = "http://" + address;
    }
  } catch {}
  let response = await fetch(address, {
    method: 'POST',
    body: JSON.stringify({
      method: method,
      jsonrpc: '2.0',
      id: id,
      params: params,
    }),
    headers: {
      'Content-Type': 'application/json',
    },
  });
  let json = await response.json();
  if (json.error) {
    throw json.error;
  }
  return json.result;
}

async function getOpenRpcSchema() {
  return await jsonRpc('rpc.discover');
}

async function getIdentity() {
  return await jsonRpc('get_identity');
}
async function getCommsStats() {
  return await jsonRpc('get_comms_stats');
}
async function getAllVns(epoch: number) {
  return await jsonRpc('get_network_validators', {epoch});
}
async function getConnections() {
  return await jsonRpc('get_connections');
}
async function addPeer(public_key: string, addresses: string[]) {
  return await jsonRpc('add_peer', {
    public_key,
    addresses,
    wait_for_dial: false,
  });
}
async function getRecentTransactions() {
  return await jsonRpc('get_recent_transactions');
}
async function getSubstate(address: string, version?: number) {
  return await jsonRpc('get_substate', { address, version });
}
async function inspectSubstate(address: string, version?: number) {
  return await jsonRpc('inspect_substate', { address, version });
}
async function getAddresses() {
  return await jsonRpc('get_addresses');
}
async function addAddress(address: string) {
  return await jsonRpc('add_address', [address]);
}
async function deleteAddress(address: string) {
  return await jsonRpc('delete_address', [address]);
}
async function clearAddresses() {
  return await jsonRpc('clear_addresses');
}
async function getNonFungibleCollections() {
  return await jsonRpc('get_non_fungible_collections');
}
async function getNonFungibles(
  address: string,
  start_index: number,
  end_index: number
) {
  return await jsonRpc('get_non_fungibles', {
    address,
    start_index,
    end_index,
  });
}
async function getNonFungibleCount(address: string) {
  return await jsonRpc('get_non_fungible_count', [address]);
}

export {
  addAddress,
  addPeer,
  clearAddresses,
  deleteAddress,
  getAddresses,
  getAllVns,
  getCommsStats,
  getConnections,
  getIdentity,
  getOpenRpcSchema,
  getRecentTransactions,
  getSubstate,
  getNonFungibleCollections,
  getNonFungibles,
  getNonFungibleCount,
  inspectSubstate,
};
