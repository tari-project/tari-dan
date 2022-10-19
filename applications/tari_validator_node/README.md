# Tari Validator Node

### Web GUI

React frontend that uses the JSON-RPC backend running. By default runs on port 5000.
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

Server is running by default on port 18145. Exposing all the functionality.

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
