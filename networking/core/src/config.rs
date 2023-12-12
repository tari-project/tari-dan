//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub swarm: tari_swarm::Config,
    pub listener_port: u16,
    pub reachability_mode: ReachabilityMode,
}

#[derive(Debug, Clone, Default)]
pub enum ReachabilityMode {
    #[default]
    Auto,
    Private,
}

impl ReachabilityMode {
    pub fn is_private(&self) -> bool {
        matches!(self, ReachabilityMode::Private)
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, ReachabilityMode::Auto)
    }
}
