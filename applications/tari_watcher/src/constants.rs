// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use tokio::time::Duration;

pub const CONSENSUS_CONSTANT_REGISTRATION_DURATION: u64 = 1000; // in blocks: 100 epochs * 10 blocks/epoch

pub const DEFAULT_MAIN_PROJECT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
pub const DEFAULT_WATCHER_CONFIG_PATH: &str = "data/watcher/config.toml";
pub const DEFAULT_VALIDATOR_PID_PATH: &str = "data/watcher/validator.pid";
pub const DEFAULT_VALIDATOR_DIR: &str = "data/vn1";
pub const DEFAULT_VALIDATOR_KEY_PATH: &str = "data/vn1/esmeralda/registration.json";
pub const DEFAULT_VALIDATOR_NODE_BINARY_PATH: &str = "target/release/tari_validator_node";
pub const DEFAULT_MINOTARI_MINER_BINARY_PATH: &str = "target/release/minotari_miner";
pub const DEFAULT_BASE_NODE_GRPC_ADDRESS: &str = "http://127.0.0.1:12001"; // note: protocol
pub const DEFAULT_BASE_WALLET_GRPC_ADDRESS: &str = "http://127.0.0.1:12003"; // note: protocol

pub const DEFAULT_PROCESS_MONITORING_INTERVAL: Duration = Duration::from_secs(20); // time to sleep before checking VN process status
pub const DEFAULT_THRESHOLD_WARN_EXPIRATION: u64 = 100; // warn at this many blocks before the registration expires
