async function jsonRpc(method: string, params: any = null) {
  let id = 0;
  id += 1;
  let address = "127.0.0.1:18145";
  try {
    let text = await (await fetch("json_rpc_address")).text();
    if (/^\d+(\.\d+){3}:[0-9]+$/.test(text)) {
      address = text;
    }
  } catch {}
  let response = await fetch(`http://${address}`, {
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
async function getCommittee(height: number, shard_key: string) {
  return await jsonRpc("get_committee", [height, shard_key]);
}
async function getAllVns(epoch: number) {
  return await jsonRpc("get_all_vns", epoch);
}
async function registerValidatorNode() {
  return await jsonRpc("register_validator_node");
}

export {
  getIdentity,
  getEpochManagerStats,
  getCommsStats,
  getMempoolStats,
  getShardKey,
  getCommittee,
  getAllVns,
  registerValidatorNode,
};
