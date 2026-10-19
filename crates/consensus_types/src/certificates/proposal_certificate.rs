//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup, hashing::quorum_certificate_id_hasher};
use tari_sidechain::QuorumDecision;

use crate::{
    bookkeeping::{HighPc, LeafBlock},
    ids::{BlockId, PcId},
    validator_signature::ValidatorSignatureBytes,
};

#[derive(Debug, Clone, Deserialize, Serialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ProposalCertificate {
    #[n(0)]
    height: NodeHeight,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::hex")]
    // FixedHash is from tari_common_types (external) — bridge through serde.
    #[n(1)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    header_hash: FixedHash,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(2)]
    parent_id: BlockId,
    #[n(3)]
    epoch: Epoch,
    #[n(4)]
    shard_group: ShardGroup,
    #[n(5)]
    signatures: Vec<ValidatorSignatureBytes>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    // QuorumDecision is from tari_sidechain (external) — bridge through serde.
    #[n(6)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    decision: QuorumDecision,
}

impl ProposalCertificate {
    pub fn new(
        header_hash: FixedHash,
        parent_id: BlockId,
        height: NodeHeight,
        epoch: Epoch,
        shard_group: ShardGroup,
        signatures: Vec<ValidatorSignatureBytes>,
        decision: QuorumDecision,
    ) -> Self {
        Self {
            header_hash,
            parent_id,
            height,
            epoch,
            shard_group,
            signatures,
            decision,
        }
    }

    pub fn genesis(epoch: Epoch, shard_group: ShardGroup) -> Self {
        Self {
            header_hash: FixedHash::zero(),
            parent_id: BlockId::zero(),
            height: NodeHeight::zero(),
            epoch,
            shard_group,
            signatures: vec![],
            decision: QuorumDecision::Accept,
        }
    }

    /// Returns the hash of the QC. This is used to identify the QC and not for any secure purposes.
    /// However, we implement a secure hash (as opposed to a cheaper, non-collision-resistant hash e.g. siphash) to
    /// avoid any collision issues e.g. storage keys.
    pub fn calculate_id(&self) -> PcId {
        // We use the same fields as tari_sidechain::QuorumCertificate. Since should calculate a consistent ID between
        // shards. Although, worth noting that in the current protocol, this does not matter because the foreign
        // QC id only needs to be consistent within a shard group. This may change in the future.
        Self::calculate_id_from_parts(&self.header_hash, &self.parent_id, &self.signatures, &self.decision)
    }

    pub fn calculate_id_from_parts(
        header_hash: &FixedHash,
        parent_id: &BlockId,
        signatures: &[ValidatorSignatureBytes],
        decision: &QuorumDecision,
    ) -> PcId {
        quorum_certificate_id_hasher()
            .chain(header_hash)
            .chain(parent_id)
            .chain(signatures)
            .chain(decision)
            .finalize_into_array()
            .into()
    }
}

impl ProposalCertificate {
    pub fn justifies_zero_block(&self) -> bool {
        self.header_hash.as_slice().iter().all(|b| *b == 0)
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.shard_group
    }

    pub fn signatures(&self) -> &[ValidatorSignatureBytes] {
        &self.signatures
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn decision(&self) -> QuorumDecision {
        self.decision
    }

    pub fn calculate_block_id(&self) -> BlockId {
        BlockId::from_parent_and_header_hash(&self.parent_id, &self.header_hash)
    }

    pub fn header_hash(&self) -> &FixedHash {
        &self.header_hash
    }

    pub fn parent_id(&self) -> &BlockId {
        &self.parent_id
    }

    pub fn as_high_pc(&self) -> HighPc {
        HighPc {
            block_id: self.calculate_block_id(),
            block_height: self.height,
            epoch: self.epoch,
            qc_id: self.calculate_id(),
        }
    }

    pub fn as_leaf_block(&self) -> LeafBlock {
        LeafBlock {
            block_id: self.calculate_block_id(),
            height: self.height,
            epoch: self.epoch,
            shard_group: self.shard_group,
        }
    }
}

impl Display for ProposalCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ProposalCertificate(block: {} {}, qc_id: {}, epoch: {}, {} signatures)",
            self.height,
            self.calculate_block_id(),
            self.calculate_id(),
            self.epoch,
            self.signatures.len()
        )
    }
}
