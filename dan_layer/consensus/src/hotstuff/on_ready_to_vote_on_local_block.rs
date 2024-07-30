//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#![allow(dead_code)]
use std::num::NonZeroU64;

use log::*;
use tari_dan_common_types::{committee::CommitteeInfo, optional::Optional, Epoch};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockDiff,
        BlockId,
        Command,
        Decision,
        LastExecuted,
        LastVoted,
        LockedBlock,
        PendingStateTreeDiff,
        QuorumDecision,
        SubstateLockFlag,
        TransactionAtom,
        TransactionExecution,
        TransactionPool,
        TransactionPoolStage,
        TransactionRecord,
        ValidBlock,
        VersionedSubstateIdLockIntent,
    },
    StateStore,
    StateStoreWriteTransaction,
};
use tari_transaction::TransactionId;
use tokio::sync::broadcast;

use crate::{
    hotstuff::{
        block_change_set::{BlockDecision, ProposedBlockChangeSet},
        calculate_state_merkle_root,
        error::HotStuffError,
        event::HotstuffEvent,
        substate_store::{PendingSubstateStore, ShardedStateTree},
        HotstuffConfig,
        ProposalValidationError,
        EXHAUST_DIVISOR,
    },
    traits::{BlockTransactionExecutor, ConsensusSpec, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_ready_to_vote_on_local_block";

#[derive(Debug, Clone)]
pub struct OnReadyToVoteOnLocalBlock<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_events: broadcast::Sender<HotstuffEvent>,
    transaction_executor: TConsensusSpec::TransactionExecutor,
}

impl<TConsensusSpec> OnReadyToVoteOnLocalBlock<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        validator_addr: TConsensusSpec::Addr,
        config: HotstuffConfig,
        store: TConsensusSpec::StateStore,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        transaction_executor: TConsensusSpec::TransactionExecutor,
    ) -> Self {
        Self {
            local_validator_addr: validator_addr,
            config,
            store,
            transaction_pool,
            tx_events,
            transaction_executor,
        }
    }

    pub fn handle(
        &mut self,
        valid_block: &ValidBlock,
        local_committee_info: CommitteeInfo,
        can_propose_epoch_end: bool,
    ) -> Result<BlockDecision, HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 LOCAL PROPOSAL READY: {}",
            valid_block,
        );

        self.store.with_write_tx(|tx| {
            let change_set = self.decide_on_block(&**tx, &local_committee_info, valid_block, can_propose_epoch_end)?;

            let mut locked_blocks = Vec::new();
            let mut finalized_transactions = Vec::new();
            let mut end_of_epoch = None;

            if change_set.is_accept() {
                // Update nodes
                valid_block.block().update_nodes(
                    tx,
                    |tx, locked, block| {
                        locked_blocks.push(block.clone());
                        self.on_lock_block(tx, locked, block)
                    },
                    |tx, last_exec, commit_block| {
                        let committed = self.on_commit(tx, last_exec, commit_block, &local_committee_info)?;
                        if commit_block.is_epoch_end() {
                            end_of_epoch = Some(commit_block.epoch());
                        }
                        if !committed.is_empty() {
                            finalized_transactions.push(committed);
                        }
                        Ok(())
                    },
                )?;
            }

            if change_set.is_accept() {
                valid_block.block().as_last_voted().set(tx)?;
            }

            let quorum_decision = change_set.quorum_decision();
            change_set.save(tx)?;

            Ok::<_, HotStuffError>(BlockDecision {
                quorum_decision,
                locked_blocks,
                finalized_transactions,
                end_of_epoch,
            })
        })
    }

    fn decide_on_block(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        local_committee_info: &CommitteeInfo,
        valid_block: &ValidBlock,
        can_propose_epoch_end: bool,
    ) -> Result<ProposedBlockChangeSet, HotStuffError> {
        // let qc_block = valid_block.block().justify().get_block(tx)?;
        // let locked_block = LockedBlock::get(tx)?.get_block(tx)?;

        // If the previous qc block was in different epoch, we have to have EpochEvent::Start
        // let epoch_should_start = qc_block.epoch() < valid_block.epoch();

        if !self.should_vote(tx, valid_block.block())? {
            return Ok(ProposedBlockChangeSet::new(valid_block.block().as_leaf_block()).no_vote());
        }

        self.decide_what_to_vote(tx, valid_block.block(), local_committee_info, can_propose_epoch_end)
    }

    /// if b_new .height > vheight && (b_new extends b_lock || b_new .justify.node.height > b_lock .height)
    ///
    /// If we have not previously voted on this block and the node extends the current locked node, then we vote
    fn should_vote(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
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

    #[allow(clippy::too_many_lines)]
    fn decide_what_to_vote(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        local_committee_info: &CommitteeInfo,
        can_propose_epoch_end: bool,
    ) -> Result<ProposedBlockChangeSet, HotStuffError> {
        let mut total_leader_fee = 0;

        // Store used for transactions that have inputs without specific versions.
        // It lives through the entire block so multiple transactions can be sequenced together in the same block
        // let tree_store = ChainScopedTreeStore::new(block.epoch(), block.shard_group(), tx);
        let mut substate_store = PendingSubstateStore::new(tx, *block.parent(), self.config.num_preshards);
        let mut proposed_block_change_set = ProposedBlockChangeSet::new(block.as_leaf_block());

        if block.is_epoch_end() && block.commands().len() > 1 {
            warn!(
                target: LOG_TARGET,
                "❌ EpochEvent::End command in block {} but block contains other commands",
                block.id()
            );
            return Ok(proposed_block_change_set.no_vote());
        }

        for cmd in block.commands() {
            if let Some(foreign_proposal) = cmd.foreign_proposal() {
                if !foreign_proposal.exists(tx)? {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Foreign proposal for block {block_id} from bucket {bucket} does not exist in the store",
                        block_id = foreign_proposal.block_id,bucket = foreign_proposal.shard_group
                    );
                    return Ok(proposed_block_change_set.no_vote());
                }
                continue;
            }
            if cmd.is_epoch_end() {
                if !can_propose_epoch_end {
                    warn!(
                        target: LOG_TARGET,
                        "❌ EpochEvent::End command received for block {} but it is not the next epoch",
                        block.id(),
                    );
                    return Ok(proposed_block_change_set.no_vote());
                }
                continue;
            }

            let atom = cmd
                .transaction()
                .expect("all remaining commands have a transaction atom");

            let Some(mut tx_rec) = self
                .transaction_pool
                .get(tx, block.as_leaf_block(), atom.id())
                .optional()?
            else {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                    block,
                    cmd.id(),
                );
                return Ok(proposed_block_change_set.no_vote());
            };

            // TODO: we probably need to provide the all/some of the QCs referenced in local transactions as
            //       part of the proposal DanMessage so that there is no race condition between receiving the
            //       proposed block and receiving the foreign proposals. Because this is only added on locked block,
            //       this should be less common.
            tx_rec.add_evidence(local_committee_info, *block.justify().id());

            debug!(
                target: LOG_TARGET,
                "🔥 processing command {} for block {}",
                cmd,
                block,
            );
            match cmd {
                Command::LocalOnly(t) => {
                    info!(
                        target: LOG_TARGET,
                        "👨‍🔧 LOCAL-ONLY: Executing transaction {} in block {}",
                        tx_rec.transaction_id(),
                        block,
                    );

                    let executed =
                        self.execute_transaction_if_required(&substate_store, &atom.id, block.id(), block.epoch())?;
                    tx_rec.set_local_decision(executed.decision());
                    tx_rec.set_evidence(executed.to_initial_evidence());
                    tx_rec.set_transaction_fee(executed.transaction_fee());
                    proposed_block_change_set.add_transaction_execution(executed);

                    if !tx_rec.current_stage().is_new() && !tx_rec.current_stage().is_local_only() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for tx {} in block {}. Leader proposed LocalOnly, local stage is {}",
                            tx_rec.transaction_id(),
                            block,
                            tx_rec.current_stage(),
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if tx_rec.transaction_fee() != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ LocalOnly transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block,
                            t.transaction_fee,
                            tx_rec.transaction_fee()
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    // If the leader proposed to commit a transaction that we want to abort, we abstain from voting
                    // If the leader proposed to abort a transaction that we want to commit, perhaps the transaction has
                    // a lock conflict, so we'll need to check this.
                    if tx_rec.current_decision().is_abort() && t.decision.is_commit() {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for tx {} in block {}. Leader proposed {}, we decided {}",
                            tx_rec.transaction_id(),
                            block,
                            t.decision,
                            tx_rec.current_decision()
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if !local_committee_info
                        .includes_all_substate_addresses(tx_rec.evidence().substate_addresses_iter())
                    {
                        warn!(
                            target: LOG_TARGET,
                            "❌ LocalOnly transaction {} in block {} has more than one involved shard",
                            t.id,
                            block,
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if tx_rec.current_decision().is_commit() {
                        let execution =
                            proposed_block_change_set
                                .get_transaction_execution(&t.id)
                                .ok_or_else(|| {
                                    HotStuffError::InvariantError(format!(
                                        "Transaction execution not found for transaction {} in block {}",
                                        t.id, block
                                    ))
                                })?;
                        // TODO: If we proposed this block, we shouldn't have to reprocess the locks. Locks are
                        // currently fairly expensive.
                        if !self.try_obtain_locks(execution, local_committee_info, &mut substate_store)? {
                            // They want to ABORT a successfully executed transaction because of a lock conflict, which
                            // we also have.
                            if t.decision.is_abort() {
                                warn!(
                                    target: LOG_TARGET,
                                    "🔥 Proposer chose to ABORT and we chose to ABORT due to lock conflict for transaction {} in block {}",
                                    block,
                                    tx_rec.transaction_id(),
                                );
                                // TODO: Add a reason for the ABORT. Perhaps a reason enum
                                //       Decision::Abort(AbortReason::LockConflict)
                                tx_rec.set_local_decision(Decision::Abort);
                                proposed_block_change_set.set_next_transaction_update(
                                    &tx_rec,
                                    TransactionPoolStage::LocalOnly,
                                    false,
                                );
                                continue;
                            }

                            warn!(
                                target: LOG_TARGET,
                                "❌ Failed to lock inputs/outputs for transaction {}. Not voting for block {}",
                                block,
                                tx_rec.transaction_id(),
                            );
                            return Ok(proposed_block_change_set.no_vote());
                        }

                        // If we've decided COMMIT and they decided ABORT, we need to abstain from voting
                        if t.decision.is_abort() {
                            warn!(
                                target: LOG_TARGET,
                                "❌ LocalOnly decision disagreement for transaction {} in block {}. Leader proposed {}, we decided {}",
                                tx_rec.transaction_id(),
                                block,
                                t.decision,
                                tx_rec.current_decision()
                            );
                            return Ok(proposed_block_change_set.no_vote());
                        }

                        if let Some(diff) = execution.result.finalize.accept() {
                            if let Err(err) = substate_store.put_diff(t.id, diff) {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Failed to store diff for transaction {} in block {}. Error: {}",
                                    block,
                                    tx_rec.transaction_id(),
                                    err
                                );
                                let _err = err.ok_or_fatal_error()?;
                                return Ok(proposed_block_change_set.no_vote());
                            }
                        }

                        if t.leader_fee.is_none() {
                            warn!(
                                target: LOG_TARGET,
                                "❌ Leader fee for tx {} is None for LocalOnly command in block {}",
                                t.id,
                                block,
                            );
                            return Ok(proposed_block_change_set.no_vote());
                        }

                        let calculated_leader_fee =
                            tx_rec.calculate_leader_fee(NonZeroU64::new(1).expect("1 > 0"), EXHAUST_DIVISOR);
                        if calculated_leader_fee != *t.leader_fee.as_ref().expect("None already checked") {
                            warn!(
                                target: LOG_TARGET,
                                "❌ LocalOnly leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                                block,
                                t.leader_fee.as_ref().expect("None already checked"),
                                calculated_leader_fee
                            );

                            return Ok(proposed_block_change_set.no_vote());
                        }
                        total_leader_fee += calculated_leader_fee.fee();
                    }

                    proposed_block_change_set.set_next_transaction_update(
                        &tx_rec,
                        TransactionPoolStage::LocalOnly,
                        false,
                    );
                },
                Command::Prepare(t) => {
                    info!(
                        target: LOG_TARGET,
                        "👨‍🔧 PREPARE: Executing transaction {} in block {}",
                        tx_rec.transaction_id(),
                        block,
                    );

                    let executed =
                        self.execute_transaction_if_required(&substate_store, &atom.id, block.id(), block.epoch())?;
                    tx_rec.set_local_decision(executed.decision());
                    tx_rec.set_evidence(executed.to_initial_evidence());
                    tx_rec.set_transaction_fee(executed.transaction_fee());
                    proposed_block_change_set.add_transaction_execution(executed);

                    if !tx_rec.current_stage().is_new() && !tx_rec.current_stage().is_prepared() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Stage disagreement for tx {} in block {}. Leader proposed Prepare, local stage is {}",
                            tx_rec.transaction_id(),
                            block,
                            tx_rec.current_stage(),
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if tx_rec.transaction_fee() != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block,
                            t.transaction_fee,
                            tx_rec.transaction_fee()
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if tx_rec.current_decision().is_abort() && t.decision.is_commit() {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ Prepare decision disagreement for tx {} in block {}. Leader proposed {}, we decided {}",
                            tx_rec.transaction_id(),
                            block,
                            t.decision,
                            tx_rec.current_decision()
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if tx_rec.current_decision().is_commit() {
                        let execution =
                            proposed_block_change_set
                                .get_transaction_execution(&t.id)
                                .ok_or_else(|| {
                                    HotStuffError::InvariantError(format!(
                                        "Transaction execution not found for transaction {} in block {}",
                                        t.id, block
                                    ))
                                })?;
                        if !self.try_obtain_locks(execution, local_committee_info, &mut substate_store)? {
                            // They want to ABORT a successfully executed transaction because of a lock conflict, which
                            // we also have.
                            if t.decision.is_abort() {
                                tx_rec.set_local_decision(Decision::Abort);
                                proposed_block_change_set.set_next_transaction_update(
                                    &tx_rec,
                                    TransactionPoolStage::Prepared,
                                    true,
                                );
                                continue;
                            }
                            warn!(
                                target: LOG_TARGET,
                                "❌ Failed to lock inputs/outputs for transaction {}. Not voting for block {}",
                                block,
                                tx_rec.transaction_id(),
                            );
                            return Ok(proposed_block_change_set.no_vote());
                        }
                    }

                    proposed_block_change_set.set_next_transaction_update(
                        &tx_rec,
                        TransactionPoolStage::Prepared,
                        true,
                    );
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
                        return Ok(proposed_block_change_set.no_vote());
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
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if tx_rec.transaction_fee() != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                            tx_rec.transaction_id(),
                            block,
                            t.transaction_fee,
                            tx_rec.transaction_fee()
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    proposed_block_change_set.set_next_transaction_update(
                        &tx_rec,
                        TransactionPoolStage::LocalPrepared,
                        tx_rec.evidence().all_shards_justified(),
                    );
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
                        return Ok(proposed_block_change_set.no_vote());
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
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if !tx_rec.evidence().all_shards_justified() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept evidence disagreement tx {} in block {}. Evidence for {} out of {} shards",
                            tx_rec.transaction_id(),
                            block,
                            tx_rec.evidence().num_justified_shards(),
                            tx_rec.evidence().len(),
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    if tx_rec.transaction_fee() != t.transaction_fee {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Accept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                            tx_rec.transaction_id(),
                            block,
                            t.transaction_fee,
                            tx_rec.transaction_fee()
                        );

                        return Ok(proposed_block_change_set.no_vote());
                    }

                    // Check if we have LocalPrepared ready i.e. LocalPrepared from all shards
                    // It is possible that the transaction was not marked as ready yet because of the order we
                    // received messages, but if we are in LocalPrepared and we have all the
                    // evidence, we would have proposed this too so we can continue.
                    if !tx_rec.is_ready() && !tx_rec.evidence().all_shards_justified() {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ Local proposal received ({}) for transaction {} which is not ready. Not voting.",
                            block,
                            tx_rec.transaction_id()
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    let distinct_shard_groups =
                        local_committee_info.count_distinct_shard_groups(tx_rec.evidence().substate_addresses_iter());
                    let distinct_shards = NonZeroU64::new(distinct_shard_groups as u64).ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Distinct shards is zero for transaction {} in block {}",
                            tx_rec.transaction_id(),
                            block,
                        ))
                    })?;

                    // If the decision was changed to Abort, which can only happen when a foreign shard decides
                    // ABORT, and we decide COMMIT, we set SomePrepared, otherwise
                    // AllPrepared. There are no further stages after these, so these MUST
                    // never be ready to propose.
                    if tx_rec.current_decision().is_abort() {
                        proposed_block_change_set.set_next_transaction_update(
                            &tx_rec,
                            TransactionPoolStage::SomePrepared,
                            false,
                        );
                    } else {
                        proposed_block_change_set.set_next_transaction_update(
                            &tx_rec,
                            TransactionPoolStage::AllPrepared,
                            false,
                        );

                        if t.leader_fee.is_none() {
                            warn!(
                                target: LOG_TARGET,
                                "❌ Leader fee for tx {} is None for Accept command in block {}",
                                t.id,
                                block,
                            );
                            return Ok(proposed_block_change_set.no_vote());
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

                            return Ok(proposed_block_change_set.no_vote());
                        }
                        total_leader_fee += calculated_leader_fee.fee();

                        let execution = TransactionExecution::get_pending_for_block(tx, &t.id, block.parent())?;
                        if let Some(diff) = execution.result().finalize.accept() {
                            if let Err(err) = substate_store.put_diff(t.id, diff) {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Failed to store diff for transaction {} in block {}. Error: {}",
                                    block,
                                    tx_rec.transaction_id(),
                                    err
                                );
                                let _err = err.ok_or_fatal_error()?;
                                return Ok(proposed_block_change_set.no_vote());
                            }
                        }
                    }
                },
                Command::ForeignProposal(_proposal) => {
                    // TODO: this is not correct. we need to check the proposal and no-vote if invalid
                    warn!(
                        target: LOG_TARGET,
                        "❌ Foreign proposal command for block {}. Not voting.",
                        block,
                    );
                },
                // This was already handled above
                Command::EndEpoch => {},
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
            // TODO: investigate
            // return Ok(proposed_block_change_set.no_vote());
        }

        let pending = PendingStateTreeDiff::get_all_up_to_commit_block(tx, block.justify().block_id())?;
        let (expected_merkle_root, tree_diffs) =
            calculate_state_merkle_root(tx, block.shard_group(), pending, substate_store.diff())?;
        if expected_merkle_root != *block.merkle_root() {
            warn!(
                target: LOG_TARGET,
                "❌ Merkle root disagreement for block {}. Leader proposed {}, we calculated {}",
                block,
                block.merkle_root(),
                expected_merkle_root
            );
            return Ok(proposed_block_change_set.no_vote());
        }

        let (diff, locks) = substate_store.into_parts();
        proposed_block_change_set
            .set_block_diff(diff)
            .set_state_tree_diffs(tree_diffs)
            .set_substate_locks(locks)
            .set_quorum_decision(QuorumDecision::Accept);

        Ok(proposed_block_change_set)
    }

    /// Executes the given transaction. If the transaction has already been executed for this block (on propose) then we
    /// load without re-executing.
    fn execute_transaction_if_required(
        &self,
        store: &PendingSubstateStore<TConsensusSpec::StateStore>,
        transaction_id: &TransactionId,
        block_id: &BlockId,
        current_epoch: Epoch,
    ) -> Result<TransactionExecution, HotStuffError> {
        // If the transaction is already executed in the propose phase we simply load it for this block
        if let Some(execution) =
            TransactionExecution::get_by_block(store.read_transaction(), transaction_id, block_id).optional()?
        {
            return Ok(execution);
        }

        let transaction = TransactionRecord::get(store.read_transaction(), transaction_id)?;

        let executed = self
            .transaction_executor
            .execute(transaction.into_transaction(), store, current_epoch)
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        Ok(executed.into_execution_for_block(*block_id))
    }

    fn try_obtain_locks(
        &self,
        transaction_execution: &TransactionExecution,
        local_committee_info: &CommitteeInfo,
        store: &mut PendingSubstateStore<'_, '_, TConsensusSpec::StateStore>,
    ) -> Result<bool, HotStuffError> {
        let is_local_only = local_committee_info.includes_all_substate_addresses(
            transaction_execution
                .resolved_inputs()
                .iter()
                .map(|i| i.to_substate_address())
                .chain(
                    transaction_execution
                        .resulting_outputs()
                        .iter()
                        .map(|id| id.to_substate_address()),
                ),
        );

        let objects = transaction_execution.resolved_inputs().iter().cloned().chain(
            transaction_execution
                .resulting_outputs()
                .iter()
                .map(|id| VersionedSubstateIdLockIntent::new(id.clone(), SubstateLockFlag::Output)),
        );
        if let Err(err) = store.try_lock_all(*transaction_execution.transaction_id(), objects, is_local_only) {
            let err = err.ok_or_fatal_error()?;
            warn!(
                target: LOG_TARGET,
                "❌ Failed to lock inputs/outputs for transaction {} because {err}", transaction_execution.transaction_id()
            );
            return Ok(false);
        }
        Ok(true)
    }

    fn on_commit(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        last_executed: &LastExecuted,
        block: &Block,
        local_committee_info: &CommitteeInfo,
    ) -> Result<Vec<TransactionAtom>, HotStuffError> {
        let committed_transactions = self.finalize_block(tx, block, local_committee_info)?;
        debug!(
            target: LOG_TARGET,
            "✅ COMMIT block {}, last executed height = {}",
            block,
            last_executed.height
        );
        self.publish_event(HotstuffEvent::BlockCommitted {
            epoch: block.epoch(),
            block_id: *block.id(),
            height: block.height(),
        });
        Ok(committed_transactions)
    }

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

        // This moves the stage update from pending to current for all transactions on the locked block
        self.transaction_pool.confirm_all_transitions(
            tx,
            locked,
            &block.as_locked_block(),
            block.all_transaction_ids(),
        )?;

        Ok(())
    }

    fn publish_event(&self, event: HotstuffEvent) {
        let _ignore = self.tx_events.send(event);
    }

    fn finalize_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        local_committee_info: &CommitteeInfo,
    ) -> Result<Vec<TransactionAtom>, HotStuffError> {
        if block.is_dummy() {
            // Nothing to do here for empty dummy blocks. Just mark the block as committed.
            block.commit_diff(tx, BlockDiff::empty(*block.id()))?;
            return Ok(vec![]);
        }

        let diff = block.get_diff(&**tx)?;
        info!(
            target: LOG_TARGET,
            "🌳 Committing block {} with {} substate change(s)", block, diff.len()
        );

        // NOTE: this must happen before we commit the diff because the state transitions use this version
        let pending = PendingStateTreeDiff::remove_by_block(tx, block.id())?;
        let mut state_tree = ShardedStateTree::new(tx);
        state_tree.commit_diff(pending)?;
        let tx = state_tree.into_transaction();

        let local_diff = diff.into_filtered(local_committee_info);
        block.commit_diff(tx, local_diff)?;

        let finalized_transactions = self
            .transaction_pool
            .remove_all(tx, block.all_accepted_transactions_ids())?;
        TransactionRecord::finalize_all(tx, *block.id(), &finalized_transactions)?;

        if !finalized_transactions.is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ {} transactions finalized",
                finalized_transactions.len(),
            );
        }

        // Remove locks for finalized transactions
        tx.substate_locks_remove_many_for_transactions(block.all_accepted_transactions_ids())?;

        let total_transaction_fee = block.total_transaction_fee();
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
