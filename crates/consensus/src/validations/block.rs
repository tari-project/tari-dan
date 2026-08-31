//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_ootle_common_types::{
    DerivableFromPublicKey,
    Epoch,
    committee::{Committee, CommitteeInfo},
};
use tari_ootle_storage::consensus_models::{Block, BlockHeader};

use super::common::{
    check_block_signature,
    check_current_epoch,
    check_epoch_hash,
    check_height,
    check_network,
    check_proposal_certificate,
    check_proposed_by_leader,
    check_protocol_version,
    check_shard_group_bounds,
    check_shard_group_matches,
    check_sidechain_id,
    check_timeout_certificate,
};
use crate::{
    hotstuff::{HotStuffError, HotstuffConfig, ProposalValidationError},
    traits::{ConsensusSpec, LeaderStrategy},
};

pub fn check_local_proposal<TConsensusSpec: ConsensusSpec>(
    current_epoch: Epoch,
    block: &Block,
    committee_for_block: &Committee<TConsensusSpec::Addr>,
    local_committee_info: &CommitteeInfo,
    vote_signing_service: &TConsensusSpec::SignerService,
    leader_strategy: &TConsensusSpec::LeaderStrategy,
    config: &HotstuffConfig,
    expected_epoch_hash: &FixedHash,
) -> Result<(), HotStuffError> {
    check_proposal::<TConsensusSpec>(
        block,
        committee_for_block,
        vote_signing_service,
        leader_strategy,
        config,
        expected_epoch_hash,
    )?;
    check_shard_group_matches(block.header(), local_committee_info.shard_group())?;
    // This proposal is valid, if it is for an epoch ahead of us, we need to sync
    check_current_epoch(block, current_epoch)?;
    Ok(())
}
fn check_proposal<TConsensusSpec: ConsensusSpec>(
    block: &Block,
    committee_for_block: &Committee<TConsensusSpec::Addr>,
    signer_service: &TConsensusSpec::SignerService,
    leader_strategy: &TConsensusSpec::LeaderStrategy,
    config: &HotstuffConfig,
    expected_epoch_hash: &FixedHash,
) -> Result<(), HotStuffError> {
    check_header::<TConsensusSpec>(block.header(), expected_epoch_hash, config, signer_service)?;
    check_block(leader_strategy, committee_for_block, block)?;
    check_proposal_certificate::<TConsensusSpec>(block, committee_for_block, signer_service)?;
    check_timeout_certificate::<TConsensusSpec>(block, committee_for_block, signer_service)?;

    Ok(())
}
pub(super) fn check_block<TAddr: DerivableFromPublicKey, TLeaderStrategy: LeaderStrategy<TAddr>>(
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
    block: &Block,
) -> Result<(), ProposalValidationError> {
    check_height(block)?;
    check_proposed_by_leader(leader_strategy, local_committee, block)?;
    Ok(())
}

fn check_header<TConsensusSpec: ConsensusSpec>(
    header: &BlockHeader,
    expected_epoch_hash: &FixedHash,
    config: &HotstuffConfig,
    signer_service: &TConsensusSpec::SignerService,
) -> Result<(), ProposalValidationError> {
    check_network(header, config.network)?;
    check_protocol_version(header, config.network)?;
    if header.is_genesis() {
        return Err(ProposalValidationError::ProposingGenesisBlock {
            proposed_by: header.proposed_by().to_string(),
            block_id: *header.id(),
        });
    }

    if header.is_dummy() {
        return Err(ProposalValidationError::ProposingDummyBlock {
            proposed_by: header.proposed_by().to_string(),
            block: header.as_leaf(),
        });
    }
    check_epoch_hash(header, expected_epoch_hash)?;
    check_shard_group_bounds(header, config.consensus_constants.num_preshards)?;
    check_block_signature(header, signer_service)?;
    check_sidechain_id(header, config)?;
    Ok(())
}
