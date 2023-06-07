//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::borrow::Borrow;

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_core::ValidatorNodeBmtHasherBlake256;
use tari_engine_types::{
    commit_result::RejectReason,
    hashing::{hasher, EngineHashDomainLabel},
};
use tari_mmr::MergedBalancedBinaryMerkleProof;

use crate::{
    Epoch, NodeAddressable, NodeHeight, PayloadId, ShardId, ShardPledgeCollection, TreeNodeHash, ValidatorMetadata,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum QuorumDecision {
    Accept,
    Reject(QuorumRejectReason),
}

impl QuorumDecision {
    pub fn is_reject(&self) -> bool {
        matches!(self, QuorumDecision::Reject(_))
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum QuorumRejectReason {
    ShardNotPledged,
    ExecutionFailure,
    PreviousQcRejection,
    ShardPledgedToAnotherPayload,
    ShardRejected,
    FeeTransactionFailed,
    FeeInitializationFailed,
}

impl QuorumRejectReason {
    pub fn as_u8(&self) -> u8 {
        match self {
            QuorumRejectReason::ShardNotPledged => 1,
            QuorumRejectReason::ExecutionFailure => 2,
            QuorumRejectReason::PreviousQcRejection => 3,
            QuorumRejectReason::ShardPledgedToAnotherPayload => 4,
            QuorumRejectReason::ShardRejected => 5,
            QuorumRejectReason::FeeTransactionFailed => 6,
            QuorumRejectReason::FeeInitializationFailed => 7,
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
            5 => Ok(QuorumDecision::Reject(QuorumRejectReason::ShardRejected)),
            6 => Ok(QuorumDecision::Reject(QuorumRejectReason::FeeTransactionFailed)),
            7 => Ok(QuorumDecision::Reject(QuorumRejectReason::FeeInitializationFailed)),
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
            RejectReason::ShardRejected(_) => QuorumRejectReason::ShardRejected,
            RejectReason::FeeTransactionFailed => QuorumRejectReason::FeeTransactionFailed,
            RejectReason::FeesNotPaid(_) => QuorumRejectReason::FeeTransactionFailed,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuorumCertificate<TAddr> {
    payload_id: PayloadId,
    payload_height: NodeHeight,
    // Cache the node hash
    local_node_hash: TreeNodeHash,
    // cache the node height
    local_node_height: NodeHeight,
    shard: ShardId,
    epoch: Epoch,
    proposed_by: TAddr,
    decision: QuorumDecision,
    all_shard_pledges: ShardPledgeCollection,
    validators_metadata: Vec<ValidatorMetadata>,
    merged_proof: Option<MergedBalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>>,
    leaf_hashes: Vec<FixedHash>,
}

impl<TAddr: NodeAddressable> QuorumCertificate<TAddr> {
    pub fn new(
        payload: PayloadId,
        payload_height: NodeHeight,
        local_node_hash: TreeNodeHash,
        local_node_height: NodeHeight,
        shard: ShardId,
        epoch: Epoch,
        proposed_by: TAddr,
        decision: QuorumDecision,
        all_shard_pledges: ShardPledgeCollection,
        validators_metadata: Vec<ValidatorMetadata>,
        merged_proof: Option<MergedBalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>>,
        mut leaf_hashes: Vec<FixedHash>,
    ) -> Self {
        leaf_hashes.sort();
        Self {
            payload_id: payload,
            payload_height,
            local_node_hash,
            local_node_height,
            shard,
            epoch,
            proposed_by,
            decision,
            all_shard_pledges,
            validators_metadata,
            merged_proof,
            leaf_hashes,
        }
    }

    pub fn genesis(epoch: Epoch, payload_id: PayloadId, shard_id: ShardId, proposed_by: TAddr) -> Self {
        Self {
            payload_id,
            payload_height: NodeHeight(0),
            local_node_hash: TreeNodeHash::zero(),
            local_node_height: NodeHeight(0),
            shard: shard_id,
            epoch,
            proposed_by,
            decision: QuorumDecision::Accept,
            all_shard_pledges: ShardPledgeCollection::empty(),
            validators_metadata: vec![],
            merged_proof: None,
            leaf_hashes: vec![],
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

    pub fn proposed_by(&self) -> &TAddr {
        &self.proposed_by
    }

    pub fn merged_proof(&self) -> Option<&MergedBalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>> {
        self.merged_proof.as_ref()
    }

    pub fn leaf_hashes(&self) -> &[FixedHash] {
        &self.leaf_hashes
    }

    pub fn validators_metadata(&self) -> &[ValidatorMetadata] {
        self.validators_metadata.as_slice()
    }

    pub fn set_payload_id(&mut self, payload_id: PayloadId) {
        self.payload_id = payload_id;
    }

    pub fn to_hash(&self) -> FixedHash {
        hasher(EngineHashDomainLabel::QuorumCertificate)
            .chain(&self.local_node_hash)
            .chain(&self.local_node_height)
            .chain(&self.shard)
            .chain(&(self.validators_metadata.len() as u64))
            .chain(&self.merged_proof)
            .chain(&self.leaf_hashes)
            // TODO: add all fields
            // .chain(&self.validators_metadata)
            .result()
            .into_array()
            .into()
    }

    pub fn payload_id(&self) -> PayloadId {
        self.payload_id
    }

    pub fn payload_height(&self) -> NodeHeight {
        self.payload_height
    }

    /// Tree node hash
    pub fn node_hash(&self) -> TreeNodeHash {
        self.local_node_hash
    }

    pub fn node_height(&self) -> NodeHeight {
        self.local_node_height
    }

    pub fn decision(&self) -> &QuorumDecision {
        &self.decision
    }

    pub fn all_shard_pledges(&self) -> &ShardPledgeCollection {
        &self.all_shard_pledges
    }
}
