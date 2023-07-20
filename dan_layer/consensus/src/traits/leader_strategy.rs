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

use tari_dan_common_types::{committee::Committee, NodeAddressable, NodeHeight};
use tari_dan_storage::consensus_models::BlockId;

pub trait LeaderStrategy<TAddr: NodeAddressable> {
    fn calculate_leader(&self, committee: &Committee<TAddr>, block: &BlockId, height: NodeHeight) -> u32;

    fn is_leader(
        &self,
        validator_addr: &TAddr,
        committee: &Committee<TAddr>,
        block: &BlockId,
        height: NodeHeight,
    ) -> bool {
        let position = self.calculate_leader(committee, block, height);
        if let Some(vn) = committee.members.get(position as usize) {
            vn == validator_addr
        } else {
            false
        }
    }

    fn is_leader_for_next_block(
        &self,
        validator_addr: &TAddr,
        committee: &Committee<TAddr>,
        block: &BlockId,
        height: NodeHeight,
    ) -> bool {
        self.is_leader(validator_addr, committee, block, height + NodeHeight(1))
    }

    fn get_leader<'b>(&self, committee: &'b Committee<TAddr>, block: &BlockId, height: NodeHeight) -> &'b TAddr {
        let index = self.calculate_leader(committee, block, height);
        committee.members.get(index as usize).unwrap()
    }

    fn get_leader_for_next_block<'b>(
        &self,
        committee: &'b Committee<TAddr>,
        block: &BlockId,
        height: NodeHeight,
    ) -> &'b TAddr {
        self.get_leader(committee, block, height + NodeHeight(1))
    }
}
