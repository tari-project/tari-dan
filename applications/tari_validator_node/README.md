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
- create_backup_snapshot

#### Linux

```
sudo apt-get install git curl build-essential cmake clang pkg-config libssl-dev libsqlite3-dev sqlite3 npm
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

```

### From source

```
cargo install tari_validator_node
```

### Backups

`create_backup_snapshot` writes a point-in-time copy of the node's state while it keeps running, and returns the
directory it wrote:

```
curl -s -X POST http://127.0.0.1:18200/json_rpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"create_backup_snapshot","params":{}}'
```

The directory is `<data_dir>/backups/<UTC timestamp>/` and holds:

- `rocksdb/`, a RocksDB checkpoint of the state store
- `global_storage.sqlite`, a copy of the global database

The checkpoint hard-links the store's SST files, so it takes little time and space when it is taken, as long as
`state_db_path` is on the same filesystem as the data directory (RocksDB copies the files otherwise). It keeps those
files on disk while it exists, though, even after compaction has replaced them in the live store. Copy the snapshot
off the host, then delete it: the node refuses another snapshot while one is still present under `backups/`. The same
goes for a `*.partial` directory, which is left behind if the node exits while writing a snapshot.

The snapshot does not include the node's keys (`validator_node_id.json`, `shard_key.json`) or its configuration. Back
those up separately.

To restore:

1. Stop the node.
2. Replace the state store directory (`state_db_path`, `<data_dir>/rocksdb/` by default) and
   `<data_dir>/global_storage.sqlite` with the snapshot's copies. Delete any `<data_dir>/global_storage.sqlite-journal`
   left by the old database, or SQLite will apply it to the restored copy.
3. Wait until the network is in a later epoch than the one in which the node stopped.
4. Start the node. It syncs the blocks it missed from its peers.

Step 3 matters because the snapshot also holds the node's record of the last height it voted on and proposed at in
each epoch, which is what stops the node voting or proposing twice at one height. A restored node has the record as
of the snapshot, so within the same epoch it could vote or propose again at a height it already
used before it stopped, which other validators treat as equivocation. Votes and proposals belong to a single epoch,
so in a later epoch the old record no longer matters.

Only one node may run with a given identity. Restoring onto a new host while the original is still running makes the
two vote under the same key, which other validators treat as equivocation.
