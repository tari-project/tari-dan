//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::fees::ExhaustBurnRate;
use tari_ootle_common_types::{
    Epoch,
    committee::{Committee, CommitteeInfo},
};
use tari_ootle_storage::consensus_models::{Block, BlockHeader};
use tari_sidechain::FixedHash;

use super::common::{
    check_block_signature,
    check_current_epoch,
    check_epoch_hash,
    check_exhaust_burn_rate,
    check_height,
    check_network,
    check_next_epoch_fee_scale,
    check_proposal_certificate,
    check_protocol_version,
    check_shard_group_bounds,
    check_shard_group_matches,
    check_sidechain_id,
    check_timeout_certificate,
};
use crate::{
    hotstuff::{HotStuffError, HotstuffConfig, ProposalValidationError},
    traits::ConsensusSpec,
};

pub fn check_local_proposal<TConsensusSpec: ConsensusSpec>(
    current_epoch: Epoch,
    block: &Block,
    committee_for_block: &Committee<TConsensusSpec::Addr>,
    local_committee_info: &CommitteeInfo,
    vote_signing_service: &TConsensusSpec::SignerService,
    config: &HotstuffConfig,
    expected_epoch_hash: &FixedHash,
    expected_exhaust_burn_rate: ExhaustBurnRate,
) -> Result<(), HotStuffError> {
    check_proposal::<TConsensusSpec>(
        block,
        committee_for_block,
        vote_signing_service,
        config,
        expected_epoch_hash,
        expected_exhaust_burn_rate,
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
    config: &HotstuffConfig,
    expected_epoch_hash: &FixedHash,
    expected_exhaust_burn_rate: ExhaustBurnRate,
) -> Result<(), HotStuffError> {
    check_header::<TConsensusSpec>(
        block.header(),
        expected_epoch_hash,
        expected_exhaust_burn_rate,
        config,
        signer_service,
    )?;
    check_block(block)?;
    check_next_epoch_fee_scale(block, config.network)?;
    check_proposal_certificate::<TConsensusSpec>(config.network, block, committee_for_block, signer_service)?;
    check_timeout_certificate::<TConsensusSpec>(config.network, block, committee_for_block, signer_service)?;

    Ok(())
}
/// Checks that do not depend on committed state. Whether the proposer is the leader of the view
/// below the block does: it is checked in `validate_local_proposed_block`, where the block's justify
/// is on hand to anchor the liveness state that decides it.
pub(super) fn check_block(block: &Block) -> Result<(), ProposalValidationError> {
    check_height(block)?;
    Ok(())
}

fn check_header<TConsensusSpec: ConsensusSpec>(
    header: &BlockHeader,
    expected_epoch_hash: &FixedHash,
    expected_exhaust_burn_rate: ExhaustBurnRate,
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
    check_exhaust_burn_rate(header, expected_exhaust_burn_rate)?;
    check_shard_group_bounds(header, config.consensus_constants.num_preshards)?;
    check_block_signature(header, signer_service)?;
    check_sidechain_id(header, config)?;
    Ok(())
}
