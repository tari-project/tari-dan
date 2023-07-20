//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::LeaderStrategy;
use tari_dan_common_types::{committee::Committee, NodeAddressable, NodeHeight};
use tari_dan_storage::consensus_models::BlockId;

#[derive(Debug, Clone, Copy, Default)]
pub struct RandomDeterministicLeaderStrategy;

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for RandomDeterministicLeaderStrategy {
    fn calculate_leader(&self, committee: &Committee<TAddr>, block: &BlockId, height: NodeHeight) -> u32 {
        // TODO: Maybe Committee should not be able to be constructed with an empty committee
        assert!(!committee.is_empty(), "Committee was empty in calculate_leader");
        let block_id = block.as_bytes();
        let val = u32::from_le_bytes([block_id[0], block_id[1], block_id[2], block_id[3]]);
        let first = val % committee.members.len() as u32;
        (first + height.0 as u32) % committee.members.len() as u32
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RoundRobinLeaderStrategy;
impl RoundRobinLeaderStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for RoundRobinLeaderStrategy {
    fn calculate_leader(&self, committee: &Committee<TAddr>, _block: &BlockId, height: NodeHeight) -> u32 {
        (height.0 % committee.members.len() as u64) as u32
    }
}
