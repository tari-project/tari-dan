//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::debug;
use tari_dan_storage::consensus_models::Block;
use crate::hotstuff::{HotStuffError, ProposalValidationError};

pub async fn check_foreign_proposal_message( candidate_block: &Block,) -> Result<(), HotStuffError> {
 let Some(incoming_count) = candidate_block.get_foreign_counter(&local_shard) else {
  debug!(target:LOG_TARGET, "Our bucket {local_shard:?} is missing reliability index in the proposed block {candidate_block:?}");
  return Err(ProposalValidationError::MissingForeignCounters {
   proposed_by: from.to_string(),
   hash: *candidate_block.id(),
  });
 };
 let current_count = foreign_receive_counter.get_count(&foreign_shard);
 if current_count + 1 != incoming_count {
  debug!(target:LOG_TARGET, "We were expecting the index to be {expected_count}, but the index was {incoming_count}", expected_count = current_count + 1);
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

 todo!()
}
