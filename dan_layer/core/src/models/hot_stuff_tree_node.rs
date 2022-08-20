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
use tari_crypto::hash::blake2::Blake256;
use tari_dan_engine::state::models::StateRoot;

use crate::{
    models::{payload::PayloadId, Payload, QuorumCertificate, TreeNodeHash},
    services::infrastructure_services::NodeAddressable,
};

#[derive(Debug, Clone)]
pub struct HotStuffTreeNode<TAddr: NodeAddressable> {
    hash: TreeNodeHash,
    parent: TreeNodeHash,
    payload: PayloadId,
    shard: u32,
    height: u32,
    payload_height: u32,
    involved_shards: Vec<u32>,
    epoch: u32,
    leader: TAddr,
    justify: QuorumCertificate,
}

impl<TAddr: NodeAddressable> HotStuffTreeNode<TAddr> {
    pub fn new(
        parent: TreeNodeHash,
        payload: Option<TPayload>,
        height: u32,
        shard: u32,
        leader: TAddr,
        involved_shards: Vec<u32>,
        epoch: u32,
        justify: QuorumCertificate,
    ) -> Self {
        let mut s = HotStuffTreeNode {
            parent,
            payload,
            hash: TreeNodeHash::zero(),
            leader,
            involved_shards,
            shard,
            epoch,
            height,
            justify,
        };
        s.hash = s.calculate_hash();
        s
    }

    pub fn genesis() -> Self {
        let mut s = Self {
            parent: TreeNodeHash::zero(),
            payload: None,
            hash: TreeNodeHash::zero(),
            shard: 0,
            involved_shards: vec![],
            leader: TAddr::zero(),
            height: 0,
            epoch: 0,
            justify: QuorumCertificate::genesis(),
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
            .chain(self.leader.as_bytes())
            .chain((self.involved_shards.len() as u32).to_le_bytes())
            .chain(self.involved_shards.iter().fold(Vec::new(), |mut acc, shard| {
                acc.extend_from_slice(&shard.to_le_bytes());
                acc
            }));

        if let Some(p) = &self.payload {
            let hash = p.consensus_hash();
            result = result.chain((hash.len() as u32).to_le_bytes()).chain(hash);
        } else {
            result = result.chain(0u32.to_le_bytes())
        }
        let result = result.finalize_fixed();
        result.into()
    }

    pub fn hash(&self) -> &TreeNodeHash {
        &self.hash
    }

    pub fn leader(&self) -> &TAddr {
        &self.leader
    }

    pub fn parent(&self) -> &TreeNodeHash {
        &self.parent
    }

    pub fn payload(&self) -> Option<&TPayload> {
        self.payload.as_ref()
    }

    pub fn justify(&self) -> &QuorumCertificate {
        &self.justify
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

impl<TPayload: Payload, TAddr: NodeAddressable> PartialEq for HotStuffTreeNode<TPayload, TAddr> {
    fn eq(&self, other: &Self) -> bool {
        self.hash.eq(&other.hash)
    }
}
