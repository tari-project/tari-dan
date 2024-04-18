//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, num::NonZeroU64, ops::DerefMut};

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    SubstateAddress,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Command,
        Decision,
        EpochEvent,
        ExecutedTransaction,
        ForeignProposal,
        HighQc,
        LastExecuted,
        LastSentVote,
        LastVoted,
        LockedBlock,
        LockedOutput,
        PendingStateTreeDiff,
        QuorumDecision,
        SubstateLockFlag,
        SubstateRecord,
        TransactionAtom,
        TransactionPool,
        TransactionPoolStage,
        ValidBlock,
    },
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::{Transaction, TransactionId, VersionedSubstateId};
use tokio::sync::broadcast;

use super::proposer::Proposer;
use crate::{
    hotstuff::{error::HotStuffError, event::HotstuffEvent, ProposalValidationError, EXHAUST_DIVISOR},
    messages::{HotstuffMessage, VoteMessage},
    traits::{
        hooks::ConsensusHooks,
        BlockTransactionExecutor,
        BlockTransactionExecutorBuilder,
        ConsensusSpec,
        LeaderStrategy,
        OutboundMessaging,
        StateManager,
        VoteSignatureService,
    },
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_lock_block_ready";

pub struct OnReadyToVoteOnLocalBlock<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signing_service: TConsensusSpec::SignatureService,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    state_manager: TConsensusSpec::StateManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    tx_events: broadcast::Sender<HotstuffEvent>,
    proposer: Proposer<TConsensusSpec>,
    transaction_executor_builder: TConsensusSpec::BlockTransactionExecutorBuilder,
    network: Network,
    hooks: TConsensusSpec::Hooks,
}

impl<TConsensusSpec> OnReadyToVoteOnLocalBlock<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        validator_addr: TConsensusSpec::Addr,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signing_service: TConsensusSpec::SignatureService,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        state_manager: TConsensusSpec::StateManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        tx_events: broadcast::Sender<HotstuffEvent>,
        proposer: Proposer<TConsensusSpec>,
        transaction_executor_builder: TConsensusSpec::BlockTransactionExecutorBuilder,
        network: Network,
        hooks: TConsensusSpec::Hooks,
    ) -> Self {
        Self {
            local_validator_addr: validator_addr,
            store,
            epoch_manager,
            vote_signing_service,
            leader_strategy,
            state_manager,
            transaction_pool,
            outbound_messaging,
            tx_events,
            proposer,
            transaction_executor_builder,
            network,
            hooks,
        }
    }

    pub async fn handle(&mut self, valid_block: ValidBlock) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 LOCAL PROPOSAL READY: {}",
            valid_block,
        );

        let local_committee_shard = self
            .epoch_manager
            .get_committee_shard_by_validator_public_key(valid_block.epoch(), valid_block.proposed_by())
            .await?;
        let mut locked_blocks = Vec::new();
        let mut finalized_transactions = Vec::new();
        let qc_block = self
            .store
            .with_read_tx(|tx| Block::get(tx, valid_block.block().justify().block_id()))?;
        let locked_block = self.store.with_read_tx(|tx| {
            let locked_block = LockedBlock::get(tx)?;
            Block::get(tx, locked_block.block_id())
        })?;
        // If the previous qc block was in different epoch, we have to have EpochEvent::Start
        let epoch_start = qc_block.epoch() < valid_block.epoch();

        let epoch_end = qc_block.epoch() == valid_block.epoch() &&// If we didn't locked block with an EpochEvent::End
            (qc_block.is_epoch_end() && !locked_block.is_epoch_end()) && // The last block is from previous epoch or it is an EpochEnd block
            !qc_block.is_genesis(); // If the previous epoch is the genesis epoch, we don't need to end it (there was no committee at epoch 0)
        let maybe_decision = self.store.with_write_tx(|tx| {
            let mut maybe_decision =
                self.decide_on_block(tx, &local_committee_shard, valid_block.block(), epoch_start, epoch_end)?;

            let is_accept_decision = maybe_decision.map(|d| d.is_accept()).unwrap_or(false);
            // Update nodes
            if is_accept_decision {
                let high_qc = valid_block.block().update_nodes(
                    tx,
                    |tx, locked, block| {
                        locked_blocks.push(block.clone());
                        self.on_lock_block(tx, locked, block)
                    },
                    |tx, last_exec, commit_block| {
                        let committed = self.on_commit(tx, last_exec, commit_block, &local_committee_shard)?;
                        finalized_transactions.push(committed);
                        Ok(())
                    },
                )?;

                // If we have a new high QC, we'll process the block it justifies
                if !self.process_new_leaf(tx, high_qc, valid_block.block(), &local_committee_shard)? {
                    maybe_decision = None;
                }
            }

            if maybe_decision.is_some() {
                valid_block.block().as_last_voted().set(tx)?;
            }

            Ok::<_, HotStuffError>(maybe_decision)
        })?;

        self.hooks.on_local_block_decide(&valid_block, maybe_decision);
        for t in finalized_transactions.into_iter().flatten() {
            self.hooks.on_transaction_finalized(&t);
        }
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
        block: &Block,
        epoch_start: bool,
        epoch_end: bool,
    ) -> Result<Option<QuorumDecision>, HotStuffError> {
        let mut maybe_decision = None;
        if self.should_vote(tx.deref_mut(), block)? {
            maybe_decision = self.decide_what_to_vote(tx, block, local_committee_shard, epoch_start, epoch_end)?;
        }

        Ok(maybe_decision)
    }

    #[allow(clippy::too_many_lines)]
    fn process_new_leaf(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        high_qc: HighQc,
        tip_block: &Block,
        local_committee_shard: &CommitteeShard,
    ) -> Result<bool, HotStuffError> {
        let mut leaf = high_qc.get_block(tx.deref_mut())?;
        if leaf.is_processed() {
            debug!(
                target: LOG_TARGET,
                "🔥 Process NEW leaf block: Block {} already processed",
                leaf,
            );
            return Ok(true);
        }

        debug!(
            target: LOG_TARGET,
            "🔥 Process NEW leaf block: Block {}",
            leaf,
        );

        for cmd in leaf.commands() {
            match cmd {
                Command::LocalOnly(t) => {
                    if t.decision.is_commit() {
                        let transaction = ExecutedTransaction::get(tx.deref_mut(), &t.id)?;
                        // Lock all inputs for the transaction as part of Prepare
                        let is_inputs_locked =
                            self.lock_inputs(tx, transaction.transaction(), local_committee_shard)?;
                        let is_outputs_locked = is_inputs_locked && self.lock_outputs(tx, leaf.id(), &transaction)?;

                        // This should not be possible and may be due to a BUG. The failure to lock the leaf block
                        // should have been detected in decide_on_block in the previous round.
                        if !is_inputs_locked {
                            // Unable to lock all inputs - do not vote on the child block
                            warn!(
                                target: LOG_TARGET,
                                "❌ [NEVERHAPPEN] Unable to lock all inputs for transaction {} in block {}. Leader proposed {}, we decided {}",
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
                            let mut tx_rec = self.transaction_pool.get(tx, tip_block.as_leaf_block(), t.id())?;
                            tx_rec.update_local_decision(tx, Decision::Abort)?;

                            // The leader should not have proposed conflicting transactions
                            return Ok(false);
                        } else if !is_outputs_locked {
                            // Unable to lock all outputs - do not vote
                            warn!(
                                target: LOG_TARGET,
                                "❌ [NEVERHAPPEN] Unable to lock all outputs for transaction {} in block {}. Leader proposed {}, we decided {}",
                                leaf.id(),
                                transaction.id(),
                                t.decision,
                                Decision::Abort
                            );
                            // Unlock any locked inputs because we are not voting
                            self.unlock_inputs(tx, transaction.transaction(), local_committee_shard)?;
                            // We change our decision to ABORT so that the next time we propose/receive a
                            // proposal we will check for ABORT
                            let mut tx_rec = self.transaction_pool.get(tx, tip_block.as_leaf_block(), t.id())?;
                            tx_rec.update_local_decision(tx, Decision::Abort)?;
                            return Ok(false);
                        } else {
                            // We have locked all inputs and outputs
                        }
                    }
                },
                Command::Prepare(t) => {
                    if t.decision.is_commit() {
                        let transaction = ExecutedTransaction::get(tx.deref_mut(), &t.id)?;
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
                            let mut tx_rec = self.transaction_pool.get(tx, tip_block.as_leaf_block(), t.id())?;
                            tx_rec.update_local_decision(tx, Decision::Abort)?;

                            // The leader should not have proposed conflicting transactions
                            return Ok(false);
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
                            let mut tx_rec = self.transaction_pool.get(tx, tip_block.as_leaf_block(), t.id())?;
                            tx_rec.update_local_decision(tx, Decision::Abort)?;
                            return Ok(false);
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
                Command::ForeignProposal(_) => {},
                Command::EpochEvent(_) => {},
            }
        }

        leaf.set_as_processed(tx)?;

        Ok(true)
    }

    /// if b_new .height > vheight && (b_new extends b_lock || b_new .justify.node.height > b_lock .height)
    ///
    /// If we have not previously voted on this block and the node extends the current locked node, then we vote
    fn should_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
    ) -> Result<bool, ProposalValidationError> {
        let Some(last_voted) = LastVoted::get(tx).optional()? else {
            // Never voted, then validated.block.height() > last_voted.height (0)
            return Ok(true);
        };

        // if b_new .height > vheight And ...
        if block.height() <= last_voted.height {
            info!(
                target: LOG_TARGET,
                "❌ NOT voting on block {}. Block height is not greater than last voted height {}",
                block,
                last_voted.height,
            );
            return Ok(false);
        }

        Ok(true)
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

    #[allow(clippy::too_many_lines)]
    fn decide_what_to_vote(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        local_committee_shard: &CommitteeShard,
        epoch_start: bool,
        epoch_end: bool,
    ) -> Result<Option<QuorumDecision>, HotStuffError> {
        let mut total_leader_fee = 0;
        let mut locked_inputs = HashSet::new();
        let mut locked_outputs = HashSet::new();

        // Executor used for transactions that have inputs without specific versions.
        // It lives through the entire block so multiple transactions can be "chained" together in the same block
        let mut executor = self.transaction_executor_builder.build();

        if epoch_start && !block.is_epoch_start() {
            warn!(
                target: LOG_TARGET,
                "❌ EpochEvent::Start command expected for block {} but not found",
                block.id()
            );
            return Ok(None);
        }

        if epoch_end && !block.is_epoch_end() {
            warn!(
                target: LOG_TARGET,
                "❌ EpochEvent::End command expected for block {} but not found",
                block.id()
            );
            return Ok(None);
        }

        for cmd in block.commands() {
            if let Some(foreign_proposal) = cmd.foreign_proposal() {
                if !ForeignProposal::exists(tx.deref_mut(), foreign_proposal)? {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Foreign proposal for block {block_id} from bucket {bucket} does not exist in the store",
                        block_id = foreign_proposal.block_id,bucket = foreign_proposal.bucket
                    );
                    return Ok(None);
                }
                continue;
            }
            if let Command::EpochEvent(event) = cmd {
                match event {
                    EpochEvent::Start => {
                        if !epoch_start {
                            warn!(
                                target: LOG_TARGET,
                                "❌ EpochEvent::Start command received for block {} but it is not the start of the epoch",
                                block.id()
                            );
                            return Ok(None);
                        }
                    },
                    EpochEvent::End => {},
                }
                continue;
            }

            let transaction = cmd
                .transaction()
                .expect("foreign proposal already checked, all remaining commands must have a transaction atom");

            let Some(mut tx_rec) = self
                .transaction_pool
                .get(tx, block.as_leaf_block(), transaction.id())
                .optional()?
            else {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                    block,
                    cmd.id(),
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
                Command::LocalOnly(t) => {
                    if !tx_rec.current_stage().is_new() && !tx_rec.current_stage().is_local_only() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for tx {} in block {}. Leader proposed LocalOnly, local stage is {}",
                            tx_rec.transaction_id(),
                            block,
                            tx_rec.current_stage(),
                        );
                        return Ok(None);
                    }

                    if tx_rec.transaction().transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ LocalOnly transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block,
                            t.transaction_fee,
                            tx_rec.transaction().transaction_fee
                        );
                        return Ok(None);
                    }

                    if tx_rec.current_decision() == t.decision {
                        if tx_rec.current_decision().is_commit() {
                            let executed = self.get_executed_transaction(tx, &t.id, &mut executor)?;
                            if !local_committee_shard.includes_all_substate_addresses(executed.involved_shards_iter()) {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ LocalOnly transaction {} in block {} has more than one involved shard",
                                    t.id,
                                    block,
                                );
                                return Ok(None);
                            }

                            let transaction = executed.transaction();

                            // Lock all inputs for the transaction as part of LocalOnly
                            let is_inputs_locked =
                                self.check_lock_inputs(tx, transaction, local_committee_shard, &mut locked_inputs)?;
                            let is_outputs_locked = is_inputs_locked &&
                                self.check_lock_outputs(tx, &executed, &mut locked_outputs, &locked_inputs)?;

                            if !is_inputs_locked {
                                // Unable to lock all inputs - do not vote
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Unable to lock all inputs for transaction {} in block {}.",
                                    block,
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
                                    block,
                                    transaction.id(),
                                );
                                // We change our decision to ABORT so that the next time we propose/receive a
                                // proposal we will check for ABORT
                                tx_rec.update_local_decision(tx, Decision::Abort)?;
                                return Ok(None);
                            } else {
                                // We have locked all inputs and outputs

                                // We need to update the database (transaction result and inputs/outputs)
                                // in case the transaction was re-executed because it has inputs without versions
                                let has_involved_shards = executed.num_involved_shards() > 0;
                                if transaction.has_inputs_without_version() && has_involved_shards {
                                    executed.update(tx)?;
                                }
                            }

                            if t.leader_fee.is_none() {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Leader fee for tx {} is None for LocalOnly command in block {}",
                                    t.id,
                                    block,
                                );
                                return Ok(None);
                            }

                            let calculated_leader_fee =
                                tx_rec.calculate_leader_fee(NonZeroU64::new(1).unwrap(), EXHAUST_DIVISOR);
                            if calculated_leader_fee != *t.leader_fee.as_ref().expect("None already checked") {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ LocalOnly leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                                    block,
                                    t.leader_fee.as_ref().expect("None already checked"),
                                    calculated_leader_fee
                                );

                                return Ok(None);
                            }
                            total_leader_fee += calculated_leader_fee.fee();
                        }

                        tx_rec.add_pending_status_update(
                            tx,
                            block.as_leaf_block(),
                            TransactionPoolStage::LocalOnly,
                            false,
                        )?;
                    } else {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for tx {} in block {}. Leader proposed {}, we decided {}",
                            tx_rec.transaction_id(),
                            block,
                            t.decision,
                            tx_rec.current_decision()
                        );
                        return Ok(None);
                    }
                },
                Command::Prepare(t) => {
                    if !tx_rec.current_stage().is_new() && !tx_rec.current_stage().is_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for tx {} in block {}. Leader proposed Prepare, local stage is {}",
                            tx_rec.transaction_id(),
                            block,
                            tx_rec.current_stage(),
                        );
                        return Ok(None);
                    }

                    if tx_rec.transaction().transaction_fee != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block,
                            t.transaction_fee,
                            tx_rec.transaction().transaction_fee
                        );
                        return Ok(None);
                    }

                    if tx_rec.current_decision() == t.decision {
                        if tx_rec.current_decision().is_commit() {
                            let executed = self.get_executed_transaction(tx, &t.id, &mut executor)?;
                            let transaction = executed.transaction();

                            // Lock all inputs for the transaction as part of Prepare
                            let is_inputs_locked =
                                self.check_lock_inputs(tx, transaction, local_committee_shard, &mut locked_inputs)?;
                            let is_outputs_locked = is_inputs_locked &&
                                self.check_lock_outputs(tx, &executed, &mut locked_outputs, &locked_inputs)?;

                            if !is_inputs_locked {
                                // Unable to lock all inputs - do not vote
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Unable to lock all inputs for transaction {} in block {}.",
                                    block,
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
                                    block,
                                    transaction.id(),
                                );
                                // We change our decision to ABORT so that the next time we propose/receive a
                                // proposal we will check for ABORT
                                tx_rec.update_local_decision(tx, Decision::Abort)?;
                                return Ok(None);
                            } else {
                                // We have locked all inputs and outputs

                                // We need to update the database (transaction result and inputs/outpus)
                                // in case the transaction was re-executed because it has inputs without versions
                                let has_involved_shards = executed.num_involved_shards() > 0;
                                if transaction.has_inputs_without_version() && has_involved_shards {
                                    executed.update(tx)?;
                                }
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
                            block,
                            t.decision,
                            tx_rec.current_decision()
                        );
                        return Ok(None);
                    }
                },
                Command::LocalPrepared(t) => {
                    // Happy path: We've validated all the QCs and therefore are convinced that everyone also
                    // Prepared. We only mark the next step (Accept) as ready to propose
                    // once all shards have reported LocalPrepared.

                    if !tx_rec.current_stage().is_prepared() && !tx_rec.current_stage().is_local_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "{} ❌ Stage disagreement in block {} for transaction {}. Leader proposed LocalPrepared, but local stage is {}",
                            self.local_validator_addr,
                            block,
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
                            block,
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
                            block,
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
                    // Happy path: We've validated all the QCs and therefore are convinced that everyone also
                    // received LocalPrepare. We then propose new blocks until we have a
                    // 3-chain
                    if !tx_rec.current_stage().is_local_prepared() && !tx_rec.current_stage().is_accepted() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for tx {} in block {}. Leader proposed Accept, local stage {}",
                            tx_rec.transaction_id(),
                            block,
                            tx_rec.current_stage(),
                        );
                        return Ok(None);
                    }
                    if tx_rec.current_decision() != t.decision {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept decision disagreement tx {} in for block {}. Leader proposed {}, we decided {}",
                            tx_rec.transaction_id(),
                            block,
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
                            block,
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
                            block,
                            t.transaction_fee,
                            tx_rec.transaction().transaction_fee
                        );

                        return Ok(None);
                    }

                    // Check if we have LocalPrepared ready i.e. LocalPrepared from all shards
                    // It is possible that the transaction was not marked as ready yet because of the order we
                    // received messages, but if we are in LocalPrepared and we have all the
                    // evidence, we would have proposed this too so we can continue.
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
                        local_committee_shard.count_distinct_shards(tx_rec.transaction().evidence.shards_iter());
                    let distinct_shards = NonZeroU64::new(distinct_shards as u64).ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Distinct shards is zero for transaction {} in block {}",
                            tx_rec.transaction_id(),
                            block,
                        ))
                    })?;

                    if t.leader_fee.is_none() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Leader fee for tx {} is None for Accept command in block {}",
                            t.id,
                            block,
                        );
                        return Ok(None);
                    }

                    let calculated_leader_fee = tx_rec.calculate_leader_fee(distinct_shards, EXHAUST_DIVISOR);
                    if calculated_leader_fee != *t.leader_fee.as_ref().expect("None already checked") {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block,
                            t.leader_fee.as_ref().expect("None already checked"),
                            calculated_leader_fee
                        );

                        return Ok(None);
                    }
                    total_leader_fee += calculated_leader_fee.fee();
                    // If the decision was changed to Abort, which can only happen when a foreign shard decides
                    // ABORT, and we decide COMMIT, we set SomePrepared, otherwise
                    // AllPrepared. There are no further stages after these, so these MUST
                    // never be ready to propose.
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
                Command::ForeignProposal(_) => {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Foreign proposal command for block {}. Not voting.",
                        block,
                    );
                },
                // This was already handled above
                Command::EpochEvent(_) => {},
            }
        }

        if total_leader_fee != block.total_leader_fee() {
            warn!(
                target: LOG_TARGET,
                "❌ Leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                block,
                block.total_leader_fee(),
                total_leader_fee
            );
            return Ok(None);
        }

        Ok(Some(QuorumDecision::Accept))
    }

    // Returns the execution result of a transaction.
    // If the transaction has all inputs with specific versions, it was executed in the mempool so we only fetch the
    // result from database. If the transaction has one or more inputs without version, we execute it now with the
    // most recent input versions it needs.
    fn get_executed_transaction(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction_id: &TransactionId,
        executor: &mut Box<dyn BlockTransactionExecutor<TConsensusSpec::StateStore>>,
    ) -> Result<ExecutedTransaction, HotStuffError> {
        let executed = ExecutedTransaction::get(tx.deref_mut(), transaction_id)?;
        let transaction = executed.transaction();

        // TODO: currently, we can have transactions that involve no shards (CreateFreeTestCoin). So we need to execute
        // in this case too.
        if transaction.has_inputs_without_version() || transaction.num_involved_shards() == 0 {
            let executed = executor
                .execute(executed.transaction().clone(), tx)
                .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;
            Ok(executed)
        } else {
            Ok(executed)
        }
    }

    fn lock_inputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &Transaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<bool, HotStuffError> {
        // For now we are going to only lock inputs with specific versions
        // TODO: for inputs without version, investigate if we need to use the results of re-execution
        let inputs: Vec<SubstateAddress> = transaction
            .inputs()
            .iter()
            .chain(transaction.filled_inputs())
            .filter(|i| i.version().is_some())
            .map(|i| i.to_substate_address())
            .collect::<Vec<_>>();

        let state = SubstateRecord::try_lock_all(
            tx,
            transaction.id(),
            local_committee_shard.filter(inputs.iter()),
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

        // TODO: Same as before, for inputs without version, investigate if we need to use the results of re-execution
        let inputs: Vec<SubstateAddress> = transaction
            .input_refs()
            .iter()
            .filter(|i| i.version().is_some())
            .map(|i| i.to_substate_address())
            .collect();

        let state = SubstateRecord::try_lock_all(
            tx,
            transaction.id(),
            local_committee_shard.filter(inputs.iter()),
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
        locked_inputs: &mut HashSet<SubstateAddress>,
    ) -> Result<bool, HotStuffError> {
        // TODO: for inputs without version, investigate if we need to use the results of re-execution
        let inputs = local_committee_shard
            .filter(
                transaction
                    .inputs()
                    .iter()
                    .chain(transaction.filled_inputs())
                    .filter(|i| i.version().is_some())
                    .map(|i| i.to_substate_address()),
            )
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
        // TODO: Same as before, for inputs without version, investigate if we need to use the results of re-execution
        let inputs = local_committee_shard
            .filter(
                transaction
                    .input_refs()
                    .iter()
                    .filter(|i| i.version().is_some())
                    .map(|i| i.to_substate_address()),
            )
            .collect::<HashSet<_>>();
        let state = SubstateRecord::check_lock_all(tx, inputs.iter(), SubstateLockFlag::Read)?;

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
        // We ignore inputs without version
        let write_inputs: Vec<SubstateAddress> = transaction
            .inputs()
            .iter()
            .chain(transaction.filled_inputs())
            .filter(|i| i.version().is_some())
            .map(|i| i.to_substate_address())
            .collect();
        SubstateRecord::try_unlock_many(
            tx,
            transaction.id(),
            local_committee_shard.filter(write_inputs.iter()),
            SubstateLockFlag::Write,
        )?;
        // We ignore inputs without version
        let read_inputs: Vec<SubstateAddress> = transaction
            .input_refs()
            .iter()
            .filter(|i| i.version().is_some())
            .map(|i| i.to_substate_address())
            .collect();
        SubstateRecord::try_unlock_many(
            tx,
            transaction.id(),
            local_committee_shard.filter(read_inputs.iter()),
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
            warn!(
                target: LOG_TARGET,
                "❌ Unable to lock all outputs for transaction {}: {:?}",
                transaction.id(),
                state,
            );
            return Ok(false);
        }

        Ok(true)
    }

    fn check_lock_outputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        transaction: &ExecutedTransaction,
        locked_outputs: &mut HashSet<VersionedSubstateId>,
        locked_inputs: &HashSet<SubstateAddress>,
    ) -> Result<bool, HotStuffError> {
        let resulting_outputs = transaction.resulting_outputs();
        if resulting_outputs.is_empty() {
            return Ok(true);
        }

        if resulting_outputs
            .iter()
            .any(|i| locked_outputs.contains(i) || locked_inputs.contains(&i.to_substate_address()))
        {
            warn!(
                target: LOG_TARGET,
                "❌ Locks for transaction {} conflict with other transactions in the block",
                transaction.id(),
            );
            return Ok(false);
        }

        let state = LockedOutput::check_locks(tx, resulting_outputs)?;
        if !state.is_acquired() {
            warn!(
                target: LOG_TARGET,
                "❌ Unable to lock all outputs for transaction {}: {:?}",
                transaction.id(),
                state,
            );
            return Ok(false);
        }

        locked_outputs.extend(resulting_outputs.iter().cloned());

        Ok(true)
    }

    fn unlock_outputs(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &ExecutedTransaction,
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        LockedOutput::try_release_all(
            tx,
            local_committee_shard.filter(transaction.resulting_outputs().iter().map(|v| v.to_substate_address())),
        )?;
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

    fn on_commit(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        last_executed: &LastExecuted,
        block: &Block,
        local_committee_shard: &CommitteeShard,
    ) -> Result<Vec<TransactionAtom>, HotStuffError> {
        let committed_transactions = self.execute(tx, block, local_committee_shard)?;
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
        Ok(committed_transactions)
    }

    // Returns the number processed blocks
    fn on_lock_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        locked: &LockedBlock,
        block: &Block,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔒️ LOCKED BLOCK: {}",
            block,
        );

        for foreign_proposal in block.all_foreign_proposals() {
            foreign_proposal.upsert(tx)?;
        }

        // self.processed_locked_commands(tx, local_committee_shard, block)?;
        // This moves the stage update from pending to current for all transactions on on the locked block
        self.transaction_pool.confirm_all_transitions(
            tx,
            locked,
            &block.as_locked_block(),
            block.all_transaction_ids(),
        )?;

        Ok(())
    }

    async fn propose_newly_locked_blocks(&mut self, blocks: Vec<Block>) -> Result<(), HotStuffError> {
        for block in blocks {
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
            info!(target:LOG_TARGET,"WTF epoch: {:?}",block.epoch());
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
                self.proposer.broadcast_proposal_foreignly(block).await?;
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
        block: &Block,
        local_committee_shard: &CommitteeShard,
    ) -> Result<Vec<TransactionAtom>, HotStuffError> {
        let mut finalized_transactions = Vec::with_capacity(
            block
                .commands()
                .iter()
                .filter(|cmd| matches!(cmd, Command::Accept(_)))
                .count(),
        );
        let mut total_transaction_fee = 0;
        for cmd in block.commands() {
            match cmd {
                Command::Prepare(_t) => {},
                Command::LocalPrepared(_t) => {
                    // TODO: Check if it's ok to unlock the inputs for ABORT at this point
                },
                Command::LocalOnly(t) | Command::Accept(t) => {
                    let tx_rec = self.transaction_pool.get(tx, block.as_leaf_block(), &t.id)?;
                    debug!(
                        target: LOG_TARGET,
                        "Transaction {} is finalized ({})", tx_rec.transaction_id(), t.decision
                    );

                    total_transaction_fee += tx_rec.transaction().transaction_fee;

                    let mut executed = t.get_executed_transaction(tx.deref_mut())?;
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
                            .commit_transaction(tx, block, &executed, local_committee_shard)
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
                        "🗑️ Removing transaction {} from pool", tx_rec.transaction_id()
                    );
                    tx_rec.remove(tx)?;
                    executed.set_final_decision(t.decision).update(tx)?;
                    finalized_transactions.push(t.clone());
                },
                Command::ForeignProposal(_) => {},
                Command::EpochEvent(_) => {},
            }
        }

        block.commit(tx)?;

        // We don't store (empty) pending state diffs for dummy blocks
        if !block.is_dummy() {
            let pending = PendingStateTreeDiff::remove_by_block(tx, block.id())?;
            let mut state_tree = tari_state_tree::SpreadPrefixStateTree::new(tx);
            state_tree.commit_diff(pending.diff)?;
        }

        if total_transaction_fee > 0 {
            info!(
                target: LOG_TARGET,
                "🪙 Validator fee for block {} ({}, Total Fees Paid = {})",
                block.proposed_by(),
                block.total_leader_fee(),
                total_transaction_fee
            );
        }

        Ok(finalized_transactions)
    }
}
