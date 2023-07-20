//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use tari_consensus::traits::LeaderStrategy;
use tari_dan_common_types::{committee::Committee, NodeAddressable, NodeHeight};
use tari_dan_storage::consensus_models::BlockId;

use crate::support::address::TestAddress;

pub struct AlwaysFirstLeader;

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for AlwaysFirstLeader {
    fn calculate_leader(&self, _committee: &Committee<TAddr>, _block: &BlockId, _height: NodeHeight) -> u32 {
        0
    }
}

#[derive(Debug, Clone)]
pub struct RandomDeterministicLeaderStrategy;

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for RandomDeterministicLeaderStrategy {
    fn calculate_leader(&self, committee: &Committee<TAddr>, block: &BlockId, height: NodeHeight) -> u32 {
        // TODO: Maybe Committee should not be able to be constructed with an empty committee
        assert!(!committee.is_empty(), "Committee was empty in calculate_leader");
        let hash = block.hash().as_slice();
        let hash = u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]]);
        let first = hash % committee.members.len() as u32;
        (first + height.0 as u32) % committee.members.len() as u32
    }
}

#[derive(Debug, Clone)]
pub struct SelectedIndexLeaderStrategy(Arc<AtomicU32>);

impl SelectedIndexLeaderStrategy {
    pub fn new(index: u32) -> Self {
        Self(Arc::new(AtomicU32::new(index)))
    }

    #[allow(dead_code)]
    pub fn set_index(&self, index: u32) {
        self.0.store(index, Ordering::SeqCst);
    }
}

impl LeaderStrategy<TestAddress> for SelectedIndexLeaderStrategy {
    fn calculate_leader(&self, committee: &Committee<TestAddress>, _block: &BlockId, _height: NodeHeight) -> u32 {
        let index = self.0.load(Ordering::SeqCst);
        assert!(
            (index as usize) < committee.len(),
            "SelectedIndexLeaderStrategy index out of bounds index={} len={}",
            index,
            committee.len()
        );
        index
    }
}
