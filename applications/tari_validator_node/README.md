# Tari Validator Node

### Host requirements

- **Supported platforms:** Linux (x86_64, arm64) and macOS (arm64). Windows is not supported for the
  validator node or indexer, because the engine manages guest memory with the unix
  `mmap`/`mprotect` system calls; attempting to compile for Windows fails with an error saying so.
  The wallet daemon and wallet CLI do not embed the engine and are unaffected.
- **Virtual address space (`ulimit -v` / `RLIMIT_AS`) must be unlimited** (the default on most
  distributions). Every executing WASM template reserves ~8 GiB of address space for its guest
  memory, and the engine keeps a pool of up to 2 × vCPU reservations alive for the life of the
  process (~512 GiB of address space on a 32-vCPU host). This is reservation only — it consumes no
  physical memory or commit charge, and container memory limits (Docker/k8s) are unaffected because
  they count resident memory, not address space.
- **`vm.max_map_count`:** the Linux default (65530) is ample; pooled reservations use a couple of
  mappings each.

### Web GUI

React frontend served from the JSON-RPC listener (default `http://127.0.0.1:18200`).
Shows all information about the VN:

- pub keys
- shard key
- comms state
- epoch manage state
- all the committees (with the respective shard space) that VN is part of
- list of all VNs

There is also functionality to register the VNs.
Auto-update of frontend.
Source code for this is in the `tari_validator_node_web_ui`

### JSON-RPC

Server is running by default on `127.0.0.1:18200`, at `/json_rpc`. It exposes every operator function without
authentication, so keep it off public interfaces. Cross-origin browser requests are refused unless the node is started
with `--enable-permissive-cors`. Requests must address the node by `localhost`, an IP address or a name listed in
`validator_node.json_rpc_allowed_hosts` (`--json-rpc-allowed-host`); any other host name gets `421 Misdirected Request`.

- submit_transaction
- register_template
- get_identity
- register_validator_node
- get_mempool_stats
- get_epoch_manager_stats
- get_shard_key
- get_committee
- get_all_vns
- get_comms_stats
- get_connections

#### Linux

```
sudo apt-get install git curl build-essential cmake clang pkg-config libssl-dev libsqlite3-dev sqlite3 npm
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

```

### From source

```
cargo install tari_validator_node
```
