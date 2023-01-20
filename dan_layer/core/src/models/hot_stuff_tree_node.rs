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

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{
    Epoch,
    NodeAddressable,
    NodeHeight,
    ObjectPledge,
    PayloadId,
    QuorumCertificate,
    ShardId,
    TreeNodeHash,
};
use tari_engine_types::hashing::hasher;

use super::Payload;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotStuffTreeNode<TAddr, TPayload> {
    hash: TreeNodeHash,
    parent: TreeNodeHash,
    shard: ShardId,
    height: NodeHeight,
    /// The payload that the node is proposing
    payload_id: PayloadId,
    payload: Option<TPayload>,
    /// How far in the consensus this payload is. It should be 4 in order to be committed.
    payload_height: NodeHeight,
    local_pledge: Option<ObjectPledge>,
    epoch: Epoch,
    justify: QuorumCertificate,
    // Mostly used for debugging
    proposed_by: TAddr,
}

impl<TAddr: NodeAddressable, TPayload: Payload> HotStuffTreeNode<TAddr, TPayload> {
    pub fn new(
        parent: TreeNodeHash,
        shard: ShardId,
        height: NodeHeight,
        payload_id: PayloadId,
        payload: Option<TPayload>,
        payload_height: NodeHeight,
        local_pledge: Option<ObjectPledge>,
        epoch: Epoch,
        proposed_by: TAddr,
        justify: QuorumCertificate,
    ) -> Self {
        let mut s = HotStuffTreeNode {
            hash: TreeNodeHash::zero(),
            parent,
            shard,
            payload_id,
            payload,
            epoch,
            height,
            justify,
            payload_height,
            local_pledge,
            proposed_by,
        };
        s.hash = s.calculate_hash();
        s
    }

    pub fn genesis(
        epoch: Epoch,
        payload_id: PayloadId,
        shard_id: ShardId,
        proposed_by: TAddr,
        local_pledge: Option<ObjectPledge>,
    ) -> Self {
        Self {
            // A genesis node is special, and therefore has a zero hash to indicate this
            hash: TreeNodeHash::zero(),
            parent: TreeNodeHash::zero(),
            shard: shard_id,
            height: NodeHeight(0),
            payload_id,
            payload: None,
            payload_height: NodeHeight(0),
            local_pledge,
            epoch,
            justify: QuorumCertificate::genesis(epoch, payload_id, shard_id),
            proposed_by,
        }
    }

    pub fn is_genesis(&self) -> bool {
        self.hash.is_zero()
    }

    pub fn calculate_hash(&self) -> TreeNodeHash {
        hasher("HotStuffTreeNode")
            .chain(&self.parent)
            .chain(&self.epoch)
            .chain(&self.height)
            .chain(&self.justify)
            .chain(&self.shard)
            .chain(&self.payload_id)
            .chain(&self.payload_height)
            .chain(&self.proposed_by.as_bytes())
            .chain(&self.local_pledge)
            .result()
            .into_array()
            .into()
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

    pub fn payload_id(&self) -> PayloadId {
        self.payload_id
    }

    pub fn payload(&self) -> Option<&TPayload> {
        self.payload.as_ref()
    }

    /// The payload height maps (modulo 4) onto the current phase of hotstuff (Prepare, PreCommit, Commit,
    /// Decide).
    pub fn payload_height(&self) -> NodeHeight {
        // NodeHeight(self.payload_height.as_u64() % 4)
        self.payload_height
    }

    /// Returns the hotstuff phase corresponding to the payload height
    pub fn payload_phase(&self) -> HotstuffPhase {
        self.payload_height.into()
    }

    /// The quorum certificate for this node
    pub fn justify(&self) -> &QuorumCertificate {
        &self.justify
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn shard(&self) -> ShardId {
        self.shard
    }

    /// The height of the chain for the shard
    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn local_pledge(&self) -> Option<&ObjectPledge> {
        self.local_pledge.as_ref()
    }
}

impl<TAddr: NodeAddressable, TPayload: Payload> PartialEq for HotStuffTreeNode<TAddr, TPayload> {
    fn eq(&self, other: &Self) -> bool {
        self.hash.eq(&other.hash)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotstuffPhase {
    Genesis,
    Prepare,
    PreCommit,
    Commit,
    Decide,
}

impl From<NodeHeight> for HotstuffPhase {
    fn from(value: NodeHeight) -> Self {
        match value.as_u64() % 5 {
            0 => HotstuffPhase::Genesis,
            1 => HotstuffPhase::Prepare,
            2 => HotstuffPhase::PreCommit,
            3 => HotstuffPhase::Commit,
            4 => HotstuffPhase::Decide,
            _ => unreachable!(),
        }
    }
}
