//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// (New, true) ----(cmd:Prepare) ---> (Prepared, true) -----cmd:LocalPrepared ---> (LocalPrepared, false)
// ----[foreign:LocalPrepared]--->(LocalPrepared, true) ----cmd:AllPrepare ---> (AllPrepared, true) ---cmd:Accept --->
// Complete

use std::{collections::HashSet, num::NonZeroU64, ops::DerefMut};

use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    NodeHeight,
    ShardId,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Command,
        Decision,
        ExecutedTransaction,
        HighQc,
        LastExecuted,
        LastVoted,
        LeafBlock,
        LockedBlock,
        LockedOutput,
        QuorumDecision,
        SubstateLockFlag,
        SubstateRecord,
        TransactionPool,
        TransactionPoolStage,
        TransactionRecord,
    },
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{broadcast, mpsc};

use crate::{
    hotstuff::{
        common::{BlockDecision, EXHAUST_DIVISOR},
        error::HotStuffError,
        event::HotstuffEvent,
        pacemaker_handle::PaceMakerHandle,
        ProposalValidationError,
    },
    messages::{HotstuffMessage, ProposalMessage, RequestMissingTransactionsMessage, VoteMessage},
    traits::{ConsensusSpec, LeaderStrategy, StateManager, VoteSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_proposal";

pub struct OnReceiveProposalHandler<TConsensusSpec: ConsensusSpec> {
    validator_addr: TConsensusSpec::Addr,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signing_service: TConsensusSpec::VoteSignatureService,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    state_manager: TConsensusSpec::StateManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    tx_events: broadcast::Sender<HotstuffEvent>,
    pacemaker: PaceMakerHandle,
}

impl<TConsensusSpec> OnReceiveProposalHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        validator_addr: TConsensusSpec::Addr,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signing_service: TConsensusSpec::VoteSignatureService,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        state_manager: TConsensusSpec::StateManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        pacemaker: PaceMakerHandle,
    ) -> Self {
        Self {
            validator_addr,
            store,
            epoch_manager,
            vote_signing_service,
            leader_strategy,
            state_manager,
            transaction_pool,
            tx_leader,
            tx_events,
            pacemaker,
        }
    }

    pub async fn handle(
        &self,
        from: TConsensusSpec::Addr,
        message: ProposalMessage<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;

        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        if local_committee.contains(&from) {
            debug!(
                target: LOG_TARGET,
                "🔥 Receive LOCAL PROPOSAL for block {}, parent {}, height {} from {}",
                block.id(),
                block.parent(),
                block.height(),
                from,
            );

            self.handle_local_proposal(from, local_committee, block).await
        } else {
            debug!(
                target: LOG_TARGET,
                "🔥 Receive FOREIGN PROPOSAL for block {}, parent {}, height {} from {}",
                block.id(),
                block.parent(),
                block.height(),
                from,
            );

            self.handle_foreign_proposal(from, block).await
        }
    }

    async fn handle_local_proposal(
        &self,
        from: TConsensusSpec::Addr,
        local_committee: Committee<TConsensusSpec::Addr>,
        block: Block<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(block.epoch()).await?;

        // First save the block in one db transaction
        let (missing_tx_ids, awaiting_execution) = {
            let mut tx = self.store.create_write_tx()?;
            if let Err(err) =
                self.validate_local_proposed_block_and_fill_dummy_blocks(&mut tx, &from, &block, &local_committee)
            {
                match err {
                    err @ HotStuffError::ProposalValidationError(ProposalValidationError::JustifyBlockNotFound {
                        ..
                    }) => {
                        tx.rollback()?;
                        return Err(err);
                    },
                    HotStuffError::ProposalValidationError(err) => {
                        warn!(target: LOG_TARGET, "❌ Block failed validation: {}", err);
                        // A bad block should not cause a FAILURE state transition
                        tx.rollback()?;
                        return Ok(());
                    },
                    e => {
                        tx.rollback()?;
                        return Err(e);
                    },
                }
            }
            // Now that we have all dummy blocks (if any) in place, we can check if the candidate block is safe.
            // Specifically, it should extend the locked block via the dummy blocks.
            if !block.is_safe(tx.deref_mut())? {
                tx.rollback()?;
                let err = ProposalValidationError::NotSafeBlock {
                    proposed_by: from.to_string(),
                    hash: *block.id(),
                };
                warn!(target: LOG_TARGET, "❌ Block failed validation: {}", err);
                return Ok(());
            }

            // Insert the block if it doesnt already exist
            block.justify().save(&mut tx)?;
            if block.save(&mut tx)? {
                debug!(
                    target: LOG_TARGET,
                    "🔥 Block {} saved.", block
                );
            }

            let missing_tuple = self.handle_missing_transaction(&mut tx, &block)?;
            tx.commit()?;
            missing_tuple
        };

        let high_qc = self.store.with_read_tx(|tx| HighQc::get(tx))?;

        if !missing_tx_ids.is_empty() {
            self.send_to_leader(
                &local_committee,
                block.height(),
                HotstuffMessage::RequestMissingTransactions(RequestMissingTransactionsMessage {
                    block_id: *block.id(),
                    epoch: block.epoch(),
                    transactions: missing_tx_ids,
                }),
            )
            .await;
            self.pacemaker
                .reset_leader_timeout(block.height(), high_qc.block_height())
                .await?;
            return Ok(());
        }

        if awaiting_execution.is_empty() {
            if let Some(decision) = self.decide_on_block(&local_committee_shard, &block)? {
                let vote = self.generate_vote_message(&block, decision).await?;
                let high_qc = self.store.with_write_tx(|tx| {
                    block.as_last_voted().set(tx)?;
                    HighQc::get(tx.deref_mut())
                })?;
                self.pacemaker
                    .reset_leader_timeout(block.height(), high_qc.block_height())
                    .await?;
                self.send_vote_to_leader(&local_committee, vote, block.height()).await;
            }
        } else {
            self.pacemaker
                .reset_leader_timeout(block.height(), high_qc.block_height())
                .await?;
        }

        Ok(())
    }

    fn handle_missing_transaction(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
    ) -> Result<(HashSet<TransactionId>, HashSet<TransactionId>), HotStuffError> {
        let (transactions, missing_tx_ids) = TransactionRecord::get_any(tx.deref_mut(), block.all_transaction_ids())?;
        let awaiting_execution = transactions
            .into_iter()
            .filter(|tx| tx.result.is_none())
            .map(|tx| *tx.transaction.id())
            .collect::<HashSet<_>>();

        if missing_tx_ids.is_empty() && awaiting_execution.is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} has no missing transactions", block
            );
            return Ok((HashSet::new(), HashSet::new()));
        }

        info!(
            target: LOG_TARGET,
            "🔥 Block {} has {} missing transactions and {} awaiting execution", block, missing_tx_ids.len(), awaiting_execution.len(),
        );

        tx.insert_missing_transactions(block.id(), &missing_tx_ids, &awaiting_execution)?;

        Ok((missing_tx_ids, awaiting_execution))
    }

    pub async fn reprocess_block(&self, block_id: &BlockId) -> Result<(), HotStuffError> {
        let block = self.store.with_read_tx(|tx| Block::get(tx, block_id))?;

        if !self.epoch_manager.is_epoch_active(block.epoch()).await? {
            return Err(HotStuffError::EpochNotActive {
                epoch: block.epoch(),
                details: "Cannot reprocess block from inactive epoch".to_string(),
            });
        }

        info!(target: LOG_TARGET, "♻️ Reprocessing block {block} after all transactions have been executed");

        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        self.handle_local_proposal(block.proposed_by().clone(), local_committee, block)
            .await?;

        Ok(())
    }

    fn decide_on_block(
        &self,
        local_committee_shard: &CommitteeShard,
        block: &Block<TConsensusSpec::Addr>,
    ) -> Result<Option<QuorumDecision>, HotStuffError> {
        self.store.with_write_tx(|tx| {
            let mut decision = BlockDecision::vote_accept();
            if self.should_vote(tx.deref_mut(), block)? {
                decision = self.decide_what_to_vote(tx, block, local_committee_shard)?;
            } else {
                decision.dont_vote();
            }

            if decision.is_accept() {
                let high_qc = block.update_nodes(
                    tx,
                    |tx, locked, block| self.on_lock_block(tx, locked, block),
                    |tx, last_exec, commit_block| self.on_commit(tx, last_exec, commit_block, local_committee_shard),
                )?;

                // If we have a new high QC, we'll process the block it justifies
                self.process_new_leaf(tx, high_qc, block, local_committee_shard)?;
            }

            Ok::<_, HotStuffError>(decision.as_quorum_decision())
        })
    }

    fn process_new_leaf(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        high_qc: HighQc,
        tip_block: &Block<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        let leaf = high_qc.get_block(tx.deref_mut())?;
        if leaf.is_processed() {
            return Ok(());
        }

        for cmd in leaf.commands() {
            match cmd {
                Command::Prepare(t) => {
                    let mut tx_rec = self.transaction_pool.get(tx, tip_block.as_leaf_block(), t.id())?;

                    if t.decision.is_commit() {
                        let transaction = ExecutedTransaction::get(tx.deref_mut(), cmd.transaction_id())?;
                        // Lock all inputs for the transaction as part of Prepare
                        let is_inputs_locked =
                            self.lock_inputs(tx, transaction.transaction(), local_committee_shard)?;
                        let is_outputs_locked = is_inputs_locked && self.lock_outputs(tx, leaf.id(), &transaction)?;

                        if !is_inputs_locked {
                            // Unable to lock all inputs - do not vote
                            warn!(
                                target: LOG_TARGET,
                                "❌ Unable to lock all inputs for transaction {} in block {}. Leader proposed {}, we decided {}",
                                leaf.id(),
                                transaction.id(),
                                t.decision,
                                Decision::Abort
                            );
                            // We change our decision to ABORT so that the next time we propose/receive a
                            // proposal we will check for ABORT. It may
                            // happen that the transaction causing the lock failure
                            // is ABORTED too and the locks released allowing this transaction to succeed.
                            // Currently, the client would have to resubmit the transaction to resolve this.
                            tx_rec.update_local_decision(tx, Decision::Abort)?;

                            // The leader should not have proposed conflicting transactions
                        } else if !is_outputs_locked {
                            // Unable to lock all outputs - do not vote
                            warn!(
                                target: LOG_TARGET,
                                "❌ Unable to lock all outputs for transaction {} in block {}. Leader proposed {}, we decided {}",
                                leaf.id(),
                                transaction.id(),
                                t.decision,
                                Decision::Abort
                            );
                            // Unlock any locked inputs because we are not voting
                            self.unlock_inputs(tx, transaction.transaction(), local_committee_shard)?;
                            // We change our decision to ABORT so that the next time we propose/receive a
                            // proposal we will check for ABORT
                            tx_rec.update_local_decision(tx, Decision::Abort)?;
                        } else {
                            // We have locked all inputs and outputs
                        }
                    }
                },
                Command::LocalPrepared(t) => {
                    let mut tx_rec = self.transaction_pool.get(tx, tip_block.as_leaf_block(), t.id())?;

                    debug!(
                        target: LOG_TARGET,
                        "🔥 Process NEW leaf block: Update local proposal for transaction: {}. Local stage: {}, Leaf: {}",
                        tx_rec.transaction_id(),
                        tx_rec.current_stage(),
                        leaf,
                    );

                    // If all shards are complete and we've already received our LocalPrepared, we can set the
                    // LocalPrepared transaction as ready to propose ACCEPT.
                    if tx_rec.current_stage().is_local_prepared() && tx_rec.transaction().evidence.all_shards_complete()
                    {
                        info!(
                            target: LOG_TARGET,
                            "🔥 Process NEW leaf block: Transaction is ready for propose ACCEPT({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );
                        tx_rec.add_pending_status_update(
                            tx,
                            leaf.as_leaf_block(),
                            TransactionPoolStage::LocalPrepared,
                            true,
                        )?;
                    }
                },
                Command::Accept(_) => {},
            }
        }
        leaf.set_as_processed(tx)?;
        Ok(())
    }

    /// if b_new .height > vheight && (b_new extends b_lock || b_new .justify.node.height > b_lock .height)
    ///
    /// If we have not previously voted on this block and the node extends the current locked node, then we vote
    fn should_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
    ) -> Result<bool, ProposalValidationError> {
        let Some(last_voted) = LastVoted::get(tx).optional()? else {
            // Never voted, then validated.block.height() > last_voted.height (0)
            return Ok(true);
        };

        // if b_new .height > vheight And ...
        if block.height() <= last_voted.height {
            info!(
                target: LOG_TARGET,
                "❌ NOT voting on block {}, height {}. Block height is not greater than last voted height {}",
                block.id(),
                block.height(),
                last_voted.height,
            );
            return Ok(false);
        }

        Ok(true)
    }

    async fn handle_foreign_proposal(
        &self,
        from: TConsensusSpec::Addr,
        block: Block<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let vn = self.epoch_manager.get_validator_node(block.epoch(), &from).await?;
        let committee_shard = self
            .epoch_manager
            .get_committee_shard(block.epoch(), vn.shard_key)
            .await?;
        self.validate_proposed_block(&from, &block)?;
        self.store
            .with_write_tx(|tx| self.on_receive_foreign_block(tx, &block, &committee_shard))?;

        // We could have ready transactions at this point, so if we're the leader for the next block we can propose
        self.pacemaker.beat();

        Ok(())
    }

    fn on_receive_foreign_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
        foreign_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        let leaf = LeafBlock::get(tx.deref_mut())?;
        // We only want to save the QC once if applicable
        let mut is_qc_saved = false;

        for cmd in block.commands() {
            let Some(t) = cmd.local_prepared() else {
                continue;
            };
            let Some(mut tx_rec) = self.transaction_pool.get(tx, leaf, &t.id).optional()? else {
                continue;
            };

            if tx_rec.current_stage().is_all_prepared() || tx_rec.current_stage().is_some_prepared() {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Foreign proposal received after transaction {} is {}. Ignoring.",
                    tx_rec.transaction_id(), tx_rec.current_stage()
                );
                continue;
            }

            let remote_decision = cmd.decision();
            let local_decision = tx_rec.current_local_decision();
            if remote_decision.is_abort() && local_decision.is_commit() {
                info!(
                    target: LOG_TARGET,
                    "⚠️ Foreign shard ABORT {}. Update overall decision to ABORT. Local stage: {}, Leaf: {}",
                    tx_rec.transaction_id(), tx_rec.current_stage(), leaf
                );
            }

            if !is_qc_saved {
                // Save the QCs if it doesnt exist already, we'll reference the QC in subsequent blocks
                block.justify().save(tx)?;
                is_qc_saved = true;
            }

            tx_rec.update_remote_data(tx, remote_decision, *block.justify().id(), foreign_committee_shard)?;

            // If all shards are complete and we've already received our LocalPrepared, we can set out LocalPrepared
            // transaction as ready to propose ACCEPT. If we have not received the local LocalPrepared, the transition
            // will happen when we receive the local block.
            if tx_rec.current_stage().is_local_prepared() && tx_rec.transaction().evidence.all_shards_complete() {
                info!(
                    target: LOG_TARGET,
                    "🔥 FOREIGN PROPOSAL: Transaction is ready for propose ACCEPT({}, {}) Local Stage: {}",
                    tx_rec.transaction_id(),
                    tx_rec.current_decision(),
                    tx_rec.current_stage()
                );

                tx_rec.add_pending_status_update(tx, leaf, TransactionPoolStage::LocalPrepared, true)?;
            }
        }

        Ok(())
    }

    async fn send_to_leader(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        height: NodeHeight,
        message: HotstuffMessage<TConsensusSpec::Addr>,
    ) {
        let leader = self.leader_strategy.get_leader(local_committee, height);
        if self.tx_leader.send((leader.clone(), message)).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "tx_leader in OnReceiveProposalHandler::send_to_leader is closed",
            );
        }
    }

    async fn send_vote_to_leader(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        vote: VoteMessage<TConsensusSpec::Addr>,
        height: NodeHeight,
    ) {
        let leader = self.leader_strategy.get_leader_for_next_block(local_committee, height);
        info!(
            target: LOG_TARGET,
            "🔥 VOTE {:?} for block {} to next leader {:.4}",
            vote.decision,
            vote.block_id,
            leader,
        );
        if self
            .tx_leader
            .send((leader.clone(), HotstuffMessage::Vote(vote)))
            .await
            .is_err()
        {
            debug!(
                target: LOG_TARGET,
                "tx_leader in OnReceiveProposalHandler::send_vote_to_leader is closed",
            );
        }
    }

    #[allow(clippy::too_many_lines)]
    fn decide_what_to_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<BlockDecision, HotStuffError> {
        let mut total_leader_fee = 0;
        let mut decision = BlockDecision::vote_accept();
        let mut locked_inputs = HashSet::new();
        let mut locked_outputs = HashSet::new();
        for cmd in block.commands() {
            let Some(mut tx_rec) = self
                .transaction_pool
                .get(tx, block.as_leaf_block(), cmd.transaction_id())
                .optional()?
            else {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                    block,
                    cmd.transaction_id(),
                );
                decision.dont_vote();
                return Ok(decision);
            };

            // TODO: we probably need to provide the all/some of the QCs referenced in local transactions as
            //       part of the proposal DanMessage so that there is no race condition between receiving the
            //       proposed block and receiving the foreign proposals. Because this is only added on locked block,
            //       this should be less common.
            tx_rec.add_evidence(local_committee_shard, *block.justify().id());

            debug!(
                target: LOG_TARGET,
                "🔥 processing command {} for block {}",
                cmd,
                block,
            );
            match cmd {
                Command::Prepare(t) => {
                    if !tx_rec.current_stage().is_new() && !tx_rec.current_stage().is_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for tx {} in block {}. Leader proposed Prepare, local stage is {}",
                            tx_rec.transaction_id(),
                            block.id(),
                            tx_rec.current_stage(),
                        );
                        decision.dont_vote();
                        continue;
                    }

                    if tx_rec.transaction().transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block.id(),
                            t.transaction_fee,
                            tx_rec.transaction().transaction_fee
                        );
                        decision.dont_vote();
                        continue;
                    }

                    if tx_rec.current_decision() == t.decision {
                        if tx_rec.current_decision().is_commit() {
                            let transaction = ExecutedTransaction::get(tx.deref_mut(), cmd.transaction_id())?;
                            // Lock all inputs for the transaction as part of Prepare
                            let is_inputs_locked = self.check_lock_inputs(
                                tx,
                                transaction.transaction(),
                                local_committee_shard,
                                &mut locked_inputs,
                            )?;
                            let is_outputs_locked =
                                is_inputs_locked && self.check_lock_outputs(tx, &transaction, &mut locked_outputs)?;

                            if !is_inputs_locked {
                                // Unable to lock all inputs - do not vote
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Unable to lock all inputs for transaction {} in block {}.",
                                    block.id(),
                                    transaction.id(),
                                );
                                // We change our decision to ABORT so that the next time we propose/receive a
                                // proposal we will check for ABORT. It may
                                // happen that the transaction causing the lock failure
                                // is ABORTED too and the locks released allowing this transaction to succeed.
                                // Currently, the client would have to resubmit the transaction to resolve this.
                                tx_rec.update_local_decision(tx, Decision::Abort)?;

                                // The leader should not have proposed conflicting transactions
                                decision.dont_vote();
                                return Ok(decision);
                            } else if !is_outputs_locked {
                                // Unable to lock all outputs - do not vote
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Unable to lock all outputs for transaction {} in block {}.",
                                    block.id(),
                                    transaction.id(),
                                );
                                // We change our decision to ABORT so that the next time we propose/receive a
                                // proposal we will check for ABORT
                                tx_rec.update_local_decision(tx, Decision::Abort)?;
                                decision.dont_vote();
                                return Ok(decision);
                            } else {
                                // We have locked all inputs and outputs
                            }
                        }

                        if decision.is_accept() {
                            tx_rec.add_pending_status_update(
                                tx,
                                block.as_leaf_block(),
                                TransactionPoolStage::Prepared,
                                true,
                            )?;
                        }
                    } else {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for tx {} in block {}. Leader proposed {}, we decided {}",
                            tx_rec.transaction_id(),
                            block.id(),
                            t.decision,
                            tx_rec.current_decision()
                        );
                        decision.dont_vote();
                    }
                },
                Command::LocalPrepared(t) => {
                    // Happy path: We've validated all the QCs and therefore are convinced that everyone also Prepared.
                    // We only mark the next step (Accept) as ready to propose once all shards have reported
                    // LocalPrepared.

                    if !tx_rec.current_stage().is_prepared() && !tx_rec.current_stage().is_local_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "{} ❌ Stage disagreement in block {} for transaction {}. Leader proposed LocalPrepared, but local stage is {}",
                            self.validator_addr,
                            block.id(),
                            tx_rec.transaction_id(),
                            tx_rec.current_stage()
                        );
                        decision.dont_vote();
                    }
                    // We check that the leader decision is the same as our local decision.
                    // We disregard the remote decision because not all validators may have received the foreign
                    // LocalPrepared yet. We will never accept a decision disagreement for the Accept command.
                    if tx_rec.current_local_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ LocalPrepared decision disagreement for transaction {} in block {}. Leader proposed {}, we decided {}",
                            tx_rec.transaction_id(),
                            block.id(),
                            t.decision,
                            tx_rec.current_local_decision()
                        );

                        decision.dont_vote();
                    }

                    if tx_rec.transaction().transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                            tx_rec.transaction_id(),
                            block.id(),
                            t.transaction_fee,
                            tx_rec.transaction().transaction_fee
                        );
                        decision.dont_vote();
                    }

                    if decision.is_accept() {
                        tx_rec.add_pending_status_update(
                            tx,
                            block.as_leaf_block(),
                            TransactionPoolStage::LocalPrepared,
                            tx_rec.transaction().evidence.all_shards_complete(),
                        )?;
                    }
                },
                Command::Accept(t) => {
                    // Happy path: We've validated all the QCs and therefore are convinced that everyone also received
                    // LocalPrepare. We then propose new blocks until we have a 3-chain
                    if !tx_rec.current_stage().is_local_prepared() && !tx_rec.current_stage().is_accepted() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for tx {} in block {}. Leader proposed Accept, local stage {}",
                            tx_rec.transaction_id(),
                            block.id(),
                            tx_rec.current_stage(),
                        );
                        decision.dont_vote();
                        continue;
                    }
                    if tx_rec.current_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept decision disagreement tx {} in for block {}. Leader proposed {}, we decided {}",
                            tx_rec.transaction_id(),
                            block.id(),
                            t.decision,
                            tx_rec.current_decision()
                        );
                        decision.dont_vote();
                        continue;
                    }

                    if !tx_rec.transaction().evidence.all_shards_complete() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept evidence disagreement tx {} in block {}. Evidence for {} out of {} shards",
                            tx_rec.transaction_id(),
                            block.id(),
                            tx_rec.transaction().evidence.num_complete_shards(),
                            tx_rec.transaction().evidence.len(),
                        );
                        decision.dont_vote();
                        continue;
                    }

                    if tx_rec.transaction().transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                            tx_rec.transaction_id(),
                            block.id(),
                            t.transaction_fee,
                            tx_rec.transaction().transaction_fee
                        );

                        decision.dont_vote();
                        continue;
                    }

                    // Check if we have LocalPrepared ready i.e. LocalPrepared from all shards
                    // It is possible that the transaction was not marked as ready yet because of the order we received
                    // messages, but if we are in LocalPrepared and we have all the evidence, we would have proposed
                    // this too so we can continue.
                    if !tx_rec.is_ready() && !tx_rec.transaction().evidence.all_shards_complete() {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ Local proposal received ({}) for transaction {} which is not ready. Not voting.",
                            block,
                            tx_rec.transaction()
                        );
                        decision.dont_vote();
                        continue;
                    }

                    let distinct_shards =
                        local_committee_shard.count_distinct_buckets(tx_rec.transaction().evidence.shards_iter());
                    let distinct_shards = NonZeroU64::new(distinct_shards as u64).ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Distinct shards is zero for transaction {} in block {}",
                            tx_rec.transaction_id(),
                            block.id()
                        ))
                    })?;
                    let calculated_leader_fee = tx_rec.calculate_leader_fee(distinct_shards, EXHAUST_DIVISOR);
                    if calculated_leader_fee != t.leader_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block.id(),
                            t.leader_fee,
                            calculated_leader_fee
                        );

                        decision.dont_vote();
                        continue;
                    }
                    total_leader_fee += calculated_leader_fee;
                    // If the decision was changed to Abort, which can only happen when a foreign shard decides ABORT
                    // and we decide COMMIT, we set SomePrepared, otherwise AllPrepared. There are no further stages
                    // after these, so these MUST never be ready to propose.
                    if decision.is_accept() {
                        if tx_rec.remote_decision().map(|d| d.is_abort()).unwrap_or(false) {
                            tx_rec.add_pending_status_update(
                                tx,
                                block.as_leaf_block(),
                                TransactionPoolStage::SomePrepared,
                                false,
                            )?;
                        } else {
                            tx_rec.add_pending_status_update(
                                tx,
                                block.as_leaf_block(),
                                TransactionPoolStage::AllPrepared,
                                false,
                            )?;
                        }
                    }
                },
            }
        }

        // If we decided not to vote, the total_leader_fee may be incorrectly summed up.
        if decision.is_accept() && total_leader_fee != block.total_leader_fee() {
            warn!(
                target: LOG_TARGET,
                "❌ Leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                block.id(),
                block.total_leader_fee(),
                total_leader_fee
            );
            decision.dont_vote();
        }

        Ok(decision)
    }

    fn lock_inputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &Transaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<bool, HotStuffError> {
        let state = SubstateRecord::try_lock_all(
            tx,
            transaction.id(),
            local_committee_shard.filter(transaction.inputs().iter().chain(transaction.filled_inputs())),
            SubstateLockFlag::Write,
        )?;
        if !state.is_acquired() {
            warn!(
                target: LOG_TARGET,
                "❌ Unable to write lock all inputs for transaction {}: {:?}",
                transaction.id(),
                state,
            );
            return Ok(false);
        }
        let state = SubstateRecord::try_lock_all(
            tx,
            transaction.id(),
            local_committee_shard.filter(transaction.input_refs()),
            SubstateLockFlag::Read,
        )?;

        if !state.is_acquired() {
            warn!(
                target: LOG_TARGET,
                "❌ Unable to read lock all input refs for transaction {}: {:?}",
                transaction.id(),
                state,
            );
            return Ok(false);
        }

        debug!(
            target: LOG_TARGET,
            "🔒️ Locked inputs for transaction {}",
            transaction.id(),
        );

        Ok(true)
    }

    fn check_lock_inputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        transaction: &Transaction,
        local_committee_shard: &CommitteeShard,
        locked_inputs: &mut HashSet<ShardId>,
    ) -> Result<bool, HotStuffError> {
        let inputs = local_committee_shard
            .filter(transaction.inputs().iter().chain(transaction.filled_inputs()))
            .copied()
            .collect::<HashSet<_>>();
        let state = SubstateRecord::check_lock_all(tx, inputs.iter(), SubstateLockFlag::Write)?;
        if !state.is_acquired() {
            warn!(
                target: LOG_TARGET,
                "❌ Unable to write lock all inputs for transaction {}: {:?}",
                transaction.id(),
                state,
            );
            return Ok(false);
        }
        if inputs.iter().any(|i| locked_inputs.contains(i)) {
            warn!(
                target: LOG_TARGET,
                "❌ Locks for transaction {} conflict with other transactions in the block",
                transaction.id(),
            );
            return Ok(false);
        }
        locked_inputs.extend(inputs);

        let state = SubstateRecord::check_lock_all(
            tx,
            local_committee_shard.filter(transaction.input_refs()),
            SubstateLockFlag::Read,
        )?;

        if !state.is_acquired() {
            warn!(
                target: LOG_TARGET,
                "❌ Unable to read lock all input refs for transaction {}: {:?}",
                transaction.id(),
                state,
            );
            return Ok(false);
        }

        debug!(
            target: LOG_TARGET,
            "🔒️ Locked inputs for transaction {}",
            transaction.id(),
        );

        Ok(true)
    }

    fn unlock_inputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &Transaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        SubstateRecord::try_unlock_many(
            tx,
            transaction.id(),
            local_committee_shard.filter(transaction.inputs().iter().chain(transaction.filled_inputs())),
            SubstateLockFlag::Write,
        )?;
        SubstateRecord::try_unlock_many(
            tx,
            transaction.id(),
            local_committee_shard.filter(transaction.input_refs()),
            SubstateLockFlag::Read,
        )?;
        Ok(())
    }

    fn lock_outputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block_id: &BlockId,
        transaction: &ExecutedTransaction,
    ) -> Result<bool, HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "Acquiring {} output locks for block `{}` and transaction `{}`",
            transaction.resulting_outputs().len(),
            block_id,
            transaction.id(),
        );

        let state = LockedOutput::try_acquire_all(tx, block_id, transaction.id(), transaction.resulting_outputs())?;

        if !state.is_acquired() {
            return Ok(false);
        }

        Ok(true)
    }

    fn check_lock_outputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        transaction: &ExecutedTransaction,
        locked_outputs: &mut HashSet<ShardId>,
    ) -> Result<bool, HotStuffError> {
        let state = LockedOutput::check_locks(tx, transaction.resulting_outputs())?;

        if !state.is_acquired() {
            return Ok(false);
        }

        if transaction
            .resulting_outputs()
            .iter()
            .any(|i| locked_outputs.contains(i))
        {
            warn!(
                target: LOG_TARGET,
                "❌ Locks for transaction {} conflict with other transactions in the block",
                transaction.id(),
            );
            return Ok(false);
        }
        locked_outputs.extend(transaction.resulting_outputs());

        Ok(true)
    }

    fn unlock_outputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &ExecutedTransaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        LockedOutput::try_release_all(tx, local_committee_shard.filter(transaction.resulting_outputs()))?;
        Ok(())
    }

    async fn generate_vote_message(
        &self,
        block: &Block<TConsensusSpec::Addr>,
        decision: QuorumDecision,
    ) -> Result<VoteMessage<TConsensusSpec::Addr>, HotStuffError> {
        let merkle_proof = self
            .epoch_manager
            .get_validator_node_merkle_proof(block.epoch())
            .await?;
        let vn = self
            .epoch_manager
            .get_validator_node(block.epoch(), &self.validator_addr)
            .await?;
        let leaf_hash = vn.node_hash();

        let signature = self.vote_signing_service.sign_vote(&leaf_hash, block.id(), &decision);

        Ok(VoteMessage {
            epoch: block.epoch(),
            block_id: *block.id(),
            decision,
            signature,
            merkle_proof,
        })
    }

    fn on_commit(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        last_executed: &LastExecuted,
        block: &Block<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        if last_executed.height < block.height() {
            let parent = block.get_parent(tx.deref_mut())?;
            // Recurse to "catch up" any parent parent blocks we may not have executed
            self.on_commit(tx, last_executed, &parent, local_committee_shard)?;
            self.execute(tx, block, local_committee_shard)?;
            debug!(
                target: LOG_TARGET,
                "✅ COMMIT block {}, last executed height = {}",
                block,
                last_executed.height
            );
            self.publish_event(HotstuffEvent::BlockCommitted {
                block_id: *block.id(),
                height: block.height(),
            });
        }
        Ok(())
    }

    fn on_lock_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        locked: &LockedBlock,
        block: &Block<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        if locked.height < block.height() {
            info!(
                target: LOG_TARGET,
                "🔒️ LOCKED BLOCK: {} {}",
                block.height(),
                block.id()
            );

            let parent = block.get_parent(tx.deref_mut())?;
            self.on_lock_block(tx, locked, &parent)?;

            // self.processed_locked_commands(tx, local_committee_shard, block)?;
            // This moves the stage update from pending to current for all transactions on on the locked block
            self.transaction_pool.confirm_all_transitions(
                tx,
                locked,
                &block.as_locked_block(),
                block.all_transaction_ids(),
            )?;
        }
        Ok(())
    }

    fn publish_event(&self, event: HotstuffEvent) {
        let _ignore = self.tx_events.send(event);
    }

    fn execute(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        let mut total_transaction_fee = 0;
        let mut total_fee_due = 0;
        for cmd in block.commands() {
            match cmd {
                Command::Prepare(_t) => {},
                Command::LocalPrepared(_t) => {
                    // TODO: Check if it's ok to unlock the inputs for ABORT at this point
                },
                Command::Accept(t) => {
                    let tx_rec = self
                        .transaction_pool
                        .get(tx, block.as_leaf_block(), cmd.transaction_id())?;
                    debug!(
                        target: LOG_TARGET,
                        "Transaction {} is finalized ({})", tx_rec.transaction_id(), t.decision
                    );

                    total_transaction_fee += tx_rec.transaction().transaction_fee;
                    total_fee_due += t.leader_fee;

                    let mut executed = t.get_transaction(tx.deref_mut())?;
                    // Commit the transaction substate changes.
                    if t.decision.is_commit() {
                        if let Some(reject_reason) = executed.result().finalize.reject() {
                            warn!(
                                target: LOG_TARGET,
                                "⚠️ We are unable to execute the block {} because transaction {} failed to execute but the committee decided to ACCEPT it.",
                                block,
                                tx_rec.transaction_id()
                            );
                            return Err(HotStuffError::RejectedTransactionCommitDecision {
                                block_id: *block.id(),
                                transaction_id: *tx_rec.transaction_id(),
                                reject_reason: reject_reason.to_string(),
                            });
                        }

                        self.state_manager
                            .commit_transaction(tx, block, &executed)
                            .map_err(|e| HotStuffError::StateManagerError(e.into()))?;
                    }

                    // Only unlock substates if we locked them in the first place
                    if tx_rec.current_decision().is_commit() {
                        // We unlock just so that inputs that were not mutated are unlocked, even though those
                        // should be in input_refs
                        self.unlock_inputs(tx, executed.transaction(), local_committee_shard)?;
                        // Unlock any outputs that were locked
                        self.unlock_outputs(tx, &executed, local_committee_shard)?;
                    }

                    // We are accepting the transaction so can remove the transaction from the pool
                    debug!(
                        target: LOG_TARGET,
                        "🗑️ Removing transaction {} from pool", tx_rec.transaction_id());
                    tx_rec.remove(tx)?;
                    executed.set_final_decision(t.decision).update(tx)?;
                },
            }
        }

        block.commit(tx)?;

        if total_transaction_fee > 0 {
            info!(
                target: LOG_TARGET,
                "🪙 Validator fee for block {} (amount due = {}, total fees = {})",
                block.proposed_by(),
                total_fee_due,
                total_transaction_fee
            );
        }

        Ok(())
    }

    fn validate_local_proposed_block_and_fill_dummy_blocks(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        from: &TConsensusSpec::Addr,
        candidate_block: &Block<TConsensusSpec::Addr>,
        local_committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let leader = self
            .leader_strategy
            .get_leader(local_committee, candidate_block.height());
        if leader != from {
            return Err(ProposalValidationError::NotLeader {
                proposed_by: from.to_string(),
                block_id: *candidate_block.id(),
            }
            .into());
        }
        self.validate_proposed_block(from, candidate_block)?;

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
                proposed_by: from.to_string(),
                block_id: *candidate_block.id(),
                justify_block: *candidate_block.justify().block_id(),
            }
            .into());
        };

        if justify_block.height() != candidate_block.justify().block_height() {
            return Err(ProposalValidationError::JustifyBlockInvalid {
                proposed_by: from.to_string(),
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
            return Ok(());
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

        // if the block parent is not the justify parent, then we have experienced a leader failure
        // and should make dummy blocks to fill in the gaps.
        if candidate_block.parent() != justify_block.id() {
            if candidate_block.height() < justify_block.height() {
                return Err(ProposalValidationError::CandidateBlockNotHigherThanJustifyBlock {
                    justify_block_height: justify_block.height(),
                    candidate_block_height: candidate_block.height(),
                }
                .into());
            }

            if justify_block.id() == candidate_block.parent() {
                return Ok(());
            }

            let justify_block_height = justify_block.height();

            let mut last_dummy_block = justify_block;

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
                last_dummy_block = Block::dummy_block(
                    *last_dummy_block.id(),
                    leader.clone(),
                    next_height,
                    candidate_block.justify().clone(),
                    candidate_block.epoch(),
                );
                debug!(target: LOG_TARGET, "🍼 DUMMY BLOCK: {}. Leader: {}", last_dummy_block, leader);
                last_dummy_block.save(tx)?;
                // We dont set this as the leaf block because we are not proposing next from these dummy blocks, if the
                // candidate block is valid it will become the leaf block.
            }
        }

        Ok(())
    }

    fn validate_proposed_block(
        &self,
        from: &TConsensusSpec::Addr,
        candidate_block: &Block<TConsensusSpec::Addr>,
    ) -> Result<(), ProposalValidationError> {
        if candidate_block.height() == NodeHeight::zero() || candidate_block.id().is_genesis() {
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
}
