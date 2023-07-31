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
    fn calculate_leader(&self, _committee: &Committee<TAddr>, _height: NodeHeight) -> u32 {
        0
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
    fn calculate_leader(&self, committee: &Committee<TestAddress>, _height: NodeHeight) -> u32 {
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
