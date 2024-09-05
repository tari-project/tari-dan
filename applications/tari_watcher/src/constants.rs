// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

pub const DEFAULT_WATCHER_BASE_PATH: &str = "data/watcher/";
pub const DEFAULT_WATCHER_CONFIG_PATH: &str = "data/watcher/config.toml";
pub const DEFAULT_VALIDATOR_PID_PATH: &str = "data/watcher/validator.pid";
pub const DEFAULT_VALIDATOR_DIR: &str = "data/vn1";
pub const DEFAULT_VALIDATOR_KEY_PATH: &str = "data/vn1/esmeralda/registration.json";
pub const DEFAULT_VALIDATOR_NODE_BINARY_PATH: &str = "target/release/tari_validator_node";
pub const DEFAULT_BASE_NODE_GRPC_URL: &str = "http://127.0.0.1:12001"; // note: protocol
pub const DEFAULT_BASE_WALLET_GRPC_URL: &str = "http://127.0.0.1:12003"; // note: protocol

pub const DEFAULT_THRESHOLD_WARN_EXPIRATION: u64 = 100; // warn at this many blocks before the registration expires
