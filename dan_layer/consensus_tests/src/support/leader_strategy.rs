//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::LeaderStrategy;
use tari_dan_common_types::{committee::Committee, NodeAddressable, NodeHeight};

#[derive(Debug, Clone, Copy, Default)]
pub struct RoundRobinLeaderStrategy;
impl RoundRobinLeaderStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for RoundRobinLeaderStrategy {
    fn calculate_leader(&self, committee: &Committee<TAddr>, height: NodeHeight) -> u32 {
        (height.as_u64() % committee.members.len() as u64) as u32
    }
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::PublicKey;

    use super::*;

    fn new_member(seed: &'static str) -> (String, PublicKey) {
        (seed.to_string(), PublicKey::new_generator(seed).unwrap())
    }

    #[test]
    fn it_selects_leader_based_on_height() {
        let strategy = RoundRobinLeaderStrategy::new();
        let committee = Committee::from_iter([new_member("1"), new_member("2"), new_member("3")]);

        let (addr, _) = strategy.get_leader(&committee, NodeHeight(1));
        assert_eq!(addr, "2");
        let (addr, _) = strategy.get_leader(&committee, NodeHeight(2));
        assert_eq!(addr, "3");
        let (addr, _) = strategy.get_leader(&committee, NodeHeight(3));
        assert_eq!(addr, "1");
    }
}
