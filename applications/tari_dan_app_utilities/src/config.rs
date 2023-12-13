//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, fmt::Display, str::FromStr};

use anyhow::anyhow;
use tari_bor::{Deserialize, Serialize};
use tari_common::{configuration::StringList, SubConfigPath};

#[derive(Clone, Debug, Serialize, Deserialize)]
// TODO: update configs
// #[serde(deny_unknown_fields)]
pub struct P2pConfig {
    pub enable_mdns: bool,
    pub listener_port: u16,
    pub reachability_mode: ReachabilityMode,
}

impl Default for P2pConfig {
    fn default() -> Self {
        Self {
            enable_mdns: true,
            listener_port: 0,
            reachability_mode: ReachabilityMode::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub enum ReachabilityMode {
    #[default]
    Auto,
    Private,
}

impl From<ReachabilityMode> for tari_networking::ReachabilityMode {
    fn from(mode: ReachabilityMode) -> Self {
        match mode {
            ReachabilityMode::Auto => tari_networking::ReachabilityMode::Auto,
            ReachabilityMode::Private => tari_networking::ReachabilityMode::Private,
        }
    }
}

impl FromStr for ReachabilityMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(ReachabilityMode::Auto),
            "private" => Ok(ReachabilityMode::Private),
            _ => Err(anyhow!("Invalid reachability mode '{}'", s)),
        }
    }
}

impl Display for ReachabilityMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReachabilityMode::Auto => write!(f, "Auto"),
            ReachabilityMode::Private => write!(f, "Private"),
        }
    }
}

/// Peer seed configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerSeedsConfig {
    pub override_from: Option<String>,
    /// Custom specified peer seed nodes
    pub peer_seeds: StringList,
    /// DNS seeds hosts. The DNS TXT records are queried from these hosts and the resulting peers added to the comms
    /// peer list.
    pub dns_seeds: StringList,
    // TODO
    // #[serde(
    //     deserialize_with = "deserialize_string_or_struct",
    //     serialize_with = "serialize_string"
    // )]
    // /// DNS name server to use for DNS seeds.
    // pub dns_seeds_name_server: DnsNameServer,
    // /// All DNS seed records must pass DNSSEC validation
    // pub dns_seeds_use_dnssec: bool,
}

impl SubConfigPath for PeerSeedsConfig {
    fn main_key_prefix() -> &'static str {
        "p2p.seeds"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcConfig {
    pub max_simultaneous_sessions: usize,
    pub max_sessions_per_client: usize,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            // TODO: autofiller uses a lot of sessions, once session management is improved we can reduce these
            max_simultaneous_sessions: 1000,
            max_sessions_per_client: 100,
        }
    }
}
