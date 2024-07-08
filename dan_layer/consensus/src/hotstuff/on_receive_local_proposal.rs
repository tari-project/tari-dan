//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::{
    committee::{Committee, CommitteeInfo},
    optional::Optional,
    Epoch,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        Decision,
        ExecutedTransaction,
        ForeignProposal,
        HighQc,
        LastSentVote,
        QuorumDecision,
        TransactionAtom,
        TransactionPool,
        TransactionPoolStage,
        TransactionRecord,
        ValidBlock,
    },
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;
use tokio::sync::broadcast;

use super::proposer::Proposer;
use crate::{
    hotstuff::{
        current_view::CurrentView,
        error::HotStuffError,
        on_ready_to_vote_on_local_block::OnReadyToVoteOnLocalBlock,
        pacemaker_handle::PaceMakerHandle,
        HotstuffEvent,
        ProposalValidationError,
    },
    messages::{HotstuffMessage, ProposalMessage, VoteMessage},
    traits::{hooks::ConsensusHooks, ConsensusSpec, LeaderStrategy, OutboundMessaging, VoteSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_local_proposal";

pub struct OnReceiveLocalProposalHandler<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,
    network: Network,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    pacemaker: PaceMakerHandle,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    on_ready_to_vote_on_local_block: OnReadyToVoteOnLocalBlock<TConsensusSpec>,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    vote_signing_service: TConsensusSpec::SignatureService,
    proposer: Proposer<TConsensusSpec>,
    hooks: TConsensusSpec::Hooks,
}

impl<TConsensusSpec: ConsensusSpec> OnReceiveLocalProposalHandler<TConsensusSpec> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        local_validator_addr: TConsensusSpec::Addr,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        pacemaker: PaceMakerHandle,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        vote_signing_service: TConsensusSpec::SignatureService,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        proposer: Proposer<TConsensusSpec>,
        transaction_executor: TConsensusSpec::TransactionExecutor,
        network: Network,
        hooks: TConsensusSpec::Hooks,
    ) -> Self {
        Self {
            local_validator_addr: local_validator_addr.clone(),
            network,
            store: store.clone(),
            epoch_manager,
            leader_strategy,
            pacemaker,
            transaction_pool: transaction_pool.clone(),
            vote_signing_service,
            outbound_messaging,
            proposer,
            hooks,
            on_ready_to_vote_on_local_block: OnReadyToVoteOnLocalBlock::new(
                local_validator_addr,
                store,
                transaction_pool,
                tx_events,
                transaction_executor,
            ),
        }
    }

    pub async fn handle(&mut self, current_view: CurrentView, message: ProposalMessage) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;

        debug!(
            target: LOG_TARGET,
            "🔥 LOCAL PROPOSAL: block {} from {}",
            block,
            block.proposed_by()
        );

        match self.process_block(current_view, block).await {
            Ok(()) => Ok(()),
            Err(err @ HotStuffError::ProposalValidationError(_)) => {
                self.hooks.on_block_validation_failed(&err);
                Err(err)
            },
            Err(err) => Err(err),
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn process_block(&mut self, current_view: CurrentView, block: Block) -> Result<(), HotStuffError> {
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

            let Some(valid_block) = self.validate_block_header(tx, block, &local_committee, &local_committee_info)?
            else {
                return Ok(None);
            };

            // Ensure all transactions are inserted in the pool
            // TODO(hacky): If the block has transactions (invariant: we have the transaction stored at this point) but
            // it's not in the pool (race condition: transaction
            for tx_id in valid_block.block().all_transaction_ids() {
                if self.transaction_pool.exists(&**tx, tx_id)? {
                    continue;
                }
                let transaction = TransactionRecord::get(&**tx, tx_id)?;
                // Did the mempool execute it?
                if transaction.is_executed() {
                    // This should never fail
                    let executed = ExecutedTransaction::try_from(transaction)?;
                    self.transaction_pool.insert(tx, executed.to_atom())?;
                } else {
                    // Deferred execution
                    self.transaction_pool
                        .insert(tx, TransactionAtom::deferred(*transaction.id()))?;
                }
            }

            // Save the block as soon as it is valid to ensure we have a valid pacemaker height.
            let high_qc = self.save_block(tx, &valid_block)?;
            info!(target: LOG_TARGET, "✅ Block {} is valid and persisted. HighQc({})", valid_block, high_qc);
            Ok::<_, HotStuffError>(Some((high_qc, valid_block)))
        })?;

        if let Some((high_qc, valid_block)) = maybe_high_qc_and_block {
            let current_epoch = self.epoch_manager.current_epoch().await?;
            let can_propose_epoch_end = current_epoch > current_view.get_epoch();

            let block_decision = self.on_ready_to_vote_on_local_block.handle(
                &valid_block,
                local_committee_info,
                can_propose_epoch_end,
            )?;

            self.hooks
                .on_local_block_decide(&valid_block, block_decision.quorum_decision);
            for t in block_decision.finalized_transactions.into_iter().flatten() {
                self.hooks.on_transaction_finalized(&t);
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
                        // TODO: Change VN db to include the shard in the ValidatorNode struct.
                        let num_committees = self.epoch_manager.get_num_committees(next_epoch).await?;
                        let next_shard = vn.shard_key.to_shard(num_committees);
                        self.store.with_write_tx(|tx| {
                            let genesis = Block::genesis(self.network, next_epoch, next_shard);
                            info!(target: LOG_TARGET, "⭐️ Creating new genesis block {genesis}");
                            genesis.insert(tx)?;
                            // We'll propose using the new genesis as parent
                            genesis.as_locked_block().set(tx)?;
                            genesis.as_leaf_block().set(tx)?;
                            genesis.as_last_executed().set(tx)?;
                            genesis.as_last_voted().set(tx)?;
                            genesis.justify().as_high_qc().set(tx)
                        })?;

                        // Set the pacemaker to next epoch
                        self.pacemaker.set_epoch(next_epoch).await?;
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

    async fn propose_newly_locked_blocks(&mut self, blocks: Vec<Block>) -> Result<(), HotStuffError> {
        for block in blocks {
            debug!(target:LOG_TARGET,"Broadcast new locked block: {block}");
            let local_committee = self
                .epoch_manager
                .get_committee_by_validator_public_key(block.epoch(), block.proposed_by())
                .await?;
            let Some(our_addr) = self
                .epoch_manager
                .get_our_validator_node(block.epoch())
                .await
                .optional()?
            else {
                info!(
                    target: LOG_TARGET,
                    "❌ Our validator node is not registered for epoch {}. Not proposing {block} to foreign committee",
                    block.epoch(),
                );
                continue;
            };
            let leader_index = self.leader_strategy.calculate_leader(&local_committee, block.height());
            let my_index = local_committee
                .addresses()
                .position(|addr| *addr == our_addr.address)
                .ok_or_else(|| HotStuffError::InvariantError("Our address not found in local committee".to_string()))?;
            // There are other ways to approach this. But for simplicty is better just to make sure at least one honest
            // node will send it to the whole foreign committee. So we select the leader and f other nodes. It has to be
            // deterministic so we select by index (leader, leader+1, ..., leader+f). FYI: The messages between
            // committees and within committees are not different in terms of size, speed, etc.
            let diff_from_leader = (my_index + local_committee.len() - leader_index as usize) % local_committee.len();
            // f+1 nodes (always including the leader) send the proposal to the foreign committee
            // if diff_from_leader <= (local_committee.len() - 1) / 3 + 1 {
            if diff_from_leader <= local_committee.len() / 3 {
                self.proposer.broadcast_foreign_proposal_if_required(block).await?;
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
            .get_validator_node(block.epoch(), &self.local_validator_addr)
            .await?;
        let leaf_hash = vn.get_node_hash(self.network);

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
        valid_block.block().save(tx)?;

        let high_qc = valid_block.block().justify().update_high_qc(tx)?;
        Ok(high_qc)
    }

    fn validate_block_header(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: Block,
        local_committee: &Committee<TConsensusSpec::Addr>,
        local_committee_info: &CommitteeInfo,
    ) -> Result<Option<ValidBlock>, HotStuffError> {
        let result = self
            .validate_local_proposed_block(&**tx, block, local_committee, local_committee_info)
            .and_then(|valid_block| {
                // TODO: This should be moved out of validate_block_header. Then tx can be a read transaction
                self.update_foreign_proposal_transactions(tx, valid_block.block())?;
                Ok(valid_block)
            });

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

    fn update_foreign_proposal_transactions(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
    ) -> Result<(), HotStuffError> {
        // TODO: Move this to consensus constants
        const FOREIGN_PROPOSAL_TIMEOUT: u64 = 1000;
        let all_proposed = ForeignProposal::get_all_proposed(
            &**tx,
            block.height().saturating_sub(NodeHeight(FOREIGN_PROPOSAL_TIMEOUT)),
        )?;
        for proposal in all_proposed {
            let mut has_unresolved_transactions = false;

            let (transactions, _missing) = TransactionRecord::get_any(&**tx, &proposal.transactions)?;
            for transaction in transactions {
                if transaction.is_finalized() {
                    // We don't know the transaction at all, or we know it but it's not finalised.
                    let mut tx_rec = self
                        .transaction_pool
                        .get(&**tx, block.as_leaf_block(), transaction.id())?;
                    // If the transaction is still in the pool we have to check if it was at least locally prepared,
                    // otherwise abort it.
                    if tx_rec.stage() == TransactionPoolStage::New || tx_rec.stage() == TransactionPoolStage::Prepared {
                        tx_rec.update_local_decision(tx, Decision::Abort)?;
                        has_unresolved_transactions = true;
                    }
                }
            }
            if !has_unresolved_transactions {
                proposal.delete(tx)?;
            }
        }
        Ok(())
    }

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
        local_committee_info: &CommitteeInfo,
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

        let justify_block_height = justify_block.height();
        // if the block parent is not the justify parent, then we have experienced a leader failure
        // and should make dummy blocks to fill in the gaps.
        if justify_block.id() != candidate_block.parent() {
            let mut dummy_blocks =
                Vec::with_capacity((candidate_block.height().as_u64() - justify_block_height.as_u64() - 1) as usize);
            let timestamp = justify_block.timestamp();
            let base_layer_block_height = justify_block.base_layer_block_height();
            let base_layer_block_hash = *justify_block.base_layer_block_hash();
            dummy_blocks.push(justify_block);
            let mut last_dummy_block = dummy_blocks.last().unwrap();

            while last_dummy_block.id() != candidate_block.parent() {
                if last_dummy_block.height() > candidate_block.height() {
                    warn!(target: LOG_TARGET, "❌ Bad proposal, unable to find dummy blocks (last dummy: {}) for candidate block {}", last_dummy_block, candidate_block);
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
                    local_committee_info.shard(),
                    *candidate_block.merkle_root(),
                    timestamp,
                    base_layer_block_height,
                    base_layer_block_hash,
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
