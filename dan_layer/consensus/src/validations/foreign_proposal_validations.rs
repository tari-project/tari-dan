//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::debug;
use tari_dan_common_types::shard::Shard;
use tari_dan_storage::consensus_models::{Block, ForeignReceiveCounters};

use crate::{
    hotstuff::{HotStuffError, ProposalValidationError},
    traits::ConsensusSpec,
};

pub fn check_foreign_proposal_message<TConsensusSpec: ConsensusSpec>(
    from: &TConsensusSpec::Addr,
    candidate_block: &Block,
    local_shard: Shard,
    foreign_receive_counter: &ForeignReceiveCounters,
) -> Result<(), ProposalValidationError> {
    // Check the foreign counter is present
    let Some(incoming_count) = candidate_block.get_foreign_counter(&local_shard) else {
        return Err(ProposalValidationError::MissingForeignCounters {
            proposed_by: from.to_string(),
            hash: *candidate_block.id(),
        });
    };
    // Check if we have missed any proposals.
    // TODO: This isn't an error, we should request the missing proposals
    let current_count = foreign_receive_counter.get_count(&candidate_block.shard());
    if current_count + 1 != incoming_count {
        return Err(ProposalValidationError::InvalidForeignCounters {
            proposed_by: from.to_string(),
            hash: *candidate_block.id(),
            details: format!(
                "Expected foreign receive count to be {} but it was {}",
                current_count + 1,
                incoming_count
            ),
        });
    }
    if candidate_block.height().is_zero() || candidate_block.is_genesis() {
        return Err(ProposalValidationError::ProposingGenesisBlock {
            proposed_by: from.to_string(),
            hash: *candidate_block.id(),
        });
    }

    let calculated_hash = candidate_block.calculate_hash().into();
    if calculated_hash != *candidate_block.id() {
        return Err(ProposalValidationError::NodeHashMismatch {
            proposed_by: from.to_string(),
            hash: *candidate_block.id(),
            calculated_hash,
        });
    }

    // TODO: validate justify signatures
    // self.validate_qc(candidate_block.justify(), committee)?;

    Ok(())
}
