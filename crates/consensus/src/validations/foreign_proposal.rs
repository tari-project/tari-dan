//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use log::*;
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{ProtocolVersion, committee::Committee, optional::Optional};
use tari_ootle_storage::{
    StateStoreReadTransaction,
    consensus_models::{CommandsCommitProof, ForeignProposal, ForeignProposalRecord},
};
use tari_ootle_transaction::Network;

use crate::{
    hotstuff::{HotStuffError, HotstuffConfig, ProposalValidationError},
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::ootle::consensus::validations::foreign_proposal";

/// Returns the committee of the shard group the proposal's header names, in the proposal's epoch. The proposal is
/// processed as the evidence and pledges of that shard group, so its proof must verify against this committee.
///
/// Returns `None` when no committee is assigned to exactly that shard group in that epoch.
pub async fn resolve_foreign_committee<TEpochManager: EpochManagerReader>(
    epoch_manager: &TEpochManager,
    proposal: &ForeignProposal,
) -> Result<Option<Arc<Committee<TEpochManager::Addr>>>, HotStuffError> {
    let shard_group = proposal
        .shard_group_checked()
        .ok_or_else(|| ProposalValidationError::InvalidShardGroup {
            block_id: proposal.calculate_block_id(),
            shard_group: proposal.shard_group_unchecked(),
            details: "Foreign proposal header names a shard group with invalid bounds".to_string(),
        })?;

    let committee = epoch_manager
        .get_committee_by_shard_group(proposal.epoch(), shard_group)
        .await
        .optional()?;
    if committee.is_none() {
        warn!(
            target: LOG_TARGET,
            "❌ Foreign proposal block {} names shard group {} which has no committee in epoch {}",
            proposal,
            shard_group,
            proposal.epoch(),
        );
    }
    Ok(committee)
}

/// Returns true if a foreign proposal with this proposal's commit proof and block metadata is stored. Every stored
/// foreign proposal was authenticated before it was saved, and authentication reads only those two, so this
/// proposal authenticates too.
///
/// The whole commit proof is compared because stored foreign proposals are pruned by epoch: a message that shares
/// only the block id with a stored proposal may be saved once that proposal is pruned, so it must be authenticated on
/// its own.
pub fn is_authenticated_commit_proof_stored<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    proposal: &ForeignProposal,
) -> Result<bool, HotStuffError> {
    let stored = ForeignProposalRecord::get_any(tx, [&proposal.calculate_block_id()])?;
    let metadata_hash = proposal.block_metadata().calculate_hash();
    Ok(stored.iter().any(|record| {
        record.proposal().commit_proof() == proposal.commit_proof() &&
            record.proposal().block_metadata().calculate_hash() == metadata_hash
    }))
}

/// Checks the proposal against the committee of the shard group its header names, in the proposal's epoch.
pub async fn authenticate_foreign_proposal<TConsensusSpec: ConsensusSpec>(
    epoch_manager: &TConsensusSpec::EpochManager,
    proposal: &ForeignProposal,
    config: &HotstuffConfig,
) -> Result<(), HotStuffError> {
    let committee = resolve_foreign_committee(epoch_manager, proposal)
        .await?
        .ok_or_else(|| ProposalValidationError::InvalidShardGroup {
            block_id: proposal.calculate_block_id(),
            shard_group: proposal.shard_group_unchecked(),
            details: format!(
                "Foreign proposal header names a shard group with no committee in epoch {}",
                proposal.epoch()
            ),
        })?;
    check_foreign_proposal::<TConsensusSpec>(proposal, &committee, config)
}

pub fn check_foreign_proposal<TConsensusSpec: ConsensusSpec>(
    proposal: &ForeignProposal,
    foreign_committee: &Committee<TConsensusSpec::Addr>,
    config: &HotstuffConfig,
) -> Result<(), HotStuffError> {
    check_network(proposal, config.network)?;
    check_protocol_version(proposal, config.network)?;
    check_proposer_in_committee(proposal, foreign_committee)?;
    check_header(proposal)?;
    check_block_metadata(proposal)?;
    check_commit_proof::<TConsensusSpec>(proposal.commit_proof(), config.network, foreign_committee)?;
    Ok(())
}

/// The block metadata is read for the rate every transaction the proposal pins settles at, so it must be the
/// preimage of the metadata hash the committee signed, and it must name a rate.
fn check_block_metadata(proposal: &ForeignProposal) -> Result<(), ProposalValidationError> {
    if !proposal.is_block_metadata_proven() {
        return Err(ProposalValidationError::ForeignBlockMetadataNotProven {
            block_id: proposal.calculate_block_id(),
            shard_group: proposal.shard_group_unchecked(),
        });
    }
    if proposal.exhaust_burn_rate().is_none() {
        return Err(ProposalValidationError::ForeignBlockNamesNoExhaustBurnRate {
            block_id: proposal.calculate_block_id(),
            shard_group: proposal.shard_group_unchecked(),
        });
    }
    Ok(())
}

fn check_proposer_in_committee<TAddr: PartialEq>(
    proposal: &ForeignProposal,
    foreign_committee: &Committee<TAddr>,
) -> Result<(), ProposalValidationError> {
    let proposed_by = proposal.proposed_by();
    if !foreign_committee.contains_public_key(&proposed_by) {
        return Err(ProposalValidationError::ValidatorNotInCommittee {
            validator: proposed_by.to_string(),
            details: format!(
                "Foreign proposal {} was proposed by a validator outside the committee of shard group {}",
                proposal.calculate_block_id(),
                proposal.shard_group_unchecked(),
            ),
        });
    }
    Ok(())
}

fn check_header(proposal: &ForeignProposal) -> Result<(), ProposalValidationError> {
    proposal.commit_proof().validate_header()?;
    Ok(())
}

pub(super) fn check_network(proposal: &ForeignProposal, network: Network) -> Result<(), ProposalValidationError> {
    if proposal.network_byte() != network.as_byte() {
        return Err(ProposalValidationError::InvalidNetwork {
            block_network: Network::try_from(proposal.network_byte())
                .map(|n| n.to_string())
                .unwrap_or_else(|_| format!("<unknown> byte: {}", proposal.network_byte())),
            expected_network: network.to_string(),
            block_id: proposal.calculate_block_id(),
        });
    }
    Ok(())
}

/// A foreign block is held to the same activation schedule as a local one. Its version decides whether its pledges
/// must match the value hashes in its evidence, so a block that claims an earlier version than its epoch runs under
/// is rejected rather than checked under the weaker rule.
fn check_protocol_version(proposal: &ForeignProposal, network: Network) -> Result<(), ProposalValidationError> {
    let block_version = proposal
        .protocol_version()
        .map_err(|e| ProposalValidationError::ForeignProposalInvalid {
            block_id: proposal.calculate_block_id(),
            shard_group: proposal.shard_group_unchecked(),
            details: e.into(),
        })?;
    let expected_version = ProtocolVersion::at(network, proposal.epoch());
    if block_version != expected_version {
        return Err(ProposalValidationError::InvalidProtocolVersion {
            expected_version,
            block_version,
            epoch: proposal.epoch(),
            block_id: proposal.calculate_block_id(),
        });
    }
    Ok(())
}

pub fn check_commit_proof<TConsensusSpec: ConsensusSpec>(
    proof: &CommandsCommitProof,
    network: Network,
    foreign_committee: &Committee<TConsensusSpec::Addr>,
) -> Result<(), ProposalValidationError> {
    let quorum_threshold = foreign_committee.quorum_threshold();
    proof.validate_committed(network, quorum_threshold, foreign_committee)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::CompressedPublicKey;
    use tari_crypto::tari_utilities::ByteArray;
    use tari_ootle_common_types::{
        ExtraData,
        ExtraFieldKey,
        ShardGroup,
        VotePower,
        committee::CommitteeMember,
        crypto::create_key_pair_from_seed,
    };
    use tari_ootle_storage::consensus_models::{BlockHeaderMetadata, BlockPledge};
    use tari_sidechain::{SidechainBlockCommitProof, SidechainBlockHeader};
    use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

    use super::*;

    fn public_key(seed: u8) -> RistrettoPublicKeyBytes {
        let (_, pk) = create_key_pair_from_seed(seed);
        RistrettoPublicKeyBytes::from_bytes(pk.as_bytes()).unwrap()
    }

    fn committee(seeds: &[u8]) -> Committee<String> {
        Committee::new(
            seeds
                .iter()
                .map(|seed| CommitteeMember {
                    address: format!("vn{seed}"),
                    public_key: public_key(*seed),
                    vote_power: VotePower::of(1),
                })
                .collect(),
        )
    }

    fn metadata_naming(rate_bps: Option<u16>) -> BlockHeaderMetadata {
        let mut extra_data = ExtraData::new();
        if let Some(bps) = rate_bps {
            extra_data.insert_bps(ExtraFieldKey::ExhaustBurnRate, bps);
        }
        BlockHeaderMetadata {
            total_leader_fee: 0,
            timestamp: 1234,
            extra_data,
            timeout_certificate_id: None,
            is_dummy: false,
        }
    }

    fn proposal(proposer_seed: u8, shard_group: ShardGroup) -> ForeignProposal {
        let metadata = metadata_naming(Some(500));
        proposal_with_metadata(proposer_seed, shard_group, &metadata, metadata.clone())
    }

    /// A proposal whose proven header commits to `hashed` and which carries `carried` as its block metadata.
    fn proposal_with_metadata(
        proposer_seed: u8,
        shard_group: ShardGroup,
        hashed: &BlockHeaderMetadata,
        carried: BlockHeaderMetadata,
    ) -> ForeignProposal {
        let (_, proposer) = create_key_pair_from_seed(proposer_seed);
        let commit_proof = CommandsCommitProof::new_latest(vec![], SidechainBlockCommitProof {
            header: SidechainBlockHeader {
                network: Network::LocalNet.as_byte(),
                protocol_version: 0,
                parent_id: Default::default(),
                justify_id: Default::default(),
                height: 1,
                epoch: 1,
                epoch_hash: Default::default(),
                shard_group: tari_sidechain::ShardGroup {
                    start: shard_group.start().as_u32(),
                    end_inclusive: shard_group.end().as_u32(),
                },
                proposed_by: CompressedPublicKey::new_from_pk(proposer),
                state_merkle_root: Default::default(),
                command_merkle_root: Default::default(),
                transaction_merkle_root: None,
                signature: Default::default(),
                accumulated_data: Default::default(),
                metadata_hash: hashed.calculate_hash(),
            },
            proof_elements: vec![],
        });
        ForeignProposal::new(commit_proof, BlockPledge::default(), carried)
    }

    fn shard_group_b() -> ShardGroup {
        ShardGroup::new(129u32, 256u32)
    }

    #[test]
    fn it_rejects_a_proposer_outside_the_named_shard_groups_committee() {
        let b = shard_group_b();
        let committee_b = committee(&[11, 12, 13]);
        // The header names shard group B, but the proposer is a validator outside B's committee
        let proposal = proposal(1, b);

        let err = check_proposer_in_committee(&proposal, &committee_b).unwrap_err();
        assert!(
            matches!(err, ProposalValidationError::ValidatorNotInCommittee { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn it_accepts_a_proposer_inside_the_named_shard_groups_committee() {
        let b = shard_group_b();
        let committee_b = committee(&[11, 12, 13]);
        let proposal = proposal(12, b);

        check_proposer_in_committee(&proposal, &committee_b).unwrap();
    }

    #[test]
    fn it_accepts_block_metadata_that_hashes_to_the_proven_header() {
        let proposal = proposal(1, shard_group_b());

        check_block_metadata(&proposal).unwrap();
        assert_eq!(proposal.exhaust_burn_rate().map(|rate| rate.as_bps()), Some(500));
    }

    #[test]
    fn it_rejects_block_metadata_naming_a_rate_the_header_does_not_commit_to() {
        let proposal = proposal_with_metadata(
            1,
            shard_group_b(),
            &metadata_naming(Some(500)),
            metadata_naming(Some(900)),
        );

        let err = check_block_metadata(&proposal).unwrap_err();
        assert!(
            matches!(err, ProposalValidationError::ForeignBlockMetadataNotProven { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn it_rejects_proven_block_metadata_that_names_no_rate() {
        let metadata = metadata_naming(None);
        let proposal = proposal_with_metadata(1, shard_group_b(), &metadata, metadata.clone());

        let err = check_block_metadata(&proposal).unwrap_err();
        assert!(
            matches!(err, ProposalValidationError::ForeignBlockNamesNoExhaustBurnRate { .. }),
            "unexpected error: {err}"
        );
    }
}
