//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::LeaderStrategy;
use tari_dan_common_types::{committee::Committee, NodeAddressable};
use tari_dan_storage::consensus_models::BlockId;

#[derive(Debug, Clone, Copy, Default)]
pub struct RandomDeterministicLeaderStrategy;

impl RandomDeterministicLeaderStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for RandomDeterministicLeaderStrategy {
    fn calculate_leader(&self, committee: &Committee<TAddr>, block: &BlockId, round: u32) -> u32 {
        // TODO: Maybe Committee should not be able to be constructed with an empty committee
        assert!(!committee.is_empty(), "Committee was empty in calculate_leader");
        let block_id = block.as_bytes();
        let val = u32::from_le_bytes([block_id[0], block_id[1], block_id[2], block_id[3]]);
        let first = val % committee.members.len() as u32;
        (first + round) % committee.members.len() as u32
    }
}
