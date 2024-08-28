# Tari Watcher

**Features**:
* Registers the validator node on L2 by sending a transaction on L1
* Monitors the chain and warns when registration is near expiration
* Automatically re-registers the node
* Alerts on Mattermost and Telegram

### Quickstart

Initialize the project with `tari_watcher init` and start it with `tari_watcher run`. Edit the newly generated `config.toml` to enable notifications on channels such as Mattermost and Telegram.Make sure to have started up `tari_validator_node` once previously to have a node directory set up, default is `tari_validator_node -- -b data/vn1`.

### Setup

The default values used (see `constants.rs`) when running the project without any flags:
```
- DEFAULT_MAIN_PROJECT_PATH: base directory, the same level as the repository `tari-dan`
- DEFAULT_WATCHER_CONFIG_PATH: relative to the base directory, main configuration file
- DEFAULT_VALIDATOR_KEY_PATH: relative to the base directory, validator node registration file
- DEFAULT_VALIDATOR_NODE_BINARY_PATH: relative to the base directory, default is Rust build directory `target/release`
- DEFAULT_VALIDATOR_DIR: relative to the project base directory, home directory for everything validator node
- DEFAULT_MINOTARI_MINER_BINARY_PATH: relative to the base directory, default is Rust build directory `target/release`
- DEFAULT_BASE_NODE_GRPC_ADDRESS: default is Tari swarm localhost and port
- DEFAULT_BASE_WALLET_GRPC_ADDRESS: default is Tari swarm localhost and port
```

### Project

```
├── alerting.rs     # channel notifier implementations
├── cli.rs          # cli and flags passed during bootup
├── config.rs       # main config file creation 
├── constants.rs    # various constants used as default values
├── helpers.rs      # common helper functions
├── logger.rs
├── main.rs
├── manager.rs      # manages the spawn validator node process and receives requests
├── minotari.rs     # communicates with the base node (L1)
├── monitoring.rs   # outputs logs and sends the alerts
├── process.rs      # spawns the validator node process and sets up the channels
├── registration.rs # handles the logic for sending a node registration transaction
└── shutdown.rs
```
