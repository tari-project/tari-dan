//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{committee::Committee, DerivableFromPublicKey};
use tari_dan_storage::consensus_models::Block;
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{HotStuffError, ProposalValidationError},
    traits::{ConsensusSpec, LeaderStrategy, VoteSignatureService},
};

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

pub async fn check_quorum_certificate<TConsensusSpec: ConsensusSpec>(
    candidate_block: &Block,
    vote_signing_service: &TConsensusSpec::SignatureService,
    epoch_manager: &TConsensusSpec::EpochManager,
) -> Result<(), HotStuffError> {
    if candidate_block.justify().epoch().as_u64() == 0 {
        // Ignore genesis block.
        return Ok(());
    }
    if candidate_block.height() < candidate_block.justify().block_height() {
        return Err(ProposalValidationError::CandidateBlockNotHigherThanJustify {
            justify_block_height: candidate_block.justify().block_height(),
            candidate_block_height: candidate_block.height(),
        }
        .into());
    }
    let mut vns = vec![];
    for signature in candidate_block.justify().signatures() {
        let vn = epoch_manager
            .get_validator_node_by_public_key(candidate_block.justify().epoch(), signature.public_key())
            .await?;
        vns.push(vn.node_hash());
    }
    let merkle_root = epoch_manager
        .get_validator_node_merkle_root(candidate_block.justify().epoch())
        .await?;
    let qc = candidate_block.justify();
    let proof = qc.merged_proof().clone();
    if !proof.verify_consume(&merkle_root, vns.iter().map(|hash| hash.to_vec()).collect())? {
        return Err(ProposalValidationError::QCisNotValid { qc: qc.clone() }.into());
    }

    for (sign, leaf) in qc.signatures().iter().zip(vns.iter()) {
        let challenge = vote_signing_service.create_challenge(leaf, qc.block_id(), &qc.decision());
        if !sign.verify(challenge) {
            return Err(ProposalValidationError::QCInvalidSignature { qc: qc.clone() }.into());
        }
    }
    let committee_shard = epoch_manager
        .get_committee_shard_by_validator_public_key(candidate_block.epoch(), candidate_block.proposed_by())
        .await?;

    if committee_shard.quorum_threshold() >
        u32::try_from(qc.signatures().len()).map_err(|_| ProposalValidationError::QCisNotValid { qc: qc.clone() })?
    {
        return Err(ProposalValidationError::QuorumWasNotReached { qc: qc.clone() }.into());
    }
    Ok(())
}
