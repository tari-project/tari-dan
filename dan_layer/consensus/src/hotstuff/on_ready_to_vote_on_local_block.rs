//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, num::NonZeroU64, ops::DerefMut};

use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    ShardId,
};
use tari_dan_storage::{
    consensus_models::{
        Block, BlockId, Command, Decision, ExecutedTransaction, HighQc, LastExecuted, LastSentVote, LastVoted,
        LockedBlock, LockedOutput, QuorumDecision, SubstateLockFlag, SubstateRecord, TransactionPool,
        TransactionPoolStage, ValidBlock,
    },
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::Transaction;
use tokio::sync::{
    broadcast,
    mpsc::{self},
};

use super::proposer::Proposer;
use crate::{
    hotstuff::{common::EXHAUST_DIVISOR, error::HotStuffError, event::HotstuffEvent, ProposalValidationError},
    messages::{HotstuffMessage, VoteMessage},
    traits::{ConsensusSpec, LeaderStrategy, StateManager, VoteSignatureService},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_lock_block_ready";

pub struct OnReadyToVoteOnLocalBlock<TConsensusSpec: ConsensusSpec> {
    validator_addr: TConsensusSpec::Addr,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signing_service: TConsensusSpec::VoteSignatureService,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    state_manager: TConsensusSpec::StateManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    tx_events: broadcast::Sender<HotstuffEvent>,
    proposer: Proposer<TConsensusSpec>,
}

impl<TConsensusSpec> OnReadyToVoteOnLocalBlock<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
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
        proposer: Proposer<TConsensusSpec>,
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
            proposer,
        }
    }

    pub async fn handle(&self, valid_block: ValidBlock<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 LOCAL PROPOSAL READY: {}",
            valid_block,
        );

        let local_committee_shard = self
            .epoch_manager
            .get_committee_shard_by_validator_address(valid_block.epoch(), valid_block.proposed_by())
            .await?;
        let mut locked_blocks = Vec::new();

        let maybe_decision = self.store.with_write_tx(|tx| {
            let maybe_decision = self.decide_on_block(tx, &local_committee_shard, valid_block.block())?;

            // Update nodes
            if maybe_decision.map(|d| d.is_accept()).unwrap_or(false) {
                let high_qc = valid_block.block().update_nodes(
                    tx,
                    |tx, locked, block, locked_blocks| self.on_lock_block(tx, locked, block, locked_blocks),
                    |tx, last_exec, commit_block| self.on_commit(tx, last_exec, commit_block, &local_committee_shard),
                    &mut locked_blocks,
                )?;

                // If we have a new high QC, we'll process the block it justifies
                self.process_new_leaf(tx, high_qc, valid_block.block(), &local_committee_shard)?;
            }

            if maybe_decision.is_some() {
                valid_block.block().as_last_voted().set(tx)?;
            }
            Ok::<_, HotStuffError>(maybe_decision)
        })?;
        self.propose_newly_locked_blocks(locked_blocks).await?;

        if let Some(decision) = maybe_decision {
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

        Ok(())
    }

    fn decide_on_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        local_committee_shard: &CommitteeShard,
        block: &Block<TConsensusSpec::Addr>,
    ) -> Result<Option<QuorumDecision>, HotStuffError> {
        let mut maybe_decision = None;
        if self.should_vote(tx.deref_mut(), block)? {
            maybe_decision = self.decide_what_to_vote(tx, block, local_committee_shard)?;
        }

        Ok(maybe_decision)
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
            debug!(
                target: LOG_TARGET,
                "🔥 Process NEW leaf block: Block {} already processed",
                leaf,
            );
            return Ok(());
        }

        debug!(
            target: LOG_TARGET,
            "🔥 Process NEW leaf block: Block {}",
            leaf,
        );

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

    async fn send_vote_to_leader(
        &self,
        local_committee: &Committee<TConsensusSpec::Addr>,
        vote: VoteMessage<TConsensusSpec::Addr>,
        block: &Block<TConsensusSpec::Addr>,
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
        if self
            .tx_leader
            .send((leader.clone(), HotstuffMessage::Vote(vote.clone())))
            .await
            .is_err()
        {
            debug!(
                target: LOG_TARGET,
                "tx_leader in OnLocalProposalReady::send_vote_to_leader is closed",
            );
        }
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

    #[allow(clippy::too_many_lines)]
    fn decide_what_to_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
        local_committee_shard: &CommitteeShard,
    ) -> Result<Option<QuorumDecision>, HotStuffError> {
        let mut total_leader_fee = 0;
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
                return Ok(None);
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
                        return Ok(None);
                    }

                    if tx_rec.transaction().transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block.id(),
                            t.transaction_fee,
                            tx_rec.transaction().transaction_fee
                        );
                        return Ok(None);
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
                                return Ok(None);
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
                                return Ok(None);
                            } else {
                                // We have locked all inputs and outputs
                            }
                        }

                        tx_rec.add_pending_status_update(
                            tx,
                            block.as_leaf_block(),
                            TransactionPoolStage::Prepared,
                            true,
                        )?;
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
                        return Ok(None);
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
                        return Ok(None);
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
                        return Ok(None);
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
                        return Ok(None);
                    }

                    tx_rec.add_pending_status_update(
                        tx,
                        block.as_leaf_block(),
                        TransactionPoolStage::LocalPrepared,
                        tx_rec.transaction().evidence.all_shards_complete(),
                    )?;
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
                        return Ok(None);
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
                        return Ok(None);
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
                        return Ok(None);
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

                        return Ok(None);
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
                        return Ok(None);
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

                        return Ok(None);
                    }
                    total_leader_fee += calculated_leader_fee;
                    // If the decision was changed to Abort, which can only happen when a foreign shard decides ABORT
                    // and we decide COMMIT, we set SomePrepared, otherwise AllPrepared. There are no further stages
                    // after these, so these MUST never be ready to propose.
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
        let vn = self
            .epoch_manager
            .get_validator_node(block.epoch(), &self.validator_addr)
            .await?;
        let leaf_hash = vn.node_hash();

        let signature = self.vote_signing_service.sign_vote(&leaf_hash, block.id(), &decision);

        Ok(VoteMessage {
            epoch: block.epoch(),
            block_id: *block.id(),
            block_height: block.height(),
            decision,
            signature,
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

    // Returns the number processed blocks
    fn on_lock_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        locked: &LockedBlock,
        block: &Block<TConsensusSpec::Addr>,
        locked_blocks: &mut Vec<Block<TConsensusSpec::Addr>>,
    ) -> Result<(), HotStuffError> {
        if locked.height < block.height() {
            info!(
                target: LOG_TARGET,
                "🔒️ LOCKED BLOCK: {} {}",
                block.height(),
                block.id()
            );

            let parent = block.get_parent(tx.deref_mut())?;
            locked_blocks.push(block.clone());
            self.on_lock_block(tx, locked, &parent, locked_blocks)?;

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

    async fn propose_newly_locked_blocks(&self, blocks: Vec<Block<TConsensusSpec::Addr>>) -> Result<(), HotStuffError> {
        for block in blocks {
            let local_committee = self.epoch_manager.get_local_committee(block.epoch()).await?;
            let is_leader = self
                .leader_strategy
                .is_leader(&self.validator_addr, &local_committee, block.height());
            // TODO: This will be changed to different strategy where not only leader is responsible for foreign block proposal.
            if is_leader {
                self.proposer.handle_on_lock_block(&block).await?;
            }
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
}
