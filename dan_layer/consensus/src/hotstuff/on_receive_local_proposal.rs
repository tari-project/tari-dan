//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// (New, true) ----(cmd:Prepare) ---> (Prepared, true) -----cmd:LocalPrepared ---> (LocalPrepared, false)
// ----[foreign:LocalPrepared]--->(LocalPrepared, true) ----cmd:AllPrepare ---> (AllPrepared, true) ---cmd:Accept --->
// Complete

use std::ops::DerefMut;

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    shard::Shard,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, ForeignSendCounters, HighQc, TransactionPool, ValidBlock},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;
use tokio::sync::broadcast;

use super::proposer::{self, Proposer};
use crate::{
    hotstuff::{
        error::HotStuffError,
        on_ready_to_vote_on_local_block::OnReadyToVoteOnLocalBlock,
        pacemaker_handle::PaceMakerHandle,
        HotstuffEvent,
        ProposalValidationError,
    },
    messages::ProposalMessage,
    traits::{ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_local_proposal";

pub struct OnReceiveLocalProposalHandler<TConsensusSpec: ConsensusSpec> {
    network: Network,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    pacemaker: PaceMakerHandle,
    on_ready_to_vote_on_local_block: OnReadyToVoteOnLocalBlock<TConsensusSpec>,
}

impl<TConsensusSpec: ConsensusSpec> OnReceiveLocalProposalHandler<TConsensusSpec> {
    pub fn new(
        validator_addr: TConsensusSpec::Addr,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        pacemaker: PaceMakerHandle,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        vote_signing_service: TConsensusSpec::SignatureService,
        state_manager: TConsensusSpec::StateManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        proposer: Proposer<TConsensusSpec>,
        network: Network,
    ) -> Self {
        Self {
            network,
            store: store.clone(),
            epoch_manager: epoch_manager.clone(),
            leader_strategy: leader_strategy.clone(),
            pacemaker,
            on_ready_to_vote_on_local_block: OnReadyToVoteOnLocalBlock::new(
                validator_addr,
                store,
                epoch_manager,
                vote_signing_service,
                leader_strategy,
                state_manager,
                transaction_pool,
                outbound_messaging,
                tx_events,
                proposer,
                network,
            ),
        }
    }

    pub async fn handle(&mut self, message: ProposalMessage) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;

        debug!(
            target: LOG_TARGET,
            "🔥 LOCAL PROPOSAL: block {} from {}",
            block,
            block.proposed_by()
        );

        self.process_block(block).await?;

        Ok(())
    }

    async fn process_block(&mut self, block: Block) -> Result<(), HotStuffError> {
        if !self.epoch_manager.is_epoch_active(block.epoch()).await? {
            return Err(HotStuffError::EpochNotActive {
                epoch: block.epoch(),
                details: "Cannot reprocess block from inactive epoch".to_string(),
            });
        }

        let local_committee = self
            .epoch_manager
            .get_committee_by_validator_public_key(block.epoch(), block.proposed_by())
            .await?;
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(block.epoch()).await?;

        let maybe_high_qc_and_block = self.store.with_write_tx(|tx| {
            let Some(valid_block) =
                self.validate_block(tx.deref_mut(), block, &local_committee, &local_committee_shard)?
            else {
                return Ok(None);
            };

            // Save the block as soon as it is valid to ensure we have a valid pacemaker height.
            let high_qc = self.save_block(tx, &valid_block)?;
            info!(target: LOG_TARGET, "✅ Block {} is valid and persisted. HighQc({})", valid_block, high_qc);
            Ok::<_, HotStuffError>(Some((high_qc, valid_block)))
        })?;

        if let Some((high_qc, valid_block)) = maybe_high_qc_and_block {
            self.pacemaker
                .update_view(valid_block.height(), high_qc.block_height())
                .await?;

            self.on_ready_to_vote_on_local_block.handle(valid_block).await?;
        }

        Ok(())
    }

    fn save_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        valid_block: &ValidBlock,
    ) -> Result<HighQc, HotStuffError> {
        valid_block.block().save_foreign_send_counters(tx)?;
        valid_block.block().justify().save(tx)?;
        valid_block.save_all_dummy_blocks(tx)?;
        valid_block.block().save(tx)?;
        let high_qc = valid_block.block().justify().update_high_qc(tx)?;
        Ok(high_qc)
    }

    fn validate_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: Block,
        local_committee: &Committee<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<Option<ValidBlock>, HotStuffError> {
        match self.validate_local_proposed_block(tx, block, local_committee, local_committee_shard) {
            Ok(validated) => Ok(Some(validated)),
            // Propagate this error out as sync is needed in the case where we have a valid QC but do not know the
            // block
            Err(err @ HotStuffError::ProposalValidationError(ProposalValidationError::JustifyBlockNotFound { .. })) => {
                Err(err)
            },
            // Validation errors should not cause a FAILURE state transition
            Err(HotStuffError::ProposalValidationError(err)) => {
                warn!(target: LOG_TARGET, "❌ Block failed validation: {}", err);
                // A bad block should not cause a FAILURE state transition
                Ok(None)
            },
            Err(e) => Err(e),
        }
    }

    fn check_foreign_indexes(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        num_committees: u32,
        local_shard: Shard,
        block: &Block,
        justify_block: &BlockId,
    ) -> Result<(), HotStuffError> {
        let non_local_shards = proposer::get_non_local_shards(tx, block, num_committees, local_shard)?;
        let block_foreign_indexes = block.foreign_indexes();
        if block_foreign_indexes.len() != non_local_shards.len() {
            return Err(ProposalValidationError::InvalidForeignCounters {
                proposed_by: block.proposed_by().to_string(),
                hash: *block.id(),
                details: format!(
                    "Foreign indexes length ({}) does not match non-local shards length ({})",
                    block_foreign_indexes.len(),
                    non_local_shards.len()
                ),
            }
            .into());
        }

        let mut foreign_counters = ForeignSendCounters::get_or_default(tx, justify_block)?;
        let mut current_shard = None;
        for (shard, foreign_count) in block_foreign_indexes {
            if let Some(current_shard) = current_shard {
                // Check ordering
                if current_shard > shard {
                    return Err(ProposalValidationError::InvalidForeignCounters {
                        proposed_by: block.proposed_by().to_string(),
                        hash: *block.id(),
                        details: format!(
                            "Foreign indexes are not sorted by shard. Current shard: {}, shard: {}",
                            current_shard, shard
                        ),
                    }
                    .into());
                }
            }

            current_shard = Some(shard);
            // Check that each shard is correct
            if !non_local_shards.contains(shard) {
                return Err(ProposalValidationError::InvalidForeignCounters {
                    proposed_by: block.proposed_by().to_string(),
                    hash: *block.id(),
                    details: format!("Shard {} is not a non-local shard", shard),
                }
                .into());
            }

            // Check that foreign counters are correct
            let expected_count = foreign_counters.increment_counter(*shard);
            if *foreign_count != expected_count {
                return Err(ProposalValidationError::InvalidForeignCounters {
                    proposed_by: block.proposed_by().to_string(),
                    hash: *block.id(),
                    details: format!(
                        "Foreign counter for shard {} is incorrect. Expected {}, got {}",
                        shard, expected_count, foreign_count
                    ),
                }
                .into());
            }
        }

        Ok(())
    }

    /// Perform final block validations (TODO: implement all validations)
    /// We assume at this point that initial stateless validations have been done (in inbound messages)
    fn validate_local_proposed_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        candidate_block: Block,
        local_committee: &Committee<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<ValidBlock, HotStuffError> {
        if Block::has_been_processed(tx, candidate_block.id())? {
            return Err(ProposalValidationError::BlockAlreadyProcessed {
                block_id: *candidate_block.id(),
                height: candidate_block.height(),
            }
            .into());
        }

        // Check that details included in the justify match previously added blocks
        let Some(justify_block) = candidate_block.justify().get_block(tx).optional()? else {
            // This will trigger a sync
            return Err(ProposalValidationError::JustifyBlockNotFound {
                proposed_by: candidate_block.proposed_by().to_string(),
                block_description: candidate_block.to_string(),
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

        // Special case for genesis block
        if candidate_block.parent().is_genesis() && candidate_block.justify().is_genesis() {
            return Ok(ValidBlock::new(candidate_block));
        }

        if candidate_block.height() < justify_block.height() {
            return Err(ProposalValidationError::CandidateBlockNotHigherThanJustify {
                justify_block_height: justify_block.height(),
                candidate_block_height: candidate_block.height(),
            }
            .into());
        }

        self.check_foreign_indexes(
            tx,
            local_committee_shard.num_committees(),
            local_committee_shard.shard(),
            &candidate_block,
            justify_block.id(),
        )?;

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
                let leader = self.leader_strategy.get_leader_public_key(local_committee, next_height);

                // TODO: replace with actual leader's propose
                dummy_blocks.push(Block::dummy_block(
                    self.network,
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
        if !candidate_block.is_safe(tx)? {
            return Err(ProposalValidationError::NotSafeBlock {
                proposed_by: candidate_block.proposed_by().to_string(),
                hash: *candidate_block.id(),
            }
            .into());
        }

        Ok(ValidBlock::new(candidate_block))
    }
}
