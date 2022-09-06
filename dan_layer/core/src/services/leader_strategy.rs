use tari_dan_common_types::{PayloadId, ShardId};

use crate::{models::Committee, services::infrastructure_services::NodeAddressable};

pub trait LeaderStrategy<TAddr: NodeAddressable, TPayload> {
    fn calculate_leader(&self, committee: &Committee<TAddr>, payload: PayloadId, shard: ShardId, round: u32) -> u32;
    fn is_leader(
        &self,
        node: &TAddr,
        committee: &Committee<TAddr>,
        payload: PayloadId,
        shard: ShardId,
        round: u32,
    ) -> bool {
        let position = self.calculate_leader(committee, payload, shard, round);
        if let Some(index) = committee.members.iter().position(|m| m == node) {
            position == index as u32
        } else {
            false
        }
    }

    fn get_leader<'a, 'b>(
        &'a self,
        committee: &'b Committee<TAddr>,
        payload: PayloadId,
        shard: ShardId,
        round: u32,
    ) -> &'b TAddr {
        let index = self.calculate_leader(committee, payload, shard, round);
        committee.members.get(index as usize).unwrap()
    }
}

pub struct AlwaysFirstLeader {}

impl<TAddr: NodeAddressable, TPayload> LeaderStrategy<TAddr, TPayload> for AlwaysFirstLeader {
    fn calculate_leader(&self, committee: &Committee<TAddr>, payload: PayloadId, shard: ShardId, round: u32) -> u32 {
        0
    }
}
