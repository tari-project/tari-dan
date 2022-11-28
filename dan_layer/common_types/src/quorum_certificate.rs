//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use digest::Digest;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_crypto::hash::blake2::Blake256;

use crate::{Epoch, NodeHeight, PayloadId, ShardId, ShardVote, TreeNodeHash, ValidatorMetadata};

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
            .chain(self.shard.as_bytes())
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
