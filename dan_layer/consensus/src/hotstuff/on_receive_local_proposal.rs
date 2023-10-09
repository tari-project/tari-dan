//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// (New, true) ----(cmd:Prepare) ---> (Prepared, true) -----cmd:LocalPrepared ---> (LocalPrepared, false)
// ----[foreign:LocalPrepared]--->(LocalPrepared, true) ----cmd:AllPrepare ---> (AllPrepared, true) ---cmd:Accept --->
// Complete

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::{committee::Committee, optional::Optional, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, ValidBlock},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{error::HotStuffError, on_new_valid_local_block::OnNewValidLocalBlock, ProposalValidationError},
    messages::ProposalMessage,
    traits::{ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_local_proposal";

pub struct OnReceiveProposalHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    on_new_valid_local_block: OnNewValidLocalBlock<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveProposalHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        on_new_valid_local_block: OnNewValidLocalBlock<TConsensusSpec>,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            leader_strategy,
            on_new_valid_local_block,
        }
    }

    pub async fn handle(&self, message: ProposalMessage<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;

        info!(
            target: LOG_TARGET,
            "🔥 Receive LOCAL PROPOSAL for block {} from {}",
            block,
            block.proposed_by()
        );

        if let Some(valid_block) = self.validate_block(block).await? {
            self.on_new_valid_local_block.handle(valid_block).await?;
        }

        Ok(())
    }

    async fn validate_block(
        &self,
        block: Block<TConsensusSpec::Addr>,
    ) -> Result<Option<ValidBlock<TConsensusSpec::Addr>>, HotStuffError> {
        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        // First save the block in one db transaction
        self.store.with_write_tx(|tx| {
            match self.validate_local_proposed_block_and_fill_dummy_blocks(tx, block, &local_committee) {
                Ok(validated) => Ok(Some(validated)),
                // Block sync
                Err(
                    err @ HotStuffError::ProposalValidationError(ProposalValidationError::JustifyBlockNotFound {
                        ..
                    }),
                ) => Err(err),
                // Validation errors should not cause a FAILURE state transition
                Err(HotStuffError::ProposalValidationError(err)) => {
                    warn!(target: LOG_TARGET, "❌ Block failed validation: {}", err);
                    // A bad block should not cause a FAILURE state transition
                    Ok(None)
                },
                Err(e) => Err(e),
            }
        })
    }

    fn validate_local_proposed_block_and_fill_dummy_blocks(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        candidate_block: Block<TConsensusSpec::Addr>,
        local_committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<ValidBlock<TConsensusSpec::Addr>, HotStuffError> {
        let leader = self
            .leader_strategy
            .get_leader(local_committee, candidate_block.height());
        if leader != candidate_block.proposed_by() {
            return Err(ProposalValidationError::NotLeader {
                proposed_by: candidate_block.proposed_by().to_string(),
                expected_leader: leader.to_string(),
                block_id: *candidate_block.id(),
            }
            .into());
        }

        self.validate_proposed_block(&candidate_block)?;

        if Block::has_been_processed(tx.deref_mut(), candidate_block.id())? {
            return Err(ProposalValidationError::BlockAlreadyProcessed {
                block_id: *candidate_block.id(),
                height: candidate_block.height(),
            }
            .into());
        }

        // Check that details included in the justify match previously added blocks
        let Some(justify_block) = candidate_block.justify().get_block(tx.deref_mut()).optional()? else {
            // This will trigger a sync
            return Err(ProposalValidationError::JustifyBlockNotFound {
                proposed_by: candidate_block.proposed_by().to_string(),
                block_id: *candidate_block.id(),
                justify_block: *candidate_block.justify().block_id(),
            }
            .into());
        };

        if justify_block.height() != candidate_block.justify().block_height() {
            return Err(ProposalValidationError::JustifyBlockInvalid {
                proposed_by: candidate_block.proposed_by().to_string(),
                block_id: *candidate_block.id(),
                details: format!(
                    "Justify block height ({}) does not match justify block height ({})",
                    justify_block.height(),
                    candidate_block.justify().block_height()
                ),
            }
            .into());
        }

        // let leaf_block = LeafBlock::get(tx.deref_mut())?;
        // if candidate_block.height() <= leaf_block.height() {
        //     return Err(ProposalValidationError::CandidateBlockNotHigherThanLeafBlock {
        //         proposed_by: from.to_string(),
        //         leaf_block,
        //         candidate_block: candidate_block.as_leaf_block(),
        //     }
        //     .into());
        // }

        // Special case for genesis block
        if candidate_block.parent().is_genesis() && candidate_block.justify().is_genesis() {
            return Ok(ValidBlock::new(candidate_block));
        }

        // Part of the safenode predicate. Exclude this block early if this is the case
        // let locked_block = LockedBlock::get(tx.deref_mut())?;
        // if !locked_block.block_id.is_genesis() && candidate_block.justify().block_height() <= locked_block.height() {
        //     return Err(ProposalValidationError::CandidateBlockNotHigherThanLockedBlock {
        //         proposed_by: from.to_string(),
        //         locked_block,
        //         candidate_block: candidate_block.as_leaf_block(),
        //     }
        //     .into());
        // }

        // candidate_block.justify().update_high_qc(tx)?;

        // if candidate_block.height().saturating_sub(justify_block.height()).0 > local_committee.max_failures() as u64
        // { TODO: We should maybe relax this constraint during GST, before the first block, many leaders might
        // fail....
        // Note: we are adding at least one more block from b_leaf, so we need to add 1 to the max_failures
        // TODO: Skip this check for small committees just so that we can continue in testing. This case should be
        //       formalized.
        // Ignoring this for now as it blocks us when hammering nodes with transactions
        // if local_committee.max_failures() > 0 &&
        //     candidate_block.height().saturating_sub(justify_block.height()).as_u64() >
        //         local_committee.len() as u64 + 1
        // {
        //     return Err(ProposalValidationError::CandidateBlockHigherThanMaxFailures {
        //         proposed_by: from.to_string(),
        //         justify_block_height: justify_block.height(),
        //         candidate_block_height: candidate_block.height(),
        //         max_failures: local_committee.max_failures(),
        //     }
        //     .into());
        // }

        if candidate_block.height() < justify_block.height() {
            return Err(ProposalValidationError::CandidateBlockNotHigherThanJustifyBlock {
                justify_block_height: justify_block.height(),
                candidate_block_height: candidate_block.height(),
            }
            .into());
        }

        let justify_block_height = justify_block.height();

        if justify_block.id() != candidate_block.parent() {
            let mut dummy_blocks =
                Vec::with_capacity((candidate_block.height().as_u64() - justify_block_height.as_u64() - 1) as usize);
            dummy_blocks.push(justify_block);
            let mut last_dummy_block = dummy_blocks.last().unwrap();
            // if the block parent is not the justify parent, then we have experienced a leader failure
            // and should make dummy blocks to fill in the gaps.
            while last_dummy_block.id() != candidate_block.parent() {
                if last_dummy_block.height() > candidate_block.height() {
                    warn!(target: LOG_TARGET, "🔥 Bad proposal, dummy block height {} is greater than new height {}", last_dummy_block, candidate_block);
                    return Err(ProposalValidationError::CandidateBlockDoesNotExtendJustify {
                        justify_block_height,
                        candidate_block_height: candidate_block.height(),
                    }
                    .into());
                }

                let next_height = last_dummy_block.height() + NodeHeight(1);
                let leader = self.leader_strategy.get_leader(local_committee, next_height);

                // TODO: replace with actual leader's propose
                dummy_blocks.push(Block::dummy_block(
                    *last_dummy_block.id(),
                    leader.clone(),
                    next_height,
                    candidate_block.justify().clone(),
                    candidate_block.epoch(),
                ));
                last_dummy_block = dummy_blocks.last().unwrap();
                debug!(target: LOG_TARGET, "🍼 DUMMY BLOCK: {}. Leader: {}", last_dummy_block, leader);
            }

            // The logic for not checking is_safe is as follows:
            // We can't without adding the dummy blocks to the DB
            // We know that justify_block is safe because we have added it to our chain
            // We know that each dummy block is built in a chain from the justify block to the candidate block
            // We know that last dummy block is the parent of candidate block
            // Therefore we know that candidate block is safe
            return Ok(ValidBlock::with_dummy_blocks(candidate_block, dummy_blocks));
        }

        // Now that we have all dummy blocks (if any) in place, we can check if the candidate block is safe.
        // Specifically, it should extend the locked block via the dummy blocks.
        if !candidate_block.is_safe(tx.deref_mut())? {
            return Err(ProposalValidationError::NotSafeBlock {
                proposed_by: candidate_block.proposed_by().to_string(),
                hash: *candidate_block.id(),
            }
            .into());
        }

        Ok(ValidBlock::new(candidate_block))
    }

    pub async fn reprocess_block(&self, block: Block<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        if !self.epoch_manager.is_epoch_active(block.epoch()).await? {
            return Err(HotStuffError::EpochNotActive {
                epoch: block.epoch(),
                details: "Cannot reprocess block from inactive epoch".to_string(),
            });
        }

        info!(target: LOG_TARGET, "♻️ Reprocessing block {block} after all transactions have been executed");

        if let Some(valid_block) = self.validate_block(block).await? {
            self.on_new_valid_local_block.handle(valid_block).await?;
        }

        Ok(())
    }

    fn validate_proposed_block(
        &self,
        candidate_block: &Block<TConsensusSpec::Addr>,
    ) -> Result<(), ProposalValidationError> {
        if candidate_block.height() == NodeHeight::zero() || candidate_block.id().is_genesis() {
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

        // TODO: validate justify signatures
        // self.validate_qc(candidate_block.justify(), committee)?;

        Ok(())
    }
}
