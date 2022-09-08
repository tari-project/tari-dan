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

use digest::{Digest, FixedOutput};
use tari_common_types::types::FixedHash;
use tari_crypto::hash::blake2::Blake256;
use tari_dan_common_types::ShardId;

use crate::models::{ObjectPledge, QuorumDecision, TreeNodeHash, ValidatorSignature};

#[derive(Debug, Clone)]
pub struct VoteMessage {
    local_node_hash: TreeNodeHash,
    shard: ShardId,
    decision: QuorumDecision,
    other_shard_nodes: Vec<(ShardId, TreeNodeHash, Vec<ObjectPledge>)>,
    signature: Option<ValidatorSignature>,
}

impl VoteMessage {
    pub fn new(
        local_node_hash: TreeNodeHash,
        shard: ShardId,
        decision: QuorumDecision,
        mut other_shard_nodes: Vec<(ShardId, TreeNodeHash, Vec<ObjectPledge>)>,
    ) -> Self {
        other_shard_nodes.sort_by(|a, b| a.0.cmp(&b.0));

        Self {
            local_node_hash,
            shard,
            decision,
            other_shard_nodes,
            signature: None,
        }
    }

    pub fn sign(&mut self) {
        // TODO: better signature
        self.signature = Some(ValidatorSignature::from_bytes(&[9u8; 32]))
    }

    pub fn signature(&self) -> &ValidatorSignature {
        self.signature.as_ref().unwrap()
    }

    pub fn get_all_nodes_hash(&self) -> FixedHash {
        let mut result = Blake256::new().chain(&[self.decision.as_u8()]);
        // data must already be sorted
        for (shard, hash, pledges) in &self.other_shard_nodes {
            result = result
                .chain(shard.0)
                .chain(hash.as_bytes())
                .chain((pledges.len() as u32).to_le_bytes());

            for p in pledges {
                result = result.chain(p.object_id.0)
            }
        }
        result.finalize_fixed().into()
    }

    pub fn local_node_hash(&self) -> TreeNodeHash {
        self.local_node_hash
    }

    pub fn shard(&self) -> ShardId {
        self.shard
    }

    pub fn decision(&self) -> QuorumDecision {
        self.decision
    }

    pub fn other_shard_nodes(&self) -> &Vec<(ShardId, TreeNodeHash, Vec<ObjectPledge>)> {
        &self.other_shard_nodes
    }
}
