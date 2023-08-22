//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// (New, true) ----(cmd:Prepare) ---> (Prepared, true) -----cmd:LocalPrepared ---> (LocalPrepared, false)
// ----[foreign:LocalPrepared]--->(LocalPrepared, true) ----cmd:AllPrepare ---> (AllPrepared, true) ---cmd:Accept --->
// Complete

use std::{num::NonZeroU64, ops::DerefMut};

use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Command,
        Decision,
        ExecutedTransaction,
        LastExecuted,
        LastVoted,
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
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::Transaction;
use tokio::sync::{broadcast, mpsc};

use crate::{
    hotstuff::{
        common::{update_high_qc, EXHAUST_DIVISOR},
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
        // First save the block in one db transaction
        self.store.with_write_tx(|tx| {
            // TODO: We should move the safe_block check to here
            self.validate_local_proposed_block_and_fill_dummy_blocks(&mut *tx, &from, &block, &local_committee)?;
            // Insert the block if it doesnt already exist
            block.justify().save(tx)?;
            block.save(tx)?;
            Ok::<_, HotStuffError>(())
        })?;

        if self.block_has_missing_transaction(&local_committee, &block).await? {
            Ok(())
        } else {
            self.process_block(&local_committee, &block).await
        }
    }

    async fn block_has_missing_transaction(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        block: &Block<TConsensusSpec::Addr>,
    ) -> Result<bool, HotStuffError> {
        let mut missing_tx_ids = Vec::new();
        let mut awaiting_execution = Vec::new();
        // TODO(perf): n queries
        self.store.with_read_tx(|tx| {
            for tx_id in block.all_transaction_ids() {
                match TransactionRecord::get(tx, tx_id).optional()? {
                    Some(tx) => {
                        // If execution is in progress, we need to note down the transactions without requesting them
                        if tx.result.is_none() {
                            awaiting_execution.push(*tx_id);
                        }
                    },
                    None => missing_tx_ids.push(*tx_id),
                }
            }
            Ok::<_, HotStuffError>(())
        })?;

        if missing_tx_ids.is_empty() && awaiting_execution.is_empty() {
            return Ok(false);
        }

        info!(
            target: LOG_TARGET,
            "🔥 Block {} has {} missing transactions and {} awaiting execution", block.id(), missing_tx_ids.len(), awaiting_execution.len(),
        );

        self.store.with_write_tx(|tx| {
            tx.insert_missing_transactions(block.id(), missing_tx_ids.iter().chain(&awaiting_execution))
        })?;

        if !missing_tx_ids.is_empty() {
            self.send_to_leader(
                local_committee,
                block.height(),
                HotstuffMessage::RequestMissingTransactions(RequestMissingTransactionsMessage {
                    block_id: *block.id(),
                    epoch: block.epoch(),
                    transactions: missing_tx_ids,
                }),
            )
            .await?;
        }

        Ok(true)
    }

    pub async fn reprocess_block(&self, block_id: &BlockId) -> Result<(), HotStuffError> {
        let block = self.store.with_read_tx(|tx| Block::get(tx, block_id))?;
        if !self.epoch_manager.is_epoch_active(block.epoch()).await? {
            return Err(HotStuffError::EpochNotActive {
                epoch: block.epoch(),
                details: "Cannot reprocess block from inactive epoch".to_string(),
            });
        }
        let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
        self.process_block(&local_committee, &block).await
    }

    async fn process_block(
        &self,
        local_committee: &Committee<<TConsensusSpec as ConsensusSpec>::Addr>,
        block: &Block<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(block.epoch()).await?;
        let maybe_decision = self.store.with_write_tx(|tx| {
            let should_vote = self.should_vote(&mut *tx, block)?;

            let mut maybe_decision = None;
            if should_vote {
                maybe_decision = self.decide_what_to_vote(tx, block, &local_committee_shard)?;
            }

            // Only update_node and set the last voted height if we vote
            if maybe_decision.is_some() {
                block.as_last_voted().set(tx)?;
                self.update_nodes(tx, block, &local_committee_shard)?;
            }
            Ok::<_, HotStuffError>(maybe_decision)
        })?;

        if let Some(decision) = maybe_decision {
            self.pacemaker.reset_leader_timeout(block.height()).await?;
            let vote = self.generate_vote_message(block, decision).await?;
            debug!(
                target: LOG_TARGET,
                "🔥 Send {:?} VOTE for block {}, parent {}, height {}",
                decision,
                block.id(),
                block.parent(),
                block.height(),
            );
            self.send_vote_to_leader(local_committee, vote, block.height()).await?;
        } else {
            // If the timer isnt started we need to start it.
            self.pacemaker.start_timer().await?;
        }

        Ok(())
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
        self.pacemaker.beat().await?;

        Ok(())
    }

    fn on_receive_foreign_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
        foreign_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        // Save the QCs if it doesnt exist already, we'll reference the QC in subsequent blocks
        block.justify().save(tx)?;

        // TODO(perf): n queries
        for cmd in block.commands() {
            let Some(t) = cmd.local_prepared() else {
                continue;
            };
            let Some(mut tx_rec) = self.transaction_pool.get(tx, &t.id).optional()? else {
                continue;
            };

            if tx_rec.stage().is_all_prepared() || tx_rec.stage().is_some_prepared() {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Foreign proposal received after transaction {} is {}. Ignoring.",
                    tx_rec.transaction.id, tx_rec.stage
                );
                continue;
            }

            tx_rec.update_evidence(tx, foreign_committee_shard, *block.justify().id())?;
            let change_to_abort = cmd.decision().is_abort() && tx_rec.original_decision().is_commit();
            if change_to_abort {
                info!(
                    target: LOG_TARGET,
                    "⚠️ Foreign shard ABORT {}. Update decision to ABORT",
                    tx_rec.transaction.id
                );
                tx_rec.update_remote_decision(tx, Decision::Abort)?;
            }

            // If all shards are complete and we've already received our LocalPrepared, we can set out LocalPrepared
            // transaction as ready to propose ACCEPT. If we have not received the local LocalPrepared, the transition
            // will happen when we receive the local block.
            if tx_rec.stage().is_local_prepared() && tx_rec.transaction.evidence.all_shards_complete() {
                tx_rec.transition(tx, TransactionPoolStage::LocalPrepared, true)?;
            }
        }

        Ok(())
    }

    async fn send_to_leader(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        height: NodeHeight,
        message: HotstuffMessage<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let leader = self.leader_strategy.get_leader(local_committee, height);
        self.tx_leader
            .send((leader.clone(), message))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_leader in OnReceiveProposalHandler::send_to_leader",
            })
    }

    async fn send_vote_to_leader(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        vote: VoteMessage<TConsensusSpec::Addr>,
        height: NodeHeight,
    ) -> Result<(), HotStuffError> {
        let leader = self.leader_strategy.get_leader_for_next_block(local_committee, height);
        self.tx_leader
            .send((leader.clone(), HotstuffMessage::Vote(vote)))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_leader in OnReceiveProposalHandler::send_vote_to_leader",
            })
    }

    #[allow(clippy::too_many_lines)]
    fn decide_what_to_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<Option<QuorumDecision>, HotStuffError> {
        let mut total_leader_fee = 0;
        for cmd in block.commands() {
            let Some(mut tx_rec) = self.transaction_pool.get(tx, cmd.transaction_id()).optional()? else {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Local proposal received for transaction {} which is not in the pool. Ignoring.",
                    cmd.transaction_id());
                return Ok(None);
            };
            // TODO: we probably need to provide the all/some of the QCs referenced in local transactions as
            //       part of the proposal DanMessage so that there is no race condition between receiving the
            //       proposed block and receiving the foreign proposals
            tx_rec.update_evidence(tx, local_committee_shard, *block.justify().id())?;

            debug!(
                target: LOG_TARGET,
                "🔥 vote for block {} {}. Cmd: {}",
                block.id(),
                block.height(),
                cmd,
            );
            match cmd {
                Command::Prepare(t) => {
                    if !tx_rec.stage().is_new() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for block {}. Leader proposed Prepare, local stage {}",
                            block.id(),
                            tx_rec.stage()
                        );
                        return Ok(None);
                    }

                    if tx_rec.transaction.transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block.id(),
                            t.transaction_fee,
                            tx_rec.transaction.transaction_fee
                        );
                        return Ok(None);
                    }

                    if tx_rec.current_decision() == t.decision {
                        if tx_rec.current_decision().is_commit() {
                            let transaction = ExecutedTransaction::get(tx.deref_mut(), cmd.transaction_id())?;
                            // Lock all inputs for the transaction as part of LocalPrepare
                            if !self.lock_inputs(tx, transaction.transaction(), local_committee_shard)? {
                                // Unable to lock all inputs - do not vote
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Unable to lock all inputs for transaction {} in block {}. Leader proposed {}, we decided {}",
                                    block.id(),
                                    transaction.id(),
                                    t.decision,
                                    Decision::Abort
                                );
                                // We change our decision to ABORT so that the next time we propose/receive a proposal
                                // we will check for ABORT. It may happen that the transaction causing the lock failure
                                // is ABORTED too and the locks released allowing this transaction to succeed.
                                // Currently, the client would have to resubmit the transaction to resolve this.
                                tx_rec.update_local_decision(tx, Decision::Abort)?;
                                // This brings up an interesting problem. If we decide to abstain from voting, then
                                // object conflicts essentially induce leader failures. This is problematic since it
                                // puts leader failure under the control of users and potentially malicious parties.
                                return Ok(None);
                            }
                            if !self.lock_outputs(tx, block.id(), &transaction)? {
                                // Unable to lock all outputs - do not vote
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Unable to lock all outputs for transaction {} in block {}. Leader proposed {}, we decided {}",
                                    block.id(),
                                    transaction.id(),
                                    t.decision,
                                    Decision::Abort
                                );
                                // We change our decision to ABORT so that the next time we propose/receive a proposal
                                // we will check for ABORT
                                tx_rec.update_local_decision(tx, Decision::Abort)?;
                                return Ok(None);
                            }
                        }

                        tx_rec.transition(tx, TransactionPoolStage::Prepared, true)?;
                    } else {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            tx_rec.current_decision()
                        );
                        return Ok(None);
                    }
                },
                Command::LocalPrepared(t) => {
                    // Happy path: We've validated all the QCs and therefore are convinced that everyone also Prepared.
                    // We only mark the next step (Accept) as ready to propose once all shards have reported
                    // LocalPrepared.

                    if !tx_rec.stage().is_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement in block {} for transaction {}. Leader proposed LocalPrepared, but we have not prepared",
                            block.id(),
                            tx_rec.transaction_id()
                        );
                        return Ok(None);
                    }
                    // We check that the leader decision is the same as our local decision.
                    // We disregard the remote decision because not all validators may have received the foreign
                    // LocalPrepared yet. We will never accept a decision disagreement for the Accept command.
                    if tx_rec.current_local_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ LocalPrepared decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            tx_rec.current_local_decision()
                        );
                        // We still vote to accept the block,
                        return Ok(None);
                    }

                    if tx_rec.transaction.transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block.id(),
                            t.transaction_fee,
                            tx_rec.transaction.transaction_fee
                        );
                        return Ok(None);
                    }

                    tx_rec.transition(
                        tx,
                        TransactionPoolStage::LocalPrepared,
                        tx_rec.transaction.evidence.all_shards_complete(),
                    )?;
                },
                Command::Accept(t) => {
                    // Happy path: We've validated all the QCs and therefore are convinced that everyone also received
                    // LocalPrepare. We then propose new blocks until we have a 3-chain
                    if !tx_rec.stage().is_local_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for block {}. Leader proposed Accept, local stage {}",
                            block.id(),
                            tx_rec.stage()
                        );
                        return Ok(None);
                    }
                    if tx_rec.current_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept decision disagreement for block {}. Leader proposed {}, we decided {}",
                            block.id(),
                            t.decision,
                            tx_rec.current_decision()
                        );
                        return Ok(None);
                    }

                    if !tx_rec.transaction.evidence.all_shards_complete() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept evidence disagreement for block {}. Evidence for {} out of {} shards",
                            block.id(),
                            tx_rec.transaction.evidence.num_complete_shards(),
                            tx_rec.transaction.evidence.len(),
                        );
                        return Ok(None);
                    }

                    if tx_rec.transaction.transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block.id(),
                            t.transaction_fee,
                            tx_rec.transaction.transaction_fee
                        );
                        return Ok(None);
                    }

                    let distinct_shards =
                        local_committee_shard.count_distinct_buckets(tx_rec.transaction.evidence.shards_iter());
                    let distinct_shards = NonZeroU64::new(distinct_shards as u64).ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Distinct shards is zero for transaction {} in block {}",
                            tx_rec.transaction.id,
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
                        return Ok(None);
                    }
                    total_leader_fee += calculated_leader_fee;
                    // If the decision was changed to Abort, which can only happen when a foreign shard decides ABORT
                    // and we decide COMMIT, we set SomePrepared, otherwise AllPrepared. There are no further stages
                    // after these, so these MUST never be ready to propose.
                    if tx_rec.remote_decision().map(|d| d.is_abort()).unwrap_or(false) {
                        tx_rec.transition(tx, TransactionPoolStage::SomePrepared, false)?;
                    } else {
                        tx_rec.transition(tx, TransactionPoolStage::AllPrepared, false)?;
                    }
                },
            }
        }

        if total_leader_fee != block.total_leader_fee() {
            warn!(
                target: LOG_TARGET,
                "❌ Leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                block.id(),
                block.total_leader_fee(),
                total_leader_fee
            );
            return Ok(None);
        }

        info!(target: LOG_TARGET, "✅ Voting to accept block {} {}", block.id(), block.height());
        Ok(Some(QuorumDecision::Accept))
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
        let state = LockedOutput::try_acquire_all(tx, block_id, transaction.id(), transaction.resulting_outputs())?;

        if !state.is_acquired() {
            return Ok(false);
        }

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

    fn update_nodes(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        update_high_qc(tx, block.justify())?;

        // b'' <- b*.justify.node
        let Some(commit_node) = block.justify().get_block(tx.deref_mut()).optional()? else {
            return Ok(());
        };

        // b' <- b''.justify.node
        let Some(precommit_node) = commit_node.justify().get_block(tx.deref_mut()).optional()? else {
            return Ok(());
        };

        let locked_block = LockedBlock::get(tx.deref_mut())?;
        if precommit_node.height() > locked_block.height {
            debug!(target: LOG_TARGET, "LOCKED NODE SET: {} {}", precommit_node.height(), precommit_node.id());
            // precommit_node is at COMMIT phase
            precommit_node.as_locked().set(tx)?;
        }

        // b <- b'.justify.node
        let prepare_node = precommit_node.justify().block_id();
        if commit_node.parent() == precommit_node.id() && precommit_node.parent() == prepare_node {
            debug!(
                target: LOG_TARGET,
                "✅ Node {} {} forms a 3-chain b'' = {}, b' = {}, b = {}",
                block.height(),
                block.id(),
                commit_node.id(),
                precommit_node.id(),
                prepare_node,
            );

            // Commit prepare_node (b)
            let prepare_node = Block::get(tx.deref_mut(), prepare_node)?;
            let last_executed = LastExecuted::get(tx.deref_mut())?;
            self.on_commit(tx, &last_executed, &prepare_node, local_committee_shard)?;
            prepare_node.as_last_executed().set(tx)?;
        } else {
            debug!(
                target: LOG_TARGET,
                "Node {} {} DOES NOT form a 3-chain b'' = {}, b' = {}, b = {}, b* = {}",
                block.height(),
                block.id(),
                commit_node.id(),
                precommit_node.id(),
                prepare_node,
                block.id()
            );
        }

        Ok(())
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
            debug!(
                target: LOG_TARGET,
                "✅ ACCEPT Node {} {}, last executed height = {}",
                block.height(),
                block.id(),
                last_executed.height
            );
            self.execute(tx, block, local_committee_shard)?;

            self.publish_event(HotstuffEvent::BlockCommitted { block_id: *block.id() });
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
            let tx_rec = self.transaction_pool.get(tx, cmd.transaction_id())?;
            match cmd {
                Command::Prepare(_t) => {},
                Command::LocalPrepared(_t) => {
                    // TODO: Check if it's ok to unlock the inputs for ABORT at this point
                },
                Command::Accept(t) => {
                    debug!(
                        target: LOG_TARGET,
                        "Transaction {} is finalized ({})", tx_rec.transaction.id, t.decision
                    );

                    if t.decision != tx_rec.current_decision() {
                        return Err(HotStuffError::InvariantError(format!(
                            "Transaction {} decision mismatch on COMMIT block {}. Block decision {}, local decision: \
                             {}",
                            tx_rec.transaction.id,
                            block.id(),
                            t.decision,
                            tx_rec.current_decision(),
                        )));
                    }

                    total_transaction_fee += tx_rec.transaction.transaction_fee;
                    total_fee_due += t.leader_fee;

                    let mut executed = t.get_transaction(tx.deref_mut())?;
                    // Commit the transaction substate changes.
                    if tx_rec.current_decision().is_commit() {
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
                    tx_rec.remove(tx)?;
                    executed.set_final_decision(t.decision).update(tx)?;
                },
            }
        }

        block.commit(tx)?;

        info!(
            target: LOG_TARGET,
            "🪙 Validator fee for block {} (amount due = {}, total fees = {})",
            block.proposed_by(),
            total_fee_due,
            total_transaction_fee
        );

        Ok(())
    }

    fn validate_local_proposed_block_and_fill_dummy_blocks(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        from: &TConsensusSpec::Addr,
        candidate_block: &Block<TConsensusSpec::Addr>,
        local_committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<(), ProposalValidationError> {
        let leader = self
            .leader_strategy
            .get_leader(local_committee, candidate_block.height());
        if leader != from {
            return Err(ProposalValidationError::NotLeader {
                proposed_by: from.to_string(),
                block_id: *candidate_block.id(),
            });
        }
        self.validate_proposed_block(from, candidate_block)?;

        // Check that details included in the justify match previously added blocks
        let Some(justify_block) = candidate_block.justify().get_block(tx.deref_mut()).optional()? else {
            // TODO: This may mean that we have to catch up
            return Err(ProposalValidationError::JustifyBlockNotFound {
                proposed_by: from.to_string(),
                hash: *candidate_block.id(),
                justify_block: *candidate_block.justify().block_id(),
            });
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
            });
        }

        // Special case for genesis block
        if candidate_block.parent().is_genesis() && candidate_block.justify().is_genesis() {
            return Ok(());
        }

        // if candidate_block.height().saturating_sub(justify_block.height()).0 > local_committee.max_failures() as u64
        // { TODO: We should maybe relax this constraint during GST, before the first block, many leaders might
        // fail....
        // Note: we are adding at least one more block from b_leaf, so we need to add 1 to the max_failures
        if candidate_block.height().saturating_sub(justify_block.height()).0 > local_committee.len() as u64 + 1 {
            return Err(ProposalValidationError::CandidateBlockHigherThanMaxFailures {
                proposed_by: from.to_string(),
                justify_block_height: justify_block.height(),
                candidate_block_height: candidate_block.height(),
                max_failures: local_committee.max_failures(),
            });
        }

        // if the block parent is not the justify parent, then we have experienced a leader failure
        // and should make dummy blocks to fill in the gaps.
        if candidate_block.parent() != justify_block.parent() {
            if candidate_block.height() < justify_block.height() {
                return Err(ProposalValidationError::CandidateBlockNotHigherThanJustifyBlock {
                    justify_block_height: justify_block.height(),
                    candidate_block_height: candidate_block.height(),
                });
            }

            let justify_block_height = justify_block.height();
            let mut last_dummy_block = justify_block;

            let mut leader = self
                .leader_strategy
                .get_leader_for_next_block(local_committee, last_dummy_block.height());
            while last_dummy_block.id() != candidate_block.parent() {
                if last_dummy_block.height() > candidate_block.height() {
                    warn!(target: LOG_TARGET, "🔥 Bad proposal, leaf block height {} is greater than new height {}", last_dummy_block.height(), candidate_block.height());
                    return Err(ProposalValidationError::CandidateBlockDoesNotExtendJustify {
                        justify_block_height,
                        candidate_block_height: candidate_block.height(),
                    });
                }

                info!(target: LOG_TARGET, "Creating dummy block for leader {}, height: {}", leader, last_dummy_block.height() + NodeHeight(1));
                // TODO: replace with actual leader's propose
                last_dummy_block = Block::dummy_block(
                    *last_dummy_block.id(),
                    leader.clone(),
                    last_dummy_block.height() + NodeHeight(1),
                    candidate_block.epoch(),
                );
                last_dummy_block.save(tx)?;
                // last_dummy_block.as_leaf_block().set(tx)?;
                leader = self
                    .leader_strategy
                    .get_leader_for_next_block(local_committee, last_dummy_block.height());
            }
        }

        // TODO: remove other call to should_vote
        if !is_safe_block(tx.deref_mut(), candidate_block)? {
            return Err(ProposalValidationError::NotSafeBlock {
                proposed_by: from.to_string(),
                hash: *candidate_block.id(),
            });
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

        // (b_new extends b_lock && b_new .justify.node.height > b_lock .height)
        if !is_safe_block(tx, block)? {
            return Ok(false);
        }

        Ok(true)
    }
}

/// safeNode predicate (https://arxiv.org/pdf/1803.05069v6.pdf)
///
/// The safeNode predicate is a core ingredient of the protocol. It examines a proposal message
/// m carrying a QC justication m.justify, and determines whether m.node is safe to accept. The safety rule to accept
/// a proposal is the branch of m.node extends from the currently locked node lockedQC.node. On the other hand, the
/// liveness rule is the replica will accept m if m.justify has a higher view than the current lockedQC. The predicate
/// is true as long as either one of two rules holds.
fn is_safe_block<TTx: StateStoreReadTransaction>(
    tx: &mut TTx,
    block: &Block<TTx::Addr>,
) -> Result<bool, ProposalValidationError> {
    let locked = LockedBlock::get(tx)?;
    let locked_block = locked.get_block(tx)?;

    // Liveness
    if block.justify().block_height() <= locked_block.height() {
        info!(
            target: LOG_TARGET,
            "❌ justify block height {} less than or equal to locked block height {}. Block does not satisfy safeNode predicate",
            block.justify().block_height(),
            locked_block.height(),
        );
        return Ok(false);
    }

    // Safety
    let extends = block.extends(tx, locked_block.id())?;
    if !extends {
        info!(
            target: LOG_TARGET,
            "❌ Block {} does not extend locked block {}. Block does not satisfy safeNode predicate",
            block.id(),
            locked_block.id(),
        );
    }
    Ok(extends)
}
