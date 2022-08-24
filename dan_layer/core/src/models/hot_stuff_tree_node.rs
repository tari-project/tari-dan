// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use digest::{Digest, FixedOutput};
use tari_common_types::types::FixedHash;
use tari_crypto::hash::blake2::Blake256;
use tari_dan_engine::state::models::StateRoot;

use crate::{
    models::{payload::PayloadId, Epoch, NodeHeight, ObjectPledge, Payload, QuorumCertificate, ShardId, TreeNodeHash},
    services::infrastructure_services::NodeAddressable,
};

#[derive(Debug, Clone)]
pub struct HotStuffTreeNode<TAddr: NodeAddressable> {
    hash: TreeNodeHash,
    parent: TreeNodeHash,
    shard: ShardId,
    height: NodeHeight,
    // The payload that the node is proposing
    payload: PayloadId,
    // How far in the consensus this payload is. It should be 4 in order to be committed.
    payload_height: NodeHeight,
    local_pledges: Vec<ObjectPledge>,
    epoch: Epoch,
    // Mostly used for debugging
    proposed_by: TAddr,
    justify: QuorumCertificate,
}

impl<TAddr: NodeAddressable> HotStuffTreeNode<TAddr> {
    pub fn new(
        parent: TreeNodeHash,
        shard: ShardId,
        height: NodeHeight,
        payload: PayloadId,
        payload_height: NodeHeight,
        local_pledges: Vec<ObjectPledge>,
        epoch: Epoch,
        proposed_by: TAddr,
        justify: QuorumCertificate,
    ) -> Self {
        let mut s = HotStuffTreeNode {
            hash: TreeNodeHash::zero(),
            parent,
            shard,
            payload,
            epoch,
            height,
            justify,
            payload_height,
            local_pledges,
            proposed_by,
        };
        s.hash = s.calculate_hash();
        s
    }

    pub fn genesis() -> Self {
        let mut s = Self {
            parent: TreeNodeHash::zero(),
            payload: PayloadId::zero(),
            payload_height: NodeHeight(0),
            hash: TreeNodeHash::zero(),
            shard: ShardId(0),
            height: NodeHeight(0),
            epoch: Epoch(0),
            proposed_by: TAddr::zero(),
            justify: QuorumCertificate::genesis(),
            local_pledges: vec![],
        };
        s.hash = s.calculate_hash();
        s
    }

    pub fn calculate_hash(&self) -> TreeNodeHash {
        let mut result = Blake256::new()
            .chain(self.parent.as_bytes())
            .chain(self.epoch.to_le_bytes())
            .chain(self.height.to_le_bytes())
            .chain(self.justify.as_bytes())
            .chain(self.shard.to_le_bytes())
            .chain(self.payload.as_slice())
            .chain(self.payload_height.to_le_bytes())
            .chain(self.proposed_by.as_bytes());
        // TODO: Add in other fields
        // .chain((self.local_pledges.len() as u32).to_le_bytes())
        // .chain(self.local_pledges.iter().fold(Vec::new(), |mut acc, substate| {
        //     acc.extend_from_slice(substate.as_bytes())
        // }));

        let result = result.finalize_fixed();
        result.into()
    }

    pub fn hash(&self) -> &TreeNodeHash {
        &self.hash
    }

    pub fn proposed_by(&self) -> &TAddr {
        &self.proposed_by
    }

    pub fn parent(&self) -> &TreeNodeHash {
        &self.parent
    }

    pub fn payload(&self) -> PayloadId {
        self.payload
    }

    pub fn payload_height(&self) -> NodeHeight {
        self.payload_height
    }

    pub fn justify(&self) -> &QuorumCertificate {
        &self.justify
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn shard(&self) -> ShardId {
        self.shard
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn local_pledges(&self) -> &[ObjectPledge] {
        self.local_pledges.as_slice()
    }
}

impl<TAddr: NodeAddressable> PartialEq for HotStuffTreeNode<TAddr> {
    fn eq(&self, other: &Self) -> bool {
        self.hash.eq(&other.hash)
    }
}
