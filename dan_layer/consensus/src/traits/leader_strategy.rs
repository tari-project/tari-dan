//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_dan_common_types::{committee::Committee, NodeAddressable};
use tari_dan_storage::consensus_models::BlockId;

pub trait LeaderStrategy<TAddr: NodeAddressable> {
    fn calculate_leader(&self, committee: &Committee<TAddr>, block: &BlockId, round: u32) -> u32;

    fn is_leader(&self, validator_addr: &TAddr, committee: &Committee<TAddr>, block: &BlockId, round: u32) -> bool {
        let position = self.calculate_leader(committee, block, round);
        if let Some(vn) = committee.members.get(position as usize) {
            vn == validator_addr
        } else {
            false
        }
    }

    fn get_leader<'b>(&self, committee: &'b Committee<TAddr>, block: &BlockId, round: u32) -> &'b TAddr {
        let index = self.calculate_leader(committee, block, round);
        committee.members.get(index as usize).unwrap()
    }
}

pub struct AlwaysFirstLeader;

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for AlwaysFirstLeader {
    fn calculate_leader(&self, _committee: &Committee<TAddr>, _block: &BlockId, _round: u32) -> u32 {
        0
    }
}

pub struct RotatingLeader;

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for RotatingLeader {
    fn calculate_leader(&self, committee: &Committee<TAddr>, _block: &BlockId, round: u32) -> u32 {
        round % (committee.len() as u32)
    }
}

pub struct RandomDeterministicLeaderStrategy;

impl<TAddr: NodeAddressable> LeaderStrategy<TAddr> for RandomDeterministicLeaderStrategy {
    fn calculate_leader(&self, committee: &Committee<TAddr>, block: &BlockId, round: u32) -> u32 {
        // TODO: Maybe Committee should not be able to be constructed with an empty committee
        assert!(!committee.is_empty(), "Committee was empty in calculate_leader");
        let hash = block.hash().as_slice();
        let hash = u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]]);
        let first = hash % committee.members.len() as u32;
        (first + round) % committee.members.len() as u32
    }
}
