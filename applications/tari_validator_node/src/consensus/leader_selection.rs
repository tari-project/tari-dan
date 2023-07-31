//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::LeaderStrategy;
use tari_dan_common_types::{committee::Committee, NodeAddressable, NodeHeight};
use tari_dan_storage::consensus_models::BlockId;

#[derive(Debug, Clone, Copy, Default)]
pub struct RoundRobinLeaderStrategy;
impl RoundRobinLeaderStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for RoundRobinLeaderStrategy {
    fn calculate_leader(&self, committee: &Committee<TAddr>, height: NodeHeight) -> u32 {
        (height.0 % committee.members.len() as u64) as u32
    }
}
