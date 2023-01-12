//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::borrow::Borrow;

use borsh::BorshSerialize;
use digest::Digest;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_crypto::hash::blake2::Blake256;
use tari_engine_types::commit_result::RejectReason;

use crate::{Epoch, NodeHeight, PayloadId, ShardId, ShardPledge, TreeNodeHash, ValidatorMetadata};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, BorshSerialize)]
pub enum QuorumDecision {
    Accept,
    Reject(QuorumRejectReason),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, BorshSerialize)]
pub enum QuorumRejectReason {
    ShardNotPledged,
    ExecutionFailure,
    PreviousQcRejection,
    ShardPledgedToAnotherPayload,
}

impl QuorumRejectReason {
    pub fn as_u8(&self) -> u8 {
        match self {
            QuorumRejectReason::ShardNotPledged => 1,
            QuorumRejectReason::ExecutionFailure => 2,
            QuorumRejectReason::PreviousQcRejection => 3,
            QuorumRejectReason::ShardPledgedToAnotherPayload => 4,
        }
    }
}

impl QuorumDecision {
    pub fn as_u8(&self) -> u8 {
        match self {
            QuorumDecision::Accept => 0,
            QuorumDecision::Reject(reason) => reason.as_u8(),
        }
    }

    pub fn from_u8(v: u8) -> Result<Self, anyhow::Error> {
        match v {
            0 => Ok(QuorumDecision::Accept),
            1 => Ok(QuorumDecision::Reject(QuorumRejectReason::ShardNotPledged)),
            2 => Ok(QuorumDecision::Reject(QuorumRejectReason::ExecutionFailure)),
            3 => Ok(QuorumDecision::Reject(QuorumRejectReason::PreviousQcRejection)),
            4 => Ok(QuorumDecision::Reject(QuorumRejectReason::ShardPledgedToAnotherPayload)),
            // TODO: Add error type
            _ => Err(anyhow::anyhow!("Invalid QuorumDecision")),
        }
    }
}

impl<T: Borrow<RejectReason>> From<T> for QuorumRejectReason {
    fn from(reason: T) -> Self {
        match reason.borrow() {
            RejectReason::ShardsNotPledged(_) => QuorumRejectReason::ShardNotPledged,
            RejectReason::ExecutionFailure(_) => QuorumRejectReason::ExecutionFailure,
            RejectReason::PreviousQcRejection => QuorumRejectReason::PreviousQcRejection,
            RejectReason::ShardPledgedToAnotherPayload(_) => QuorumRejectReason::ShardPledgedToAnotherPayload,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, BorshSerialize)]
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
    all_shard_pledges: Vec<ShardPledge>,
    validators_metadata: Vec<ValidatorMetadata>,
}

impl QuorumCertificate {
    pub fn set_node(&mut self, node_hash: TreeNodeHash, node_height: NodeHeight) -> &mut Self {
        self.local_node_hash = node_hash;
        self.local_node_height = node_height;
        self
    }
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
        all_shard_pledges: Vec<ShardPledge>,
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
            all_shard_pledges,
            validators_metadata,
        }
    }

    pub fn genesis(epoch: Epoch, payload_id: PayloadId, shard_id: ShardId) -> Self {
        Self {
            payload_id,
            payload_height: NodeHeight(0),
            local_node_hash: TreeNodeHash::zero(),
            local_node_height: NodeHeight(0),
            shard: shard_id,
            epoch,
            decision: QuorumDecision::Accept,
            all_shard_pledges: vec![],
            validators_metadata: vec![],
        }
    }

    pub fn is_genesis(&self) -> bool {
        self.local_node_hash.is_zero()
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

    pub fn set_payload_id(&mut self, payload_id: PayloadId) {
        self.payload_id = payload_id;
    }

    pub fn to_hash(&self) -> FixedHash {
        let mut result = Blake256::new()
            .chain(self.local_node_hash.as_bytes())
            .chain(self.local_node_height.to_le_bytes())
            .chain(self.shard.as_bytes())
            .chain((self.validators_metadata.len() as u64).to_le_bytes());
        // TODO: add all fields

        for vm in &self.validators_metadata {
            result = result.chain(vm.encode_merkle_proof());
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
    pub fn node_hash(&self) -> TreeNodeHash {
        self.local_node_hash
    }

    pub fn node_height(&self) -> NodeHeight {
        self.local_node_height
    }

    pub fn decision(&self) -> &QuorumDecision {
        &self.decision
    }

    pub fn all_shard_pledges(&self) -> &[ShardPledge] {
        &self.all_shard_pledges
    }
}
