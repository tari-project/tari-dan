//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{committee::Committee, DerivableFromPublicKey, NodeAddressable};
use tari_dan_storage::consensus_models::Block;

use crate::{hotstuff::ProposalValidationError, traits::LeaderStrategy};

pub fn check_hash_and_height(candidate_block: &Block) -> Result<(), ProposalValidationError> {
    if candidate_block.height().is_zero() || candidate_block.is_genesis() {
        return Err(ProposalValidationError::ProposingGenesisBlock {
            proposed_by: candidate_block.proposed_by().to_string(),
            hash: *candidate_block.id(),
        });
    }

    let calculated_hash = candidate_block.calculate_hash().into();
    if calculated_hash != *candidate_block.id() {
        return Err(ProposalValidationError::NodeHashMismatch {
            proposed_by: candidate_block.proposed_by().to_string(),
            hash: *candidate_block.id(),
            calculated_hash,
        });
    }

    Ok(())
}

pub fn check_proposed_by_leader<TAddr: DerivableFromPublicKey, TLeaderStrategy: LeaderStrategy<TAddr>>(
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    candidate_block: &Block,
) -> Result<(), ProposalValidationError> {
    let leader = leader_strategy.get_leader(local_committee, candidate_block.height());
    if !leader.eq_to_public_key(candidate_block.proposed_by()) {
        return Err(ProposalValidationError::NotLeader {
            proposed_by: candidate_block.proposed_by().to_string(),
            expected_leader: leader.to_string(),
            block_id: *candidate_block.id(),
        });
    }
    Ok(())
}

pub fn check_signature(candidate_block: &Block) -> Result<(), ProposalValidationError> {
    if candidate_block.is_dummy() {
        // Dummy blocks don't have signatures
        return Ok(());
    }
    if candidate_block.is_genesis() {
        // Genesis block doesn't have signatures
        return Ok(());
    }
    let validator_signature = candidate_block
        .get_signature()
        .ok_or(ProposalValidationError::MissingSignature {
            block_id: *candidate_block.id(),
            height: candidate_block.height(),
        })?;
    if !validator_signature.verify(candidate_block.proposed_by(), candidate_block.id()) {
        return Err(ProposalValidationError::InvalidSignature {
            block_id: *candidate_block.id(),
            height: candidate_block.height(),
        });
    }
    Ok(())
}

pub fn check_quorum_certificate<TAddr: NodeAddressable>(
    _local_committee: &Committee<TAddr>,
    candidate_block: &Block,
) -> Result<(), ProposalValidationError> {
    if candidate_block.height() < candidate_block.justify().block_height() {
        return Err(ProposalValidationError::CandidateBlockNotHigherThanJustify {
            justify_block_height: candidate_block.justify().block_height(),
            candidate_block_height: candidate_block.height(),
        });
    }
    // TODO: validate QC

    Ok(())
}
