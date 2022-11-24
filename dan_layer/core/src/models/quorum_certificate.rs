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

use digest::Digest;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_crypto::hash::blake2::Blake256;
use tari_dan_common_types::{Epoch, PayloadId, ShardId};

use crate::models::{NodeHeight, ShardVote, TreeNodeHash, ValidatorMetadata};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum QuorumDecision {
    Accept,
    Reject(QuorumRejectReason),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum QuorumRejectReason {
    ShardNotPledged,
    ExecutionFailure,
}

impl QuorumDecision {
    pub fn as_u8(&self) -> u8 {
        match self {
            QuorumDecision::Accept => 0,
            QuorumDecision::Reject(reason) => match reason {
                QuorumRejectReason::ShardNotPledged => 1,
                QuorumRejectReason::ExecutionFailure => 2,
            },
        }
    }

    pub fn from_u8(v: u8) -> Result<Self, anyhow::Error> {
        match v {
            0 => Ok(QuorumDecision::Accept),
            1 => Ok(QuorumDecision::Reject(QuorumRejectReason::ShardNotPledged)),
            2 => Ok(QuorumDecision::Reject(QuorumRejectReason::ExecutionFailure)),
            // TODO: Add error type
            _ => Err(anyhow::anyhow!("Invalid QuorumDecision")),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuorumCertificate {
    payload_id: PayloadId,
    payload_height: NodeHeight,
    // Cache the node hash
    local_node_hash: TreeNodeHash,
    // cache the node height
    local_node_height: NodeHeight,
    shard: ShardId,
    epoch: Epoch,
    decision: QuorumDecision,
    all_shard_nodes: Vec<ShardVote>,
    validators_metadata: Vec<ValidatorMetadata>,
}

impl QuorumCertificate {
    pub fn new(
        payload: PayloadId,
        payload_height: NodeHeight,
        local_node_hash: TreeNodeHash,
        local_node_height: NodeHeight,
        shard: ShardId,
        epoch: Epoch,
        decision: QuorumDecision,
        all_shard_nodes: Vec<ShardVote>,
        validators_metadata: Vec<ValidatorMetadata>,
    ) -> Self {
        Self {
            payload_id: payload,
            payload_height,
            local_node_hash,
            local_node_height,
            shard,
            epoch,
            decision,
            all_shard_nodes,
            validators_metadata,
        }
    }

    pub fn genesis(epoch: Epoch) -> Self {
        Self {
            payload_id: PayloadId::zero(),
            payload_height: NodeHeight(0),
            local_node_hash: TreeNodeHash::zero(),
            local_node_height: NodeHeight(0),
            shard: ShardId::zero(),
            epoch,
            decision: QuorumDecision::Accept,
            all_shard_nodes: vec![],
            validators_metadata: vec![],
        }
    }

    pub fn shard(&self) -> ShardId {
        self.shard
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn validators_metadata(&self) -> &[ValidatorMetadata] {
        self.validators_metadata.as_slice()
    }

    pub fn to_hash(&self) -> FixedHash {
        let mut result = Blake256::new()
            .chain(self.local_node_hash.as_bytes())
            .chain(self.local_node_height.to_le_bytes())
            .chain(self.shard.to_le_bytes())
            .chain((self.validators_metadata.len() as u64).to_le_bytes());
        // TODO: add all fields

        for vm in &self.validators_metadata {
            result = result.chain(vm.to_bytes());
        }
        // result = result.chain((self.involved_shards.len() as u32).to_le_bytes());
        // for shard in &self.involved_shards {
        //     result = result.chain((*shard).to_le_bytes());
        // }
        result.finalize().into()
    }

    pub fn payload_id(&self) -> PayloadId {
        self.payload_id
    }

    pub fn payload_height(&self) -> NodeHeight {
        self.payload_height
    }

    /// The locally stable hash of the node
    pub fn local_node_hash(&self) -> TreeNodeHash {
        self.local_node_hash
    }

    pub fn local_node_height(&self) -> NodeHeight {
        self.local_node_height
    }

    pub fn decision(&self) -> &QuorumDecision {
        &self.decision
    }

    pub fn all_shard_nodes(&self) -> &[ShardVote] {
        &self.all_shard_nodes
    }
}
