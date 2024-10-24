//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common::configuration::Network;
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_dan_common_types::{
    committee::{Committee, CommitteeInfo},
    DerivableFromPublicKey,
    ExtraFieldKey,
};
use tari_dan_storage::consensus_models::Block;
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{HotStuffError, HotstuffConfig, ProposalValidationError},
    traits::{ConsensusSpec, LeaderStrategy, VoteSignatureService},
};

pub fn check_proposal<TConsensusSpec: ConsensusSpec>(
    block: &Block,
    committee_info: &CommitteeInfo,
    committee_for_block: &Committee<TConsensusSpec::Addr>,
    vote_signing_service: &TConsensusSpec::SignatureService,
    leader_strategy: &TConsensusSpec::LeaderStrategy,
    config: &HotstuffConfig,
) -> Result<(), HotStuffError> {
    // TODO: in order to do the base layer block has validation, we need to ensure that we have synced to the tip.
    //       If not, we need some strategy for "parking" the blocks until we are at least at the provided hash or the
    //       tip. Without this, the check has a race condition between the base layer scanner and consensus.
    // check_base_layer_block_hash::<TConsensusSpec>(block, epoch_manager, config).await?;
    check_network(block, config.network)?;
    check_sidechain_id(block, config)?;
    check_hash_and_height(block)?;
    check_proposed_by_leader(leader_strategy, committee_for_block, block)?;
    check_signature(block)?;
    check_quorum_certificate::<TConsensusSpec>(block, committee_for_block, committee_info, vote_signing_service)?;
    Ok(())
}

pub fn check_network(candidate_block: &Block, network: Network) -> Result<(), ProposalValidationError> {
    if candidate_block.network() != network {
        return Err(ProposalValidationError::InvalidNetwork {
            block_network: candidate_block.network().to_string(),
            expected_network: network.to_string(),
            block_id: *candidate_block.id(),
        });
    }
    Ok(())
}

// TODO: remove allow(dead_code)
#[allow(dead_code)]
pub async fn check_base_layer_block_hash<TConsensusSpec: ConsensusSpec>(
    block: &Block,
    epoch_manager: &TConsensusSpec::EpochManager,
    config: &HotstuffConfig,
) -> Result<(), HotStuffError> {
    if block.is_genesis() {
        return Ok(());
    }
    // Check if know the base layer block hash
    let base_layer_block_height = epoch_manager
        .get_base_layer_block_height(*block.base_layer_block_hash())
        .await?
        .ok_or_else(|| ProposalValidationError::BlockHashNotFound {
            hash: *block.base_layer_block_hash(),
        })?;
    // Check if the base layer block height is matching the base layer block hash
    if base_layer_block_height != block.base_layer_block_height() {
        Err(ProposalValidationError::BlockHeightMismatch {
            height: block.base_layer_block_height(),
            real_height: base_layer_block_height,
        })?;
    }
    // Check if the base layer block height is within the acceptable range
    let current_height = epoch_manager.current_base_layer_block_info().await?.0;
    // TODO: uncomment this when the sync information is available here, otherwise during sync this will fail
    // if base_layer_block_height + config.max_base_layer_blocks_behind < current_height {
    //     Err(ProposalValidationError::BlockHeightTooSmall {
    //         proposed: base_layer_block_height,
    //         current: current_height,
    //     })?;
    // }
    if base_layer_block_height > current_height + config.consensus_constants.max_base_layer_blocks_ahead {
        Err(ProposalValidationError::BlockHeightTooHigh {
            proposed: base_layer_block_height,
            current: current_height,
        })?;
    }
    // if block.is_epoch_end() && !epoch_manager.is_last_block_of_epoch(base_layer_block_height).await? {
    //     Err(ProposalValidationError::NotLastBlockOfEpoch {
    //         block_id: *block.id(),
    //         base_layer_block_height,
    //     })?;
    // }
    Ok(())
}

pub fn check_hash_and_height(candidate_block: &Block) -> Result<(), ProposalValidationError> {
    if candidate_block.is_genesis() {
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
    let (leader, _) = leader_strategy.get_leader(local_committee, candidate_block.height());
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
        .signature()
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

pub fn check_quorum_certificate<TConsensusSpec: ConsensusSpec>(
    candidate_block: &Block,
    committee: &Committee<TConsensusSpec::Addr>,
    committee_info: &CommitteeInfo,
    vote_signing_service: &TConsensusSpec::SignatureService,
) -> Result<(), HotStuffError> {
    let qc = candidate_block.justify();
    if qc.is_zero() {
        // TODO: This is potentially dangerous. There should be a check
        // to make sure this is the start of the chain.

        return Ok(());
    }
    if candidate_block.height() <= qc.block_height() {
        return Err(ProposalValidationError::CandidateBlockNotHigherThanJustify {
            justify_block_height: qc.block_height(),
            candidate_block_height: candidate_block.height(),
        }
        .into());
    }

    if qc.signatures().is_empty() {
        return Err(ProposalValidationError::QuorumWasNotReached { qc: qc.clone() }.into());
    }

    for signature in qc.signatures() {
        if !committee.contains_public_key(signature.public_key()) {
            return Err(ProposalValidationError::ValidatorNotInCommittee {
                validator: signature.public_key().to_string(),
                details: format!(
                    "QC signed with validator {} that is not in committee {}",
                    signature.public_key(),
                    committee_info.shard_group()
                ),
            }
            .into());
        }
    }

    for sign in qc.signatures() {
        let message = vote_signing_service.create_message(qc.block_id(), &qc.decision());
        if !sign.verify(message) {
            return Err(ProposalValidationError::QCInvalidSignature { qc: qc.clone() }.into());
        }
    }

    if committee_info.quorum_threshold() >
        u32::try_from(qc.signatures().len()).map_err(|_| ProposalValidationError::QCConversionError)?
    {
        return Err(ProposalValidationError::QuorumWasNotReached { qc: qc.clone() }.into());
    }
    Ok(())
}

pub fn check_sidechain_id(candidate_block: &Block, config: &HotstuffConfig) -> Result<(), HotStuffError> {
    // We only require the sidechain id on the genesis block
    if !candidate_block.is_genesis() {
        return Ok(());
    }

    // If we are using a sidechain id in the network, we need to check it matches the candidate block one
    if let Some(expected_sidechain_id) = &config.sidechain_id {
        // Extract the sidechain id from the candidate block
        let extra_data = candidate_block.extra_data().ok_or::<HotStuffError>(
            ProposalValidationError::MissingSidechainId {
                block_id: *candidate_block.id(),
            }
            .into(),
        )?;
        let sidechain_id_bytes = extra_data.get(&ExtraFieldKey::SidechainId).ok_or::<HotStuffError>(
            ProposalValidationError::InvalidSidechainId {
                block_id: *candidate_block.id(),
                reason: "SidechainId key not present".to_owned(),
            }
            .into(),
        )?;
        let sidechain_id = RistrettoPublicKey::from_canonical_bytes(sidechain_id_bytes).map_err(|e| {
            ProposalValidationError::InvalidSidechainId {
                block_id: *candidate_block.id(),
                reason: e.to_string(),
            }
        })?;

        // The sidechain id must match the sidechain of the current network
        if sidechain_id != *expected_sidechain_id {
            return Err(ProposalValidationError::MismatchedSidechainId {
                block_id: *candidate_block.id(),
                expected_sidechain_id: expected_sidechain_id.clone(),
                sidechain_id,
            }
            .into());
        }
    }

    Ok(())
}
