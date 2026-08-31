//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::collections::HashSet;

use log::{debug, warn};
use tari_common_types::types::FixedHash;
use tari_consensus_types::{QuorumCertificateRef, TimeoutVote};
use tari_ootle_common_types::{
    DerivableFromPublicKey,
    Epoch,
    ExtraFieldKey,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    VotePower,
    committee::Committee,
};
use tari_ootle_storage::consensus_models::{Block, BlockHeader};
use tari_ootle_transaction::Network;
use tari_sidechain::ProposalVoteMessage;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    hotstuff::{HotstuffConfig, ProposalValidationError},
    traits::{ConsensusSpec, LeaderStrategy, ValidatorSignatureVerifierService},
    validations::signed_vote::SignedProposalVote,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::validations";

pub(super) fn check_current_epoch(
    candidate_block: &Block,
    current_epoch: Epoch,
) -> Result<(), ProposalValidationError> {
    if candidate_block.epoch() > current_epoch {
        warn!(target: LOG_TARGET, "⚠️ Proposal for future epoch {} received. Current epoch is {}", candidate_block.epoch(), current_epoch);
        return Err(ProposalValidationError::FutureEpoch {
            block_id: *candidate_block.id(),
            current_epoch,
            block_epoch: candidate_block.epoch(),
        });
    }

    Ok(())
}

pub(super) fn check_network(header: &BlockHeader, network: Network) -> Result<(), ProposalValidationError> {
    if header.network() != network {
        return Err(ProposalValidationError::InvalidNetwork {
            block_network: header.network().to_string(),
            expected_network: network.to_string(),
            block_id: *header.id(),
        });
    }
    Ok(())
}

/// The protocol version a block is produced under is fixed by the network's activation schedule at the block's
/// epoch. A node whose schedule disagrees rejects the block instead of hashing it under a schema the rest of the
/// network has left behind, which stalls the node rather than forking it.
pub(super) fn check_protocol_version(header: &BlockHeader, network: Network) -> Result<(), ProposalValidationError> {
    let expected_version = ProtocolVersion::at(network, header.epoch());
    if header.protocol_version() != expected_version {
        return Err(ProposalValidationError::InvalidProtocolVersion {
            expected_version,
            block_version: header.protocol_version(),
            epoch: header.epoch(),
            block_id: *header.id(),
        });
    }
    Ok(())
}

pub(super) fn check_epoch_hash(
    header: &BlockHeader,
    expected_epoch_hash: &FixedHash,
) -> Result<(), ProposalValidationError> {
    if header.epoch_hash() != expected_epoch_hash {
        return Err(ProposalValidationError::InvalidEpochHash {
            epoch: header.epoch(),
            local_epoch_hash: *expected_epoch_hash,
            invalid_epoch_hash: *header.epoch_hash(),
            block_id: *header.id(),
        });
    }

    Ok(())
}

pub(super) fn check_shard_group_matches(
    header: &BlockHeader,
    expected_shard_group: ShardGroup,
) -> Result<(), ProposalValidationError> {
    if header.shard_group() != expected_shard_group {
        return Err(ProposalValidationError::InvalidShardGroup {
            block_id: *header.id(),
            shard_group: header.shard_group(),
            details: format!(
                "Expected shard group {} but got {}",
                expected_shard_group,
                header.shard_group()
            ),
        });
    }

    Ok(())
}

pub(super) fn check_shard_group_bounds(
    header: &BlockHeader,
    num_preshards: NumPreshards,
) -> Result<(), ProposalValidationError> {
    let len = header
        .shard_group()
        .checked_len()
        .ok_or_else(|| ProposalValidationError::InvalidShardGroup {
            block_id: *header.id(),
            shard_group: header.shard_group(),
            details: "Shard group bounds are invalid".to_string(),
        })?;

    if header.shard_group().start().as_u32() as usize > num_preshards.num_shards() ||
        header.shard_group().end().as_u32() as usize > num_preshards.num_shards()
    {
        return Err(ProposalValidationError::InvalidShardGroup {
            block_id: *header.id(),
            shard_group: header.shard_group(),
            details: format!(
                "Shard group {} is out of bounds for {} preshards",
                header.shard_group(),
                num_preshards.num_shards()
            ),
        });
    }

    if len > num_preshards.num_shards() {
        return Err(ProposalValidationError::InvalidShardGroup {
            block_id: *header.id(),
            shard_group: header.shard_group(),
            details: format!(
                "Shard group {} is larger than the number of preshards {}",
                header.shard_group(),
                num_preshards.num_shards()
            ),
        });
    }

    Ok(())
}

pub(super) fn check_height(block: &Block) -> Result<(), ProposalValidationError> {
    if block.height().is_zero() {
        return Err(ProposalValidationError::InvalidBlockHeight {
            block_id: *block.id(),
            block_height: block.height(),
            details: "Block height is zero".to_string(),
        });
    }
    let max_certificate_height = block.max_certificate_height();
    // invariant: the block may only advance the view by 1 higher than the justified height
    if block.height() != max_certificate_height + NodeHeight(1) {
        return Err(ProposalValidationError::InvalidBlockHeight {
            block_id: *block.id(),
            block_height: block.height(),
            details: format!("Expected it to be one higher than the max certificate height {max_certificate_height}"),
        });
    }
    Ok(())
}

pub(super) fn check_proposed_by_leader<TAddr: DerivableFromPublicKey, TLeaderStrategy: LeaderStrategy<TAddr>>(
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    block: &Block,
) -> Result<(), ProposalValidationError> {
    let (addr, leader) = leader_strategy.get_leader(local_committee, block.height() - NodeHeight(1));
    if leader != block.proposed_by() {
        return Err(ProposalValidationError::NotLeader {
            proposed_by: block.proposed_by().to_string(),
            expected_leader: format!("{} / {}", leader, addr),
            block: block.as_leaf(),
            max_certificate_height: block.max_certificate_height(),
        });
    }
    Ok(())
}

pub(super) fn check_block_signature<TSignerService: ValidatorSignatureVerifierService>(
    header: &BlockHeader,
    signer_service: &TSignerService,
) -> Result<(), ProposalValidationError> {
    if header.is_genesis() {
        // Genesis block doesn't have signatures
        return Ok(());
    }

    let validator_signature = header.signature().ok_or(ProposalValidationError::MissingSignature {
        block_id: *header.id(),
        height: header.height(),
    })?;

    debug!(
        target: LOG_TARGET,
        "Validating signature block_id={}, P={}, R={}",
        header.id(),
        header.proposed_by(),
        validator_signature.public_nonce(),
    );

    if !signer_service.verify(header) {
        return Err(ProposalValidationError::InvalidSignature {
            block_id: *header.id(),
            height: header.height(),
        });
    }
    Ok(())
}

pub(super) fn check_proposal_certificate<TConsensusSpec: ConsensusSpec>(
    network: Network,
    candidate_block: &Block,
    committee: &Committee<TConsensusSpec::Addr>,
    signing_service: &TConsensusSpec::SignerService,
) -> Result<(), ProposalValidationError> {
    let qc = candidate_block.justify();
    if candidate_block.height() <= qc.height() {
        return Err(ProposalValidationError::CandidateBlockNotHigherThanJustify {
            justify_block_height: qc.height(),
            candidate_block_height: candidate_block.height(),
        });
    }

    check_quorum_certificate_signatures::<TConsensusSpec>(network, qc.into(), committee, signing_service)?;

    Ok(())
}

pub(super) fn check_timeout_certificate<TConsensusSpec: ConsensusSpec>(
    network: Network,
    candidate_block: &Block,
    committee: &Committee<TConsensusSpec::Addr>,
    signing_service: &TConsensusSpec::SignerService,
) -> Result<(), ProposalValidationError> {
    let Some(tc) = candidate_block.timeout_certificate() else {
        return Ok(());
    };
    if candidate_block.height() <= tc.height() {
        return Err(ProposalValidationError::CandidateBlockNotHigherThanJustify {
            justify_block_height: tc.height(),
            candidate_block_height: candidate_block.height(),
        });
    }

    check_quorum_certificate_signatures::<TConsensusSpec>(network, tc.into(), committee, signing_service)?;

    Ok(())
}

/// Validates the signatures of the quorum certificate.
// pub because used in on receive NEWVIEW
pub fn check_quorum_certificate_signatures<TConsensusSpec: ConsensusSpec>(
    network: Network,
    qc: QuorumCertificateRef<'_>,
    committee: &Committee<TConsensusSpec::Addr>,
    signing_service: &TConsensusSpec::SignerService,
) -> Result<(), ProposalValidationError> {
    // The "zero block" is the deterministic per-epoch genesis, known to every node without a vote, so a
    // QC that justifies it is exempt from quorum/signature validation. Only a `ProposalCertificate` in
    // canonical genesis shape (height zero, zero parent, zero header hash) qualifies for the exemption;
    // everything else - including a `TimeoutCertificate` - falls through to the checks below.
    if let Some(pc) = qc.as_proposal_certificate() &&
        pc.height().is_zero() &&
        pc.signatures().is_empty() &&
        pc.justifies_zero_block() &&
        pc.parent_id().is_zero()
    {
        return Ok(());
    }

    let mut check_dups = HashSet::with_capacity(qc.signatures().len());
    let mut total_vote_power = VotePower::zero();
    for signature in qc.signatures() {
        let Some(power) = committee.get_power_by_public_key(signature.public_key()) else {
            return Err(ProposalValidationError::ValidatorNotInCommittee {
                validator: signature.public_key().to_string(),
                details: format!(
                    "QC {} signed with validator {} that is not in committee",
                    qc,
                    signature.public_key(),
                ),
            });
        };
        total_vote_power += power;
        if !check_dups.insert(signature.public_key()) {
            return Err(ProposalValidationError::QcDuplicateSignature {
                qc: qc.calculate_id(),
                validator: *signature.public_key(),
            });
        }

        match qc {
            QuorumCertificateRef::ProposalCertificate(pc) => {
                let block_id = pc.calculate_block_id();
                let message = ProposalVoteMessage::new(
                    ProtocolVersion::at(network, pc.epoch()).as_u32(),
                    block_id.hash(),
                    pc.decision(),
                    pc.epoch().as_u64(),
                    pc.height().as_u64(),
                );
                let vote = SignedProposalVote { message, signature };
                let is_valid = signing_service.verify(&vote);
                if !is_valid {
                    return Err(ProposalValidationError::QcInvalidSignature { qc: qc.calculate_id() });
                }
            },
            QuorumCertificateRef::TimeoutCertificate(tc) => {
                let vote = TimeoutVote {
                    epoch: tc.epoch(),
                    height: tc.height(),
                    signature: signature.clone(),
                };
                let is_valid = signing_service.verify(&vote);
                if !is_valid {
                    return Err(ProposalValidationError::QcInvalidSignature { qc: qc.calculate_id() });
                }
            },
        }
    }

    if total_vote_power < committee.quorum_threshold() {
        return Err(ProposalValidationError::QuorumWasNotReached {
            qc: qc.calculate_id(),
            got: total_vote_power,
            required: committee.quorum_threshold(),
        });
    }

    Ok(())
}

pub(super) fn check_sidechain_id(header: &BlockHeader, config: &HotstuffConfig) -> Result<(), ProposalValidationError> {
    // We only require the sidechain id on the genesis block
    if !header.is_genesis() {
        return Ok(());
    }

    // If we are using a sidechain id in the network, we need to check it matches the candidate block one
    let Some(expected_sidechain_id) = &config.sidechain_id else {
        return Ok(());
    };

    // Extract the sidechain id from the candidate block
    let extra_data = header.extra_data();
    let sidechain_id_bytes =
        extra_data
            .get(&ExtraFieldKey::SidechainId)
            .ok_or(ProposalValidationError::InvalidSidechainId {
                block_id: *header.id(),
                reason: "SidechainId key not present".to_owned(),
            })?;
    let sidechain_id = RistrettoPublicKeyBytes::from_bytes(sidechain_id_bytes.as_ref()).map_err(|e| {
        ProposalValidationError::InvalidSidechainId {
            block_id: *header.id(),
            reason: e.to_string(),
        }
    })?;

    // The sidechain id must match the sidechain of the current network
    if sidechain_id != *expected_sidechain_id {
        return Err(ProposalValidationError::MismatchedSidechainId {
            block_id: *header.id(),
            expected_sidechain_id: *expected_sidechain_id,
            sidechain_id,
        });
    }

    Ok(())
}
