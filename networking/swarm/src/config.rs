//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use libp2p::ping;

use crate::protocol_version::ProtocolVersion;

#[derive(Debug, Clone)]
pub struct Config {
    pub protocol_version: ProtocolVersion,
    pub user_agent: String,
    pub messaging_protocol: String,
    pub ping: ping::Config,
    pub max_connections_per_peer: Option<u32>,
    pub enable_mdns: bool,
    pub enable_relay: bool,
    pub idle_connection_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            protocol_version: "/tari/localnet/0.0.1".parse().unwrap(),
            user_agent: "/tari/unknown/0.0.1".to_string(),
            messaging_protocol: "/tari/messaging/0.0.1".to_string(),
            ping: ping::Config::default(),
            max_connections_per_peer: Some(3),
            enable_mdns: false,
            enable_relay: false,
            idle_connection_timeout: Duration::from_secs(10 * 60),
        }
    }
}
