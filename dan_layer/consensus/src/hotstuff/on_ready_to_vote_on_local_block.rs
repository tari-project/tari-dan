//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#![allow(dead_code)]
use std::num::NonZeroU64;

use log::*;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_dan_common_types::{committee::CommitteeInfo, optional::Optional, Epoch};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockDiff,
        BlockId,
        BlockTransactionExecution,
        Command,
        Decision,
        ForeignProposalAtom,
        LastExecuted,
        LastVoted,
        LockedBlock,
        MintConfidentialOutputAtom,
        PendingShardStateTreeDiff,
        QuorumDecision,
        SubstateChange,
        SubstateRecord,
        TransactionAtom,
        TransactionPool,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
        ValidBlock,
    },
    StateStore,
};
use tari_engine_types::{commit_result::RejectReason, substate::Substate};
use tari_transaction::{TransactionId, VersionedSubstateId};
use tokio::sync::broadcast;

use crate::{
    hotstuff::{
        block_change_set::{BlockDecision, ProposedBlockChangeSet},
        calculate_state_merkle_root,
        error::HotStuffError,
        event::HotstuffEvent,
        filter_diff_for_committee,
        substate_store::{PendingSubstateStore, ShardedStateTree},
        transaction_manager::{
            ConsensusTransactionManager,
            LocalPreparedTransaction,
            PledgedTransaction,
            PreparedTransaction,
        },
        HotstuffConfig,
        ProposalValidationError,
        EXHAUST_DIVISOR,
    },
    traits::{ConsensusSpec, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_ready_to_vote_on_local_block";

#[derive(Debug, Clone)]
pub struct OnReadyToVoteOnLocalBlock<TConsensusSpec: ConsensusSpec> {
    local_validator_pk: RistrettoPublicKey,
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_events: broadcast::Sender<HotstuffEvent>,
    transaction_manager: ConsensusTransactionManager<TConsensusSpec::TransactionExecutor, TConsensusSpec::StateStore>,
}

impl<TConsensusSpec> OnReadyToVoteOnLocalBlock<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        local_validator_pk: RistrettoPublicKey,
        config: HotstuffConfig,
        store: TConsensusSpec::StateStore,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        transaction_manager: ConsensusTransactionManager<
            TConsensusSpec::TransactionExecutor,
            TConsensusSpec::StateStore,
        >,
    ) -> Self {
        Self {
            local_validator_pk,
            config,
            store,
            transaction_pool,
            tx_events,
            transaction_manager,
        }
    }

    pub fn handle(
        &mut self,
        valid_block: &ValidBlock,
        local_committee_info: &CommitteeInfo,
        can_propose_epoch_end: bool,
    ) -> Result<BlockDecision, HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 LOCAL PROPOSAL READY: {}",
            valid_block,
        );

        self.store.with_write_tx(|tx| {
            let mut change_set =
                self.decide_on_block(&**tx, local_committee_info, valid_block, can_propose_epoch_end)?;

            let mut locked_blocks = Vec::new();
            let mut finalized_transactions = Vec::new();
            let mut end_of_epoch = None;

            if change_set.is_accept() {
                // Update nodes
                let leaf_block = valid_block.block().update_nodes(
                    tx,
                    |tx, locked, block, justify_qc| {
                        if !block.is_dummy() {
                            locked_blocks.push((block.clone(), justify_qc.clone()));
                        }
                        self.on_lock_block(tx, locked, block)
                    },
                    |tx, last_exec, commit_block| {
                        let committed = self.on_commit(tx, last_exec, commit_block, local_committee_info)?;
                        if commit_block.is_epoch_end() {
                            end_of_epoch = Some(commit_block.epoch());
                        }
                        if !committed.is_empty() {
                            finalized_transactions.push(committed);
                        }
                        Ok(())
                    },
                )?;

                if !leaf_block.is_justified() {
                    self.process_newly_justified_block(tx, leaf_block, local_committee_info, &mut change_set)?;
                }

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

    fn process_newly_justified_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        mut new_leaf_block: Block,
        local_committee_info: &CommitteeInfo,
        change_set: &mut ProposedBlockChangeSet,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "✅ New leaf block {} is justified. Updating evidence for transactions",
            new_leaf_block,
        );
        new_leaf_block.set_as_justified(tx)?;

        let leaf = new_leaf_block.as_leaf_block();
        let justify_id = *new_leaf_block.justify().id();
        for cmd in new_leaf_block.commands() {
            let Some(atom) = cmd.progressing() else {
                continue;
            };

            // CASE: This code checks if the new leaf block causes the transaction to be ready.
            // For example, suppose a transaction is LocalPrepared in the currently proposed block, however it
            // is not yet the leaf block (because it has not been justified). If we then
            // receive the foreign LocalPrepared for this transaction, we evaluate the
            // foreign LocalPrepare using the transaction state as it "was" without
            // considering data from the as yet unjustified block. This means that we do not
            // recognise that the transaction is ready for AllPrepared, and we never propose
            // it. This code reevaluates the new leaf blocks and sets any transactions to
            // ready that have the required evidence.

            if let Some(update_mut) = change_set.next_update_mut(atom.id()) {
                // The leaf block already approved finalising this transaction (i.e. the justify block proposed
                // LocalAccept, the leaf proposed (some|all)Accept therefore we already have all evidence). This
                // transaction will not be proposed again and will be finalised.
                if update_mut.stage.is_finalising() {
                    continue;
                }

                update_mut
                    .evidence_mut()
                    .add_qc_evidence(local_committee_info, justify_id);

                if !update_mut.is_ready {
                    let local_prepare_is_justified =
                        update_mut.stage.is_local_prepared() && update_mut.evidence().all_input_addresses_justified();
                    let local_accept_is_justified =
                        update_mut.stage.is_local_accepted() && update_mut.evidence().all_addresses_justified();
                    if local_prepare_is_justified {
                        info!(
                            target: LOG_TARGET,
                            "✅ All inputs justified for transaction {} in block {} is now ready for AllPrepared",
                            atom.id(),
                            leaf,
                        );
                        update_mut.is_ready = true;
                    } else if local_accept_is_justified {
                        info!(
                            target: LOG_TARGET,
                            "✅ All inputs justified for transaction {} in block {} is now ready for AllAccepted",
                            atom.id(),
                            leaf,
                        );
                        update_mut.is_ready = true;
                    } else {
                        // Nothing - we've updated the evidence and is_ready remains false
                        debug!(
                            target: LOG_TARGET,
                            "Transaction {} stage: {} still not ready",
                            atom.id(),
                            update_mut.stage
                        );
                    }
                }
            } else {
                // No update from current leaf, so the current pool data is the latest
                let Some(mut pool_tx) = self.transaction_pool.get(tx, leaf, atom.id()).optional()? else {
                    return Err(HotStuffError::InvariantError(format!(
                        "Transaction {} in newly justified block {} not found in the pool",
                        atom.id(),
                        leaf,
                    )));
                };

                pool_tx.add_qc_evidence(local_committee_info, justify_id);

                if pool_tx.is_ready() {
                    change_set.set_next_transaction_update(&pool_tx, pool_tx.current_stage(), true)?;
                } else {
                    let local_prepare_is_justified = cmd
                        .local_prepare()
                        .map(|_| pool_tx.evidence().all_input_addresses_justified())
                        .unwrap_or(false);
                    let local_accept_is_justified = cmd
                        .local_accept()
                        .map(|_| pool_tx.evidence().all_addresses_justified())
                        .unwrap_or(false);
                    let is_ready = local_prepare_is_justified || local_accept_is_justified;

                    // Some logs
                    if local_prepare_is_justified {
                        info!(
                            target: LOG_TARGET,
                            "✅ All inputs justified for transaction {} in block {} is now ready for AllPrepared",
                            atom.id(),
                            leaf,
                        );
                    } else if local_accept_is_justified {
                        info!(
                            target: LOG_TARGET,
                            "✅ All inputs justified for transaction {} in block {} is now ready for AllAccepted",
                            atom.id(),
                            leaf,
                        );
                    } else {
                        // Nothing - we'll still update the evidence with is_ready = false
                    }
                    change_set.set_next_transaction_update(&pool_tx, pool_tx.current_stage(), is_ready)?;
                }
            }
        }

        Ok(())
    }

    fn decide_on_block(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        local_committee_info: &CommitteeInfo,
        valid_block: &ValidBlock,
        can_propose_epoch_end: bool,
    ) -> Result<ProposedBlockChangeSet, HotStuffError> {
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
        // Store used for transactions that have inputs without specific versions.
        // It lives through the entire block so multiple transactions can be sequenced together in the same block
        let mut substate_store = PendingSubstateStore::new(tx, *block.parent(), self.config.num_preshards);
        let mut proposed_block_change_set = ProposedBlockChangeSet::new(block.as_leaf_block());
        let mut total_leader_fee = 0;

        for cmd in block.commands() {
            match cmd {
                Command::LocalOnly(atom) => {
                    if !self.evaluate_local_only_command(
                        tx,
                        block,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        &mut proposed_block_change_set,
                        &mut total_leader_fee,
                    )? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::Prepare(atom) => {
                    if !self.evaluate_prepare_command(
                        tx,
                        block,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        &mut proposed_block_change_set,
                    )? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::LocalPrepare(atom) => {
                    if !self.evaluate_local_prepare_command(tx, block, atom, &mut proposed_block_change_set)? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::AllPrepare(atom) => {
                    if !self.evaluate_all_prepare_command(tx, block, atom, &mut proposed_block_change_set)? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::SomePrepare(atom) => {
                    if !self.evaluate_some_prepare_command(tx, block, atom, &mut proposed_block_change_set)? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::LocalAccept(atom) => {
                    if !self.evaluate_local_accept_command(
                        tx,
                        block,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        &mut proposed_block_change_set,
                    )? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::AllAccept(atom) => {
                    if !self.evaluate_all_accept_command(
                        tx,
                        block,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        &mut proposed_block_change_set,
                    )? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::SomeAccept(atom) => {
                    if !self.evaluate_some_accept_command(tx, block, atom, &mut proposed_block_change_set)? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::ForeignProposal(fp_atom) => {
                    if !self.evaluate_foreign_proposal_command(tx, fp_atom, &mut proposed_block_change_set)? {
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    continue;
                },
                Command::MintConfidentialOutput(atom) => {
                    if !self.evaluate_mint_confidential_output_command(
                        tx,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        &mut proposed_block_change_set,
                    )? {
                        return Ok(proposed_block_change_set.no_vote());
                    }
                },
                Command::EndEpoch => {
                    if !can_propose_epoch_end {
                        warn!(
                            target: LOG_TARGET,
                            "❌ EpochEvent::End command received for block {} but it is not the next epoch",
                            block.id(),
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }
                    if block.commands().len() > 1 {
                        warn!(
                            target: LOG_TARGET,
                            "❌ EpochEvent::End command in block {} but block contains other commands",
                            block.id()
                        );
                        return Ok(proposed_block_change_set.no_vote());
                    }

                    continue;
                },
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
            return Ok(proposed_block_change_set.no_vote());
        }

        let pending = PendingShardStateTreeDiff::get_all_up_to_commit_block(tx, block.justify().block_id())?;
        let (expected_merkle_root, tree_diffs) = calculate_state_merkle_root(
            tx,
            block.shard_group(),
            pending,
            substate_store
                .diff()
                .iter()
                // Calculate for local shards only
                .filter(|ch| block.shard_group().contains(&ch.shard())),
        )?;
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

    #[allow(clippy::too_many_lines)]
    fn evaluate_local_only_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
        total_leader_fee: &mut u64,
    ) -> Result<bool, HotStuffError> {
        let Some(mut tx_rec) = self
            .transaction_pool
            .get(tx, block.as_leaf_block(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(false);
        };

        if !tx_rec.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "❌ Stage disagreement for tx {} in block {}. Leader proposed LocalOnly, local stage is {}",
                tx_rec.transaction_id(),
                block,
                tx_rec.current_stage(),
            );
            return Ok(false);
        }

        // TODO(perf): proposer shouldn't have to do this twice, esp. executing the transaction and locking
        let prepared = self
            .transaction_manager
            .prepare(substate_store, local_committee_info, block.epoch(), *atom.id())
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        match prepared {
            PreparedTransaction::LocalOnly(LocalPreparedTransaction::Accept(executed)) => {
                let execution = executed.into_execution();
                tx_rec.update_from_execution(&execution);

                info!(
                    target: LOG_TARGET,
                    "👨‍🔧 LocalOnly: Prepare for transaction {} ({}) in block {}",
                    tx_rec.transaction_id(),
                    tx_rec.current_decision(),
                    block,
                );

                // If the leader proposed to commit a transaction that we want to abort, we abstain from voting
                if tx_rec.current_decision() != atom.decision {
                    // If we disagree with any local decision we abstain from voting
                    warn!(
                        target: LOG_TARGET,
                        "❌ Prepare decision disagreement for tx {} in block {}. Leader proposed {}, we decided {}",
                        tx_rec.transaction_id(),
                        block,
                        atom.decision,
                        tx_rec.current_decision()
                    );
                    return Ok(false);
                }

                if tx_rec.transaction_fee() != atom.transaction_fee {
                    warn!(
                        target: LOG_TARGET,
                        "❌ LocalOnly transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                        block,
                        atom.transaction_fee,
                        tx_rec.transaction_fee()
                    );
                    return Ok(false);
                }

                if tx_rec.current_decision().is_commit() {
                    if let Some(diff) = execution.result().finalize.accept() {
                        if let Err(err) = substate_store.put_diff(atom.id, diff) {
                            warn!(
                                target: LOG_TARGET,
                                "❌ Failed to store diff for transaction {} in block {}. Error: {}",
                                block,
                                tx_rec.transaction_id(),
                                err
                            );
                            let _err = err.or_fatal_error()?;
                            return Ok(false);
                        }
                    }

                    if atom.leader_fee.is_none() {
                        warn!(
                            target: LOG_TARGET,
                            "❌ Leader fee for tx {} is None for LocalOnly command in block {}",
                            atom.id,
                            block,
                        );
                        return Ok(false);
                    }

                    let calculated_leader_fee =
                        tx_rec.calculate_leader_fee(NonZeroU64::new(1).expect("1 > 0"), EXHAUST_DIVISOR);
                    if calculated_leader_fee != *atom.leader_fee.as_ref().expect("None already checked") {
                        warn!(
                            target: LOG_TARGET,
                            "❌ LocalOnly leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                            block,
                            atom.leader_fee.as_ref().expect("None already checked"),
                            calculated_leader_fee
                        );

                        return Ok(false);
                    }

                    *total_leader_fee += calculated_leader_fee.fee();
                }

                proposed_block_change_set.add_transaction_execution(execution)?;
            },
            PreparedTransaction::LocalOnly(LocalPreparedTransaction::EarlyAbort { transaction }) => {
                if atom.decision.is_commit() {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Failed to lock inputs/outputs for transaction {} but leader proposed COMMIT. Not voting for block {}",
                        tx_rec.transaction_id(),
                        block,
                    );
                    return Ok(false);
                }

                // They want to ABORT a successfully executed transaction because of a lock conflict, which
                // we also have.
                info!(
                    target: LOG_TARGET,
                    "⚠️ Proposer chose to ABORT and we chose to ABORT due to lock conflict for transaction {} in block {}",
                    block,
                    tx_rec.transaction_id(),
                );
                // TODO: Add a reason for the ABORT. Perhaps a reason enum
                //       Decision::Abort(AbortReason::LockConflict)
                let execution = transaction.into_execution().expect("Abort should have execution");
                tx_rec.update_from_execution(&execution);
                proposed_block_change_set.add_transaction_execution(execution)?;
            },
            PreparedTransaction::MultiShard(_) => {
                warn!(
                    target: LOG_TARGET,
                    "❌ transaction {} in block {} is not Local-Only but was proposed as LocalOnly",
                    atom.id(),
                    block,
                );
                return Ok(false);
            },
        }

        proposed_block_change_set.set_next_transaction_update(&tx_rec, TransactionPoolStage::LocalOnly, false)?;
        Ok(true)
    }

    fn evaluate_prepare_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        let Some(mut tx_rec) = self
            .transaction_pool
            .get(tx, block.as_leaf_block(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(false);
        };

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Executing transaction {} in block {}",
            tx_rec.transaction_id(),
            block,
        );

        if !tx_rec.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "❌ Stage disagreement for tx {} in block {}. Leader proposed Prepare, local stage is {}",
                tx_rec.transaction_id(),
                block,
                tx_rec.current_stage(),
            );
            return Ok(false);
        }

        let prepared = self
            .transaction_manager
            .prepare(substate_store, local_committee_info, block.epoch(), *atom.id())
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        match prepared {
            PreparedTransaction::LocalOnly(_) => {
                warn!(
                    target: LOG_TARGET,
                    "❌ transaction {} in block {} is Local-Only but was proposed as Prepare",
                    atom.id(),
                    block,
                );
                return Ok(false);
            },
            PreparedTransaction::MultiShard(multishard) => {
                if multishard.current_decision() != atom.decision {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Leader proposed {} for transaction {} in block {} but we decided to {}",
                        tx_rec.transaction_id(),
                        block,
                        atom.decision,
                        multishard.current_decision(),
                    );
                    return Ok(false);
                }

                match multishard.current_decision() {
                    Decision::Commit => {
                        if multishard.transaction().is_executed() {
                            // CASE: All inputs are local and outputs are foreign (i.e. the transaction is executed), or
                            let execution = multishard.into_execution().expect("Abort should have execution");
                            tx_rec.update_from_execution(&execution);
                            proposed_block_change_set.add_transaction_execution(execution)?;
                        } else {
                            // CASE: All local inputs were resolved. We need to continue with consensus to get the
                            // foreign inputs/outputs.
                            tx_rec.set_local_decision(Decision::Commit);
                            // Set partial evidence for local inputs using what we know.
                            tx_rec.set_evidence(multishard.to_initial_evidence());
                        }
                    },
                    Decision::Abort => {
                        // CASE: The transaction was ABORTed due to a lock conflict
                        let execution = multishard.into_execution().expect("Abort should have execution");
                        tx_rec.update_from_execution(&execution);
                        proposed_block_change_set.add_transaction_execution(execution)?;
                    },
                }
            },
        }

        proposed_block_change_set.set_next_transaction_update(&tx_rec, TransactionPoolStage::Prepared, true)?;

        Ok(true)
    }

    fn evaluate_local_prepare_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        atom: &TransactionAtom,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        let Some(tx_rec) = self
            .transaction_pool
            .get(tx, block.as_leaf_block(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(false);
        };

        if !tx_rec.current_stage().is_prepared() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ LocalPrepare Stage disagreement in block {} for transaction {}. Leader proposed LocalPrepare, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.transaction_id(),
                tx_rec.current_stage()
            );
            return Ok(false);
        }
        // We check that the leader decision is the same as our local decision.
        // We disregard the remote decision because not all validators may have received the foreign
        // LocalPrepared yet. We will never accept a decision disagreement for the Accept command.
        if tx_rec.current_local_decision() != atom.decision {
            warn!(
                target: LOG_TARGET,
                "❌ LocalPrepared decision disagreement for transaction {} in block {}. Leader proposed {}, we decided {}",
                tx_rec.transaction_id(),
                block,
                atom.decision,
                tx_rec.current_local_decision()
            );
            return Ok(false);
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ LocalPrepared transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.transaction_id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(false);
        }

        proposed_block_change_set.set_next_transaction_update(
            &tx_rec,
            TransactionPoolStage::LocalPrepared,
            tx_rec.evidence().all_input_addresses_justified(),
        )?;

        Ok(true)
    }

    fn evaluate_all_prepare_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        atom: &TransactionAtom,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        if atom.decision.is_abort() {
            warn!(
                target: LOG_TARGET,
                "❌ AllPrepare command received for block {} but requires that the transaction is COMMIT",
                block.id(),
            );
            return Ok(false);
        }

        let Some(tx_rec) = self
            .transaction_pool
            .get(tx, block.as_leaf_block(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(false);
        };

        if !tx_rec.current_stage().is_local_prepared() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ Stage disagreement in block {} for transaction {}. Leader proposed AllPrepare, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.transaction_id(),
                tx_rec.current_stage()
            );
            return Ok(false);
        }

        if tx_rec.current_decision().is_abort() {
            warn!(
                target: LOG_TARGET,
                "❌ AllPrepare decision disagreement for transaction {} in block {}. Leader proposed COMMIT, we decided ABORT",
                tx_rec.transaction_id(),
                block,
            );
            return Ok(false);
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ AllPrepare transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.transaction_id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(false);
        }
        // TODO: there is a race condition between the local node receiving the foreign LocalPrepare and the leader
        // proposing AllPrepare. If the latter comes first (because the leader received the foreign LocalPrepare),
        // this node will not vote on this block which leads inevitably to erroneous leader failures. For this reason,
        // this is commented out for now.

        // if !tx_rec.evidence().all_input_addresses_justified() {
        //     warn!(
        //         target: LOG_TARGET,
        //         "❌ LocalPrepare disagreement for transaction {} in block {}. Leader proposed that all committees
        // have justified, but local evidence is not all justified",         tx_rec.transaction_id(),
        //         block,
        //     );
        //     return Ok(false);
        // }
        //
        proposed_block_change_set.set_next_transaction_update(&tx_rec, TransactionPoolStage::AllPrepared, true)?;

        Ok(true)
    }

    fn evaluate_some_prepare_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        atom: &TransactionAtom,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        if atom.decision.is_commit() {
            warn!(
                target: LOG_TARGET,
                "❌ SomePrepare command received for block {} but requires that the transaction is ABORT",
                block.id(),
            );
            return Ok(false);
        }

        let Some(tx_rec) = self
            .transaction_pool
            .get(tx, block.as_leaf_block(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(false);
        };

        if tx_rec.current_decision().is_commit() {
            warn!(
                target: LOG_TARGET,
                "❌ SomePrepare decision disagreement for transaction {} in block {}. Leader proposed ABORT, we decided COMMIT",
                tx_rec.transaction_id(),
                block,
            );
            return Ok(false);
        }

        if !tx_rec.current_stage().is_local_prepared() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ Stage disagreement in block {} for transaction {}. Leader proposed SomePrepare, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.transaction_id(),
                tx_rec.current_stage()
            );
            return Ok(false);
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ SomePrepare transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.transaction_id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(false);
        }

        proposed_block_change_set.set_next_transaction_update(&tx_rec, TransactionPoolStage::SomePrepared, true)?;

        Ok(true)
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate_local_accept_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        let Some(mut tx_rec) = self
            .transaction_pool
            .get(tx, block.as_leaf_block(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(false);
        };

        if !tx_rec.current_stage().is_all_prepared() && !tx_rec.current_stage().is_some_prepared() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ Stage disagreement in block {} for transaction {}. Leader proposed LocalAccept, but current stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.transaction_id(),
                tx_rec.current_stage()
            );
            return Ok(false);
        }

        let maybe_execution = if tx_rec.current_decision().is_commit() {
            let execution = self.execute_transaction(tx, block.id(), block.epoch(), tx_rec.transaction_id())?;
            let execution = execution.into_transaction_execution();

            // TODO: can we modify the locks at this point? For multi-shard input transactions, we locked all inputs as
            // Write due to lack of information. We now know what locks are necessary, however this changes pledges
            // already sent.
            tx_rec.update_from_execution(&execution);

            // Lock all local outputs
            let local_outputs = execution.resulting_outputs().iter().filter(|o| {
                o.substate_id().is_transaction_receipt() ||
                    local_committee_info.includes_substate_address(&o.to_substate_address())
            });
            match substate_store.try_lock_all(*tx_rec.transaction_id(), local_outputs, false) {
                Ok(()) => {},
                Err(err) => {
                    let err = err.or_fatal_error()?;

                    if atom.decision.is_commit() {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ NO VOTE LocalAccept: Lock failure: {} but leader decided COMMIT for tx {} in block {}. Leader proposed COMMIT, we decided ABORT",
                            err,
                            tx_rec.transaction_id(),
                            block,
                        );
                        return Ok(false);
                    }

                    info!(
                        target: LOG_TARGET,
                        "⚠️ Failed to lock outputs for transaction {} in block {}. Error: {}",
                        block,
                        tx_rec.transaction_id(),
                        err
                    );

                    tx_rec.set_local_decision(Decision::Abort);
                    proposed_block_change_set
                        .set_next_transaction_update(
                            &tx_rec,
                            TransactionPoolStage::LocalAccepted,
                            tx_rec.evidence().all_addresses_justified(),
                        )?
                        .add_transaction_execution(execution)?;

                    return Ok(true);
                },
            }
            Some(execution)
        } else {
            // If we already locally decided to abort, there is no purpose in executing the transaction
            None
        };

        // We check that the leader decision is the same as our local decision.
        // We disregard the remote decision because not all validators may have received the foreign
        // LocalPrepared yet. We will never accept a decision disagreement for the Accept command.
        if tx_rec.current_decision() != atom.decision {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE LocalAccept: decision disagreement for transaction {} in block {}. Leader proposed {}, we decided {}",
                tx_rec.transaction_id(),
                block,
                atom.decision,
                tx_rec.current_decision()
            );
            return Ok(false);
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE LocalAccept: transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.transaction_id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(false);
        }

        // maybe_execution is only None if the transaction is not committed
        if let Some(execution) = maybe_execution {
            proposed_block_change_set.add_transaction_execution(execution)?;
        }

        proposed_block_change_set.set_next_transaction_update(
            &tx_rec,
            TransactionPoolStage::LocalAccepted,
            tx_rec.evidence().all_addresses_justified(),
        )?;

        Ok(true)
    }

    fn evaluate_all_accept_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        if atom.decision.is_abort() {
            warn!(
                target: LOG_TARGET,
                "❌ AllAccept command received for block {} but requires that the transaction is COMMIT",
                block.id(),
            );
            return Ok(false);
        }

        let Some(tx_rec) = self
            .transaction_pool
            .get(tx, block.as_leaf_block(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(false);
        };

        if !tx_rec.current_stage().is_local_accepted() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ AllAccept Stage disagreement in block {} for transaction {}. Leader proposed AllAccept, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.transaction_id(),
                tx_rec.current_stage()
            );
            return Ok(false);
        }

        if tx_rec.current_decision().is_abort() {
            warn!(
                target: LOG_TARGET,
                "❌ AllAccept decision disagreement for transaction {} in block {}. Leader proposed COMMIT, we decided ABORT",
                tx_rec.transaction_id(),
                block,
            );
            return Ok(false);
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ AllAccept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.transaction_id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(false);
        }

        let execution = BlockTransactionExecution::get_pending_for_block(tx, tx_rec.transaction_id(), block.parent())
            .optional()?
            .ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "evaluate_all_accept_command: Transaction {} has COMMIT decision but execution is missing",
                    tx_rec.transaction_id()
                ))
            })?;

        let diff = execution.result().finalize.accept().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "evaluate_local_accept_command: Transaction {} has COMMIT decision but execution failed when proposing",
                tx_rec.transaction_id(),
            ))
        })?;

        substate_store.put_diff(
            *tx_rec.transaction_id(),
            &filter_diff_for_committee(local_committee_info, diff),
        )?;

        proposed_block_change_set.set_next_transaction_update(&tx_rec, TransactionPoolStage::AllAccepted, false)?;

        Ok(true)
    }

    fn evaluate_some_accept_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block: &Block,
        atom: &TransactionAtom,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        if atom.decision.is_commit() {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept command received for block {} but requires that the atom is ABORT",
                block.id(),
            );
            return Ok(false);
        }

        let Some(tx_rec) = self
            .transaction_pool
            .get(tx, block.as_leaf_block(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(false);
        };

        if !tx_rec.current_stage().is_local_accepted() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ Stage disagreement in block {} for transaction {}. Leader proposed SomeAccept, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.transaction_id(),
                tx_rec.current_stage()
            );
            return Ok(false);
        }

        // We check that the leader decision is the same as our local decision (this will change to ABORT once we've
        // received the foreign LocalAccept).
        if tx_rec.current_decision().is_commit() {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept decision disagreement for transaction {} in block {}. Leader proposed ABORT, we decided COMMIT",
                tx_rec.transaction_id(),
                block,
            );
            return Ok(false);
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.transaction_id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(false);
        }

        // Check our previous local decision
        if tx_rec.current_local_decision().is_commit() {
            // CASE: We originally decided to commit the transaction, however the remote committee decided to abort.
            //       We may need to create a TransactionExecution for this.
            let mut transaction = tx_rec.get_transaction(tx)?;
            if transaction.abort_reason().is_none() {
                info!(
                    target: LOG_TARGET,
                    "⚠️ ForeignShardGroupDecidedToAbort for transaction {} in block {}",
                    block,
                    tx_rec.transaction_id(),
                );

                // TODO: consider putting the reason in the block so that all shards can report the same reason
                transaction.set_abort_reason(RejectReason::ForeignShardGroupDecidedToAbort(format!(
                    "Transaction {} was rejected by the foreign committee",
                    tx_rec.transaction_id()
                )));
                let execution = transaction
                    .into_execution()
                    .expect("set_abort_reason will always result in an execution");
                proposed_block_change_set.add_transaction_execution(execution)?;
            }
        }

        proposed_block_change_set.set_next_transaction_update(&tx_rec, TransactionPoolStage::SomeAccepted, false)?;

        Ok(true)
    }

    fn evaluate_foreign_proposal_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        fp_atom: &ForeignProposalAtom,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        if !fp_atom.exists(tx)? {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Foreign proposal for block {block_id} has not been received.",
                block_id = fp_atom.block_id,
            );
            return Ok(false);
        }

        proposed_block_change_set.set_foreign_proposal_proposed_in(fp_atom.block_id);

        Ok(true)
    }

    fn evaluate_mint_confidential_output_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        atom: &MintConfidentialOutputAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<bool, HotStuffError> {
        let Some(utxo) = atom.get(tx).optional()? else {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: MintConfidentialOutputAtom for {} is not known.",
                atom.substate_id
            );
            return Ok(false);
        };
        let id = VersionedSubstateId::new(utxo.substate_id.clone(), 0);
        let shard = id.to_substate_address().to_shard(local_committee_info.num_preshards());
        let change = SubstateChange::Up {
            id,
            shard,
            // N/A
            transaction_id: Default::default(),
            substate: Substate::new(0, utxo.substate_value),
        };

        if let Err(err) = substate_store.put(change) {
            let err = err.or_fatal_error()?;
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Failed to store mint confidential output for {}. Error: {}",
                atom.substate_id,
                err
            );
            return Ok(false);
        }

        proposed_block_change_set.set_utxo_mint_proposed_in(utxo.substate_id);

        Ok(true)
    }

    fn execute_transaction(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        block_id: &BlockId,
        current_epoch: Epoch,
        transaction_id: &TransactionId,
    ) -> Result<BlockTransactionExecution, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "👨‍🔧 DECIDE: Executing transaction {} in block {}",
            transaction_id,
            block_id,
        );
        let transaction = TransactionRecord::get(tx, transaction_id)?;
        // Might have been executed already in on propose
        if let Some(execution) =
            BlockTransactionExecution::get_pending_for_block(tx, transaction_id, block_id).optional()?
        {
            return Ok(execution);
        }

        let pledged = PledgedTransaction::load_pledges(tx, transaction)?;

        let executed = self
            .transaction_manager
            .execute(current_epoch, pledged)
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        Ok(executed.into_execution().for_block(*block_id))
    }

    fn on_commit(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        last_executed: &LastExecuted,
        block: &Block,
        local_committee_info: &CommitteeInfo,
    ) -> Result<Vec<TransactionPoolRecord>, HotStuffError> {
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

        // Release all locks for SomePrepare transactions since these can never be committed
        SubstateRecord::unlock_all(tx, block.all_some_prepare().map(|t| &t.id).peekable())?;

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
    ) -> Result<Vec<TransactionPoolRecord>, HotStuffError> {
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

        for atom in block.all_foreign_proposals() {
            atom.delete(tx)?;
        }

        for atom in block.all_confidential_output_mints() {
            atom.delete(tx)?;
        }

        // NOTE: this must happen before we commit the substate diff because the state transitions use this version
        let pending = PendingShardStateTreeDiff::remove_by_block(tx, block.id())?;
        let mut state_tree = ShardedStateTree::new(tx);
        state_tree.commit_diffs(pending)?;
        let tx = state_tree.into_transaction();

        let local_diff = diff.into_filtered(local_committee_info);
        block.commit_diff(tx, local_diff)?;

        let finalized_transactions = self
            .transaction_pool
            .remove_all(tx, block.all_finalising_transactions_ids())?;

        if !finalized_transactions.is_empty() {
            // Remove locks for finalized transactions
            SubstateRecord::unlock_all(tx, finalized_transactions.iter().map(|t| t.transaction_id()).peekable())?;
            TransactionRecord::finalize_all(tx, *block.id(), &finalized_transactions)?;

            debug!(
                target: LOG_TARGET,
                "✅ {} transactions finalized",
                finalized_transactions.len(),
            );
        }

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
