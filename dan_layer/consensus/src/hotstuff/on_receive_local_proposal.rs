//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeInfo},
    optional::Optional,
    Epoch,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{Block, HighQc, LastSentVote, QuorumCertificate, QuorumDecision, TransactionPool, ValidBlock},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;
use tokio::{sync::broadcast, task};

use crate::{
    hotstuff::{
        calculate_dummy_blocks,
        create_epoch_checkpoint,
        error::HotStuffError,
        on_ready_to_vote_on_local_block::OnReadyToVoteOnLocalBlock,
        pacemaker_handle::PaceMakerHandle,
        transaction_manager::ConsensusTransactionManager,
        HotstuffConfig,
        HotstuffEvent,
        ProposalValidationError,
    },
    messages::{ForeignProposalMessage, HotstuffMessage, VoteMessage},
    traits::{
        hooks::ConsensusHooks,
        ConsensusSpec,
        LeaderStrategy,
        OutboundMessaging,
        ValidatorSignatureService,
        VoteSignatureService,
    },
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_local_proposal";

pub struct OnReceiveLocalProposalHandler<TConsensusSpec: ConsensusSpec> {
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    pacemaker: PaceMakerHandle,
    on_ready_to_vote_on_local_block: OnReadyToVoteOnLocalBlock<TConsensusSpec>,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    vote_signing_service: TConsensusSpec::SignatureService,
    hooks: TConsensusSpec::Hooks,
}

impl<TConsensusSpec: ConsensusSpec> OnReceiveLocalProposalHandler<TConsensusSpec> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        pacemaker: PaceMakerHandle,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        vote_signing_service: TConsensusSpec::SignatureService,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        transaction_manager: ConsensusTransactionManager<
            TConsensusSpec::TransactionExecutor,
            TConsensusSpec::StateStore,
        >,
        config: HotstuffConfig,
        hooks: TConsensusSpec::Hooks,
    ) -> Self {
        let local_validator_pk = vote_signing_service.public_key().clone();
        Self {
            config: config.clone(),
            store: store.clone(),
            epoch_manager,
            leader_strategy,
            pacemaker,
            vote_signing_service,
            outbound_messaging,
            hooks,
            on_ready_to_vote_on_local_block: OnReadyToVoteOnLocalBlock::new(
                local_validator_pk,
                config,
                store,
                transaction_pool,
                tx_events,
                transaction_manager,
            ),
        }
    }

    pub async fn handle(&mut self, current_epoch: Epoch, block: Block) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 LOCAL PROPOSAL: block {} from {}",
            block,
            block.proposed_by()
        );

        match self.process_block(current_epoch, block).await {
            Ok(()) => Ok(()),
            Err(err @ HotStuffError::ProposalValidationError(_)) => {
                self.hooks.on_block_validation_failed(&err);
                Err(err)
            },
            Err(err) => Err(err),
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn process_block(&mut self, current_epoch: Epoch, block: Block) -> Result<(), HotStuffError> {
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
        let local_committee_info = self
            .epoch_manager
            .get_committee_info_by_validator_public_key(block.epoch(), block.proposed_by())
            .await?;

        let maybe_high_qc_and_block = self.store.with_write_tx(|tx| {
            if block.exists(&**tx)? {
                info!(target: LOG_TARGET, "🧊 Block {} already exists", block);
                return Ok(None);
            }

            let Some(valid_block) = self.validate_block_header(&*tx, block, &local_committee, &local_committee_info)?
            else {
                return Ok(None);
            };

            // Save the block as soon as it is valid to ensure we have a valid pacemaker height.
            let high_qc = self.save_block(tx, &valid_block)?;
            info!(target: LOG_TARGET, "✅ Block {} is valid and persisted. HighQc({})", valid_block, high_qc);
            Ok::<_, HotStuffError>(Some((high_qc, valid_block)))
        })?;

        if let Some((high_qc, valid_block)) = maybe_high_qc_and_block {
            let em_epoch = self.epoch_manager.current_epoch().await?;
            let can_propose_epoch_end = em_epoch > current_epoch;

            let mut on_ready_to_vote_on_local_block = self.on_ready_to_vote_on_local_block.clone();
            let (block_decision, valid_block) = task::spawn_blocking(move || {
                let decision = on_ready_to_vote_on_local_block.handle(
                    &valid_block,
                    &local_committee_info,
                    can_propose_epoch_end,
                )?;
                Ok::<_, HotStuffError>((decision, valid_block))
            })
            .await??;

            self.hooks
                .on_local_block_decide(&valid_block, block_decision.quorum_decision);
            for t in block_decision.finalized_transactions.into_iter().flatten() {
                self.hooks.on_transaction_finalized(&t.into_current_transaction_atom());
            }
            self.propose_newly_locked_blocks(block_decision.locked_blocks).await?;

            if let Some(decision) = block_decision.quorum_decision {
                let is_registered = self
                    .epoch_manager
                    .is_this_validator_registered_for_epoch(valid_block.epoch())
                    .await?;

                if is_registered {
                    debug!(
                        target: LOG_TARGET,
                        "🔥 LOCAL PROPOSAL {} DECIDED {:?}",
                        valid_block,
                        decision,
                    );
                    let local_committee = self.epoch_manager.get_local_committee(valid_block.epoch()).await?;

                    let vote = self.generate_vote_message(valid_block.block(), decision).await?;
                    self.send_vote_to_leader(&local_committee, vote, valid_block.block())
                        .await?;
                } else {
                    info!(
                        target: LOG_TARGET,
                        "❓️ Local validator not registered for epoch {}. Not voting on block {}",
                        valid_block.epoch(),
                        valid_block,
                    );
                }
            }

            match block_decision.end_of_epoch {
                Some(epoch) => {
                    let next_epoch = epoch + Epoch(1);

                    // If we're registered for the next epoch. Create a new genesis block.
                    if let Some(vn) = self.epoch_manager.get_our_validator_node(next_epoch).await.optional()? {
                        // TODO: Change VN db to include the shard group in the ValidatorNode struct.
                        let num_committees = self.epoch_manager.get_num_committees(next_epoch).await?;
                        let next_shard_group = vn.shard_key.to_shard_group(self.config.num_preshards, num_committees);
                        self.store.with_write_tx(|tx| {
                            // Generate checkpoint
                            create_epoch_checkpoint(tx, epoch, local_committee_info.shard_group())?;

                            // Create the next genesis
                            let mut genesis = Block::genesis(self.config.network, next_epoch, next_shard_group);
                            info!(target: LOG_TARGET, "⭐️ Creating new genesis block {genesis}");
                            genesis.justify().insert(tx)?;
                            genesis.insert(tx)?;
                            genesis.set_as_justified(tx)?;
                            // We'll propose using the new genesis as parent
                            genesis.as_locked_block().set(tx)?;
                            genesis.as_leaf_block().set(tx)?;
                            genesis.as_last_executed().set(tx)?;
                            genesis.as_last_voted().set(tx)?;
                            genesis.justify().as_high_qc().set(tx)?;
                            Ok::<_, HotStuffError>(())
                        })?;

                        // TODO: We should exit consensus to sync for the epoch - when this is implemented, we will not
                        // need to create the genesis, set the pacemaker, etc.
                        self.pacemaker.set_epoch(next_epoch).await?;
                        self.pacemaker.on_beat();
                    } else {
                        info!(
                            target: LOG_TARGET,
                            "💤 Our validator node is not registered for epoch {next_epoch}.",
                        )
                    }
                },
                None => {
                    self.pacemaker
                        .update_view(valid_block.epoch(), valid_block.height(), high_qc.block_height())
                        .await?;
                },
            }
        }

        Ok(())
    }

    async fn send_vote_to_leader(
        &mut self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        vote: VoteMessage,
        block: &Block,
    ) -> Result<(), HotStuffError> {
        let leader = self
            .leader_strategy
            .get_leader_for_next_block(local_committee, block.height());
        info!(
            target: LOG_TARGET,
            "🔥 VOTE {:?} for block {} proposed by {} to next leader {:.4}",
            vote.decision,
            block,
            block.proposed_by(),
            leader,
        );
        self.outbound_messaging
            .send(leader.clone(), HotstuffMessage::Vote(vote.clone()))
            .await?;
        self.store.with_write_tx(|tx| {
            let last_sent_vote = LastSentVote {
                epoch: vote.epoch,
                block_id: vote.block_id,
                block_height: vote.block_height,
                decision: vote.decision,
                signature: vote.signature,
            };
            last_sent_vote.set(tx)
        })?;
        Ok(())
    }

    async fn propose_newly_locked_blocks(
        &mut self,
        blocks: Vec<(Block, QuorumCertificate)>,
    ) -> Result<(), HotStuffError> {
        for (block, justify_qc) in blocks {
            debug!(target:LOG_TARGET,"Broadcast new locked block: {block}");
            let Some(our_vn) = self
                .epoch_manager
                .get_our_validator_node(block.epoch())
                .await
                .optional()?
            else {
                info!(
                    target: LOG_TARGET,
                    "❌ Our validator node is not registered for epoch {}. Not proposing {block} to foreign committee", block.epoch(),
                );
                continue;
            };

            let local_committee = self
                .epoch_manager
                .get_committee_by_validator_public_key(block.epoch(), block.proposed_by())
                .await?;
            let leader_index = self.leader_strategy.calculate_leader(&local_committee, block.height()) as usize;
            let my_index = local_committee
                .addresses()
                .position(|addr| *addr == our_vn.address)
                .ok_or_else(|| HotStuffError::InvariantError("Our address not found in local committee".to_string()))?;
            // There are other ways to approach this. But for simplicity, it is better just to make sure at least one
            // honest node will send it to the whole foreign committee. So we select the leader and f other
            // nodes. It has to be deterministic so we select by index (leader, leader+1, ..., leader+f).
            // f+1 nodes (including the leader) send the proposal to the foreign committee

            let should_broadcast = if my_index >= leader_index {
                my_index - leader_index <= local_committee.len() / 3
            } else {
                my_index + local_committee.len() - leader_index <= local_committee.len() / 3
            };

            if should_broadcast {
                self.broadcast_foreign_proposal_if_required(block, justify_qc).await?;
            }
        }
        Ok(())
    }

    async fn generate_vote_message(
        &self,
        block: &Block,
        decision: QuorumDecision,
    ) -> Result<VoteMessage, HotStuffError> {
        let vn = self
            .epoch_manager
            .get_validator_node_by_public_key(block.epoch(), self.vote_signing_service.public_key())
            .await?;
        let leaf_hash = vn.get_node_hash(self.config.network);

        let signature = self.vote_signing_service.sign_vote(&leaf_hash, block.id(), &decision);

        Ok(VoteMessage {
            epoch: block.epoch(),
            block_id: *block.id(),
            block_height: block.height(),
            decision,
            signature,
        })
    }

    fn save_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        valid_block: &ValidBlock,
    ) -> Result<HighQc, HotStuffError> {
        valid_block.block().save_foreign_send_counters(tx)?;
        valid_block.block().justify().save(tx)?;
        valid_block.save_all_dummy_blocks(tx)?;
        valid_block.block().insert(tx)?;

        let (_, high_qc) = valid_block.block().justify().check_high_qc(tx)?;
        Ok(high_qc)
    }

    fn validate_block_header(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: Block,
        local_committee: &Committee<TConsensusSpec::Addr>,
        local_committee_info: &CommitteeInfo,
    ) -> Result<Option<ValidBlock>, HotStuffError> {
        let result = self.validate_local_proposed_block(tx, block, local_committee, local_committee_info);
        // .and_then(|valid_block| {
        //     self.update_foreign_proposal_transactions(tx, valid_block.block())?;
        //     Ok(valid_block)
        // });

        match result {
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

    // fn update_foreign_proposal_transactions(
    //     &self,
    //     tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
    //     block: &Block,
    // ) -> Result<(), HotStuffError> {
    //     // TODO: Move this to consensus constants
    //     const FOREIGN_PROPOSAL_TIMEOUT: u64 = 1000;
    //     let all_proposed = ForeignProposal::get_all_proposed(
    //         &**tx,
    //         block.height().saturating_sub(NodeHeight(FOREIGN_PROPOSAL_TIMEOUT)),
    //     )?;
    //     for proposal in all_proposed {
    //         let mut has_unresolved_transactions = false;
    //
    //         let (transactions, _missing) = TransactionRecord::get_any(&**tx, &proposal.transactions)?;
    //         for transaction in transactions {
    //             if transaction.is_finalized() {
    //                 // We don't know the transaction at all, or we know it but it's not finalised.
    //                 let mut tx_rec = self
    //                     .transaction_pool
    //                     .get(&**tx, block.as_leaf_block(), transaction.id())?;
    //                 // If the transaction is still in the pool we have to check if it was at least locally prepared,
    //                 // otherwise abort it.
    //                 if tx_rec.current_stage().is_new() || tx_rec.current_stage().is_prepared() {
    //                     tx_rec.update_local_decision(tx, Decision::Abort)?;
    //                     has_unresolved_transactions = true;
    //                 }
    //             }
    //         }
    //         if !has_unresolved_transactions {
    //             proposal.delete(tx)?;
    //         }
    //     }
    //     Ok(())
    // }

    // TODO: fix
    // fn check_foreign_indexes(
    //     &self,
    //     tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
    //     num_committees: u32,
    //     local_shard: Shard,
    //     block: &Block,
    //     justify_block: &BlockId,
    // ) -> Result<(), HotStuffError> {
    //     let non_local_shards = proposer::get_non_local_shards(tx, block, num_committees, local_shard)?;
    //     let block_foreign_indexes = block.foreign_indexes();
    //     if block_foreign_indexes.len() != non_local_shards.len() {
    //         return Err(ProposalValidationError::InvalidForeignCounters {
    //             proposed_by: block.proposed_by().to_string(),
    //             hash: *block.id(),
    //             details: format!(
    //                 "Foreign indexes length ({}) does not match non-local shards length ({})",
    //                 block_foreign_indexes.len(),
    //                 non_local_shards.len()
    //             ),
    //         }
    //         .into());
    //     }
    //
    //     let mut foreign_counters = ForeignSendCounters::get_or_default(tx, justify_block)?;
    //     let mut current_shard = None;
    //     for (shard, foreign_count) in block_foreign_indexes {
    //         if let Some(current_shard) = current_shard {
    //             // Check ordering
    //             if current_shard > shard {
    //                 return Err(ProposalValidationError::InvalidForeignCounters {
    //                     proposed_by: block.proposed_by().to_string(),
    //                     hash: *block.id(),
    //                     details: format!(
    //                         "Foreign indexes are not sorted by shard. Current shard: {}, shard: {}",
    //                         current_shard, shard
    //                     ),
    //                 }
    //                 .into());
    //             }
    //         }
    //
    //         current_shard = Some(shard);
    //         // Check that each shard is correct
    //         if !non_local_shards.contains(shard) {
    //             return Err(ProposalValidationError::InvalidForeignCounters {
    //                 proposed_by: block.proposed_by().to_string(),
    //                 hash: *block.id(),
    //                 details: format!("Shard {} is not a non-local shard", shard),
    //             }
    //             .into());
    //         }
    //
    //         // Check that foreign counters are correct
    //         let expected_count = foreign_counters.increment_counter(*shard);
    //         if *foreign_count != expected_count {
    //             return Err(ProposalValidationError::InvalidForeignCounters {
    //                 proposed_by: block.proposed_by().to_string(),
    //                 hash: *block.id(),
    //                 details: format!(
    //                     "Foreign counter for shard {} is incorrect. Expected {}, got {}",
    //                     shard, expected_count, foreign_count
    //                 ),
    //             }
    //             .into());
    //         }
    //     }
    //
    //     Ok(())
    // }

    /// Perform final block validations (TODO: implement all validations)
    /// We assume at this point that initial stateless validations have been done (in inbound messages)
    #[allow(clippy::too_many_lines)]
    fn validate_local_proposed_block(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        candidate_block: Block,
        local_committee: &Committee<TConsensusSpec::Addr>,
        _local_committee_info: &CommitteeInfo,
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
                justify_block: candidate_block.justify().as_leaf_block(),
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

        if candidate_block.height() < justify_block.height() {
            return Err(ProposalValidationError::CandidateBlockNotHigherThanJustify {
                justify_block_height: justify_block.height(),
                candidate_block_height: candidate_block.height(),
            }
            .into());
        }

        // Special case for genesis block. A genesis block contains a genesis QC that does not justify anything, this is
        // the HIGH QC for the first block.
        if candidate_block.height() == NodeHeight(1) && candidate_block.justify().is_zero() {
            return Ok(ValidBlock::new(candidate_block));
        }

        // TODO: this is broken
        // self.check_foreign_indexes(
        //     tx,
        //     local_committee_info.num_committees(),
        //     local_committee_info.shard(),
        //     &candidate_block,
        //     justify_block.id(),
        // )?;

        // if the block parent is not the justify parent, then we have experienced a leader failure
        // and should make dummy blocks to fill in the gaps.
        if !candidate_block.justifies_parent() {
            let dummy_blocks =
                calculate_dummy_blocks(&candidate_block, &justify_block, &self.leader_strategy, local_committee);

            let Some(last_dummy) = dummy_blocks.last() else {
                warn!(target: LOG_TARGET, "❌ Bad proposal, does not justify parent for candidate block {}", candidate_block);
                return Err(ProposalValidationError::CandidateBlockDoesNotExtendJustify {
                    justify_block_height: justify_block.height(),
                    candidate_block_height: candidate_block.height(),
                }
                .into());
            };

            if candidate_block.parent() != last_dummy.id() {
                warn!(target: LOG_TARGET, "❌ Bad proposal, unable to find dummy blocks (last dummy: {}) for candidate block {}", last_dummy, candidate_block);
                return Err(ProposalValidationError::CandidateBlockDoesNotExtendJustify {
                    justify_block_height: justify_block.height(),
                    candidate_block_height: candidate_block.height(),
                }
                .into());
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

    async fn broadcast_foreign_proposal_if_required(
        &mut self,
        block: Block,
        justify_qc: QuorumCertificate,
    ) -> Result<(), HotStuffError> {
        let num_committees = self.epoch_manager.get_num_committees(block.epoch()).await?;

        let validator = self.epoch_manager.get_our_validator_node(block.epoch()).await?;
        let local_shard_group = validator
            .shard_key
            .to_shard_group(self.config.num_preshards, num_committees);
        let non_local_shard_groups = block
            .commands()
            .iter()
            .filter_map(|c| c.local_prepare().or_else(|| c.local_accept()))
            .flat_map(|p| p.evidence.substate_addresses_iter())
            .map(|addr| addr.to_shard_group(self.config.num_preshards, num_committees))
            .filter(|shard_group| local_shard_group != *shard_group)
            .collect::<HashSet<_>>();
        if non_local_shard_groups.is_empty() {
            return Ok(());
        }
        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING new locked block {} to {} foreign shard groups. justify: {} ({}), parent: {}",
            block,
            non_local_shard_groups.len(),
            justify_qc.block_id(),
            justify_qc.block_height(),
            block.parent()
        );
        debug!(
            target: LOG_TARGET,
            "non_local_shards : [{}]",
            non_local_shard_groups.iter().map(|s|s.to_string()).collect::<Vec<_>>().join(","),
        );

        let block_pledge = self
            .store
            .with_read_tx(|tx| block.get_block_pledge(tx))
            .optional()?
            .ok_or_else(|| HotStuffError::InvariantError(format!("Pledges not found for block {}", block)))?;

        // TODO(perf/message-size): the pledges for a given foreign proposal are not necessarily the same for each shard
        // group involved in the block. Currently we send all pledges to all shard groups but we could limit the
        // substates we send to validators to only those that are applicable to the transactions that involve
        // them.

        let mut addresses = HashSet::new();
        // TODO(perf): fetch only applicable committee addresses
        let mut committees = self.epoch_manager.get_committees(block.epoch()).await?;
        for shard_group in non_local_shard_groups {
            addresses.extend(
                committees
                    .remove(&shard_group)
                    .into_iter()
                    .flat_map(|c| c.into_iter().map(|(addr, _)| addr)),
            );
        }
        info!(
            target: LOG_TARGET,
            "🌿 FOREIGN PROPOSE: Broadcasting locked block {} with {} pledge(s) to {} foreign committees.",
            block,
            block_pledge.num_substates_pledged(),
            addresses.len(),
        );
        self.outbound_messaging
            .multicast(
                &addresses,
                HotstuffMessage::ForeignProposal(ForeignProposalMessage {
                    block,
                    block_pledge,
                    justify_qc,
                }),
            )
            .await?;
        Ok(())
    }
}
