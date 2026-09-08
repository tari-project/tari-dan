//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{CompressedPublicKey, FixedHash};
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup, VotePower};
use tari_sidechain::{SidechainBlockCommitProof, SidechainProofValidationError};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

/// A commit proof for a single committed block: a block header authenticated by the committed QC
/// chain of its shard group committee.
///
/// Verifying it yields a trusted state merkle root at a known height and epoch *without* trusting
/// the responding node's self-reported consensus state. This is the building block for verifiable
/// indexer reads: it lets a non-validator pin a quorum-signed state root, against which substate
/// inclusion/exclusion proofs can later be checked.
#[derive(Debug, Clone)]
pub struct CommittedBlockProof {
    proof: SidechainBlockCommitProof,
}

impl CommittedBlockProof {
    pub fn new(proof: SidechainBlockCommitProof) -> Self {
        Self { proof }
    }

    /// Decodes a proof from the CBOR encoding produced by [`CommittedBlockProof::to_bytes`]
    /// (matching `tari_bor::adapters::serde_bridge`, as used for [`super::EpochCheckpoint`]).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CommittedBlockProofError> {
        let proof =
            tari_bor::serde_codec::from_slice(bytes).map_err(|e| CommittedBlockProofError::Decode(e.to_string()))?;
        Ok(Self::new(proof))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        tari_bor::serde_codec::to_vec(&self.proof).expect("CommittedBlockProof serialization is infallible")
    }

    pub fn proof(&self) -> &SidechainBlockCommitProof {
        &self.proof
    }

    pub fn into_proof(self) -> SidechainBlockCommitProof {
        self.proof
    }

    pub fn epoch(&self) -> Epoch {
        Epoch(self.proof.header.epoch)
    }

    pub fn height(&self) -> NodeHeight {
        NodeHeight(self.proof.header.height)
    }

    pub fn state_merkle_root(&self) -> FixedHash {
        self.proof.header.state_merkle_root
    }

    /// The block id (header hash) of the committed block this proof attests to. Not part of the
    /// trust decision - used for diagnostics/audit.
    pub fn block_id(&self) -> FixedHash {
        self.proof.header.calculate_block_id()
    }

    /// The epoch hash the committee committed in this block's (quorum-signed) header. A verifier can
    /// cross-check it against the epoch hash it independently derives from the base layer
    /// (`EpochManagerReader::get_epoch_hash`) to detect epoch-boundary divergence.
    pub fn epoch_hash(&self) -> FixedHash {
        self.proof.header.epoch_hash
    }

    pub fn shard_group(&self) -> Result<ShardGroup, CommittedBlockProofError> {
        let sg = self.proof.header.shard_group;
        ShardGroup::new_checked(sg.start, sg.end_inclusive).ok_or(CommittedBlockProofError::InvalidShardGroup {
            start: sg.start,
            end_inclusive: sg.end_inclusive,
        })
    }

    /// Validates the commit proof against the shard group committee, returning the verified tip.
    ///
    /// `check_vn` must return the voting power of the given validator in the committee expected to
    /// have signed this proof's QCs (zero if it is not a member). The caller is responsible for
    /// looking up the committee for [`CommittedBlockProof::epoch`] /
    /// [`CommittedBlockProof::shard_group`] — verifying the proof against the wrong committee will
    /// (correctly) fail the quorum check.
    pub fn validate(
        &self,
        quorum_threshold: VotePower,
        check_vn: impl Fn(&RistrettoPublicKeyBytes) -> Result<VotePower, SidechainProofValidationError>,
    ) -> Result<VerifiedBlockTip, CommittedBlockProofError> {
        self.proof
            // TODO: Currently 1 VN = 1 vote power. When voting power becomes non-uniform, the
            //       sidechain proof library must validate accumulated power rather than membership.
            .validate_committed(quorum_threshold.value() as usize, &|pk: &CompressedPublicKey| {
                let pk_bytes = RistrettoPublicKeyBytes::from_bytes(pk.as_bytes())
                    .map_err(SidechainProofValidationError::internal_error)?;
                check_vn(&pk_bytes).map(|power| !power.is_zero())
            })?;

        Ok(VerifiedBlockTip {
            epoch: self.epoch(),
            shard_group: self.shard_group()?,
            height: self.height(),
            block_id: self.block_id(),
            epoch_hash: self.epoch_hash(),
            state_merkle_root: self.state_merkle_root(),
        })
    }
}

/// The trusted result of verifying a [`CommittedBlockProof`]: a quorum-signed state merkle root at
/// a known height within an epoch's shard group chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedBlockTip {
    pub epoch: Epoch,
    pub shard_group: ShardGroup,
    pub height: NodeHeight,
    /// The committed block's id (header hash). Diagnostics/audit only - not part of the trust key.
    pub block_id: FixedHash,
    /// The committee's quorum-signed epoch hash, for cross-checking against the verifier's own
    /// base-layer-derived epoch hash (epoch-boundary continuity).
    pub epoch_hash: FixedHash,
    pub state_merkle_root: FixedHash,
}

#[derive(Debug, thiserror::Error)]
pub enum CommittedBlockProofError {
    #[error("Failed to decode commit proof: {0}")]
    Decode(String),
    #[error("Invalid shard group in commit proof header: start={start}, end_inclusive={end_inclusive}")]
    InvalidShardGroup { start: u32, end_inclusive: u32 },
    #[error("Sidechain proof validation error: {0}")]
    SidechainProofValidationError(#[from] SidechainProofValidationError),
}

#[cfg(test)]
mod tests {
    use tari_crypto::ristretto::RistrettoSecretKey;
    use tari_sidechain::{SidechainBlockHeader, ValidatorBlockSignature};

    use super::*;

    fn sample_proof() -> CommittedBlockProof {
        let header = SidechainBlockHeader {
            network: 0,
            protocol_version: 0,
            parent_id: FixedHash::zero(),
            justify_id: FixedHash::zero(),
            height: 7,
            epoch: 3,
            epoch_hash: FixedHash::zero(),
            shard_group: tari_sidechain::ShardGroup {
                start: 1,
                end_inclusive: 4,
            },
            proposed_by: CompressedPublicKey::default(),
            state_merkle_root: FixedHash::new([7u8; 32]),
            command_merkle_root: FixedHash::zero(),
            signature: ValidatorBlockSignature::new(CompressedPublicKey::default(), RistrettoSecretKey::default()),
            accumulated_data: Default::default(),
            metadata_hash: FixedHash::zero(),
        };
        CommittedBlockProof::new(SidechainBlockCommitProof {
            header,
            proof_elements: vec![],
        })
    }

    #[test]
    fn it_round_trips_through_bytes_and_reads_header_fields() {
        // Encoding must match what the validator produces and the indexer decodes - a regression
        // here would silently break verification across the wire.
        let proof = sample_proof();
        let decoded = CommittedBlockProof::from_bytes(&proof.to_bytes()).unwrap();
        assert_eq!(decoded.epoch(), Epoch(3));
        assert_eq!(decoded.height(), NodeHeight(7));
        assert_eq!(decoded.state_merkle_root(), FixedHash::new([7u8; 32]));
        assert_eq!(decoded.shard_group().unwrap(), ShardGroup::new_checked(1, 4).unwrap());
    }

    #[test]
    fn it_rejects_undecodable_bytes() {
        assert!(CommittedBlockProof::from_bytes(&[0xff, 0x00, 0x13, 0x37]).is_err());
    }
}
