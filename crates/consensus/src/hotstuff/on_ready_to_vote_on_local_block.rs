//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::num::NonZeroU64;

use log::*;
use tari_common_types::types::FixedHash;
use tari_consensus_types::{Decision, LastVoted, LeafBlock, PcId};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_engine_types::commit_result::{AbortReason, RejectReason};
use tari_ootle_common_types::{ShardGroup, committee::CommitteeInfo, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{
        Block,
        BlockDiff,
        BlockTransactionExecution,
        BookkeepingModel,
        Command,
        ForeignProposalAtom,
        ForeignProposalStatus,
        InvalidEvidenceReason,
        LockedEpoch,
        NoVoteReason,
        PendingShardStateTreeDiff,
        SubstateRecord,
        TransactionAtom,
        TransactionExecution,
        TransactionPool,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
        ValidBlock,
        ValidatorStatsUpdate,
    },
};
use tari_sidechain::QuorumDecision;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::sync::broadcast;

use crate::{
    hotstuff::{
        HotstuffConfig,
        ProposalValidationError,
        apply_leader_fee_to_substate_store,
        block_change_set::{BlockDecision, ProposedBlockChangeSet},
        calculate_state_merkle_root,
        error::HotStuffError,
        event::HotstuffEvent,
        filter_diff_for_committee,
        foreign_proposal_processor::process_foreign_block,
        process_newly_justified_block,
        substate_store::{LockStatus, PendingSubstateStore, ShardedStateTree},
        transaction_manager::{
            ConsensusTransactionManager,
            EvidenceOrExecution,
            LocalPreparedTransaction,
            PledgedTransaction,
            PreparedTransaction,
            TransactionLockConflicts,
        },
    },
    tracing::TraceTimer,
    traits::{BlockStore, CertificateStore, ConsensusSpec, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_ready_to_vote_on_local_block";

#[derive(Debug, Clone)]
pub struct OnReadyToVoteOnLocalBlock<TConsensusSpec: ConsensusSpec> {
    local_validator_pk: RistrettoPublicKey,
    config: HotstuffConfig,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_events: broadcast::WeakSender<HotstuffEvent>,
    transaction_manager: ConsensusTransactionManager<TConsensusSpec::TransactionExecutor, TConsensusSpec::StateStore>,
}

impl<TConsensusSpec> OnReadyToVoteOnLocalBlock<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        local_validator_pk: RistrettoPublicKey,
        config: HotstuffConfig,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_events: broadcast::WeakSender<HotstuffEvent>,
        transaction_manager: ConsensusTransactionManager<
            TConsensusSpec::TransactionExecutor,
            TConsensusSpec::StateStore,
        >,
    ) -> Self {
        Self {
            local_validator_pk,
            config,
            transaction_pool,
            tx_events,
            transaction_manager,
        }
    }

    pub fn handle(
        &mut self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        valid_block: &ValidBlock,
        local_committee_info: &CommitteeInfo,
        proposer_claim_public_key_bytes: &RistrettoPublicKeyBytes,
        mut can_propose_epoch_end: bool,
        // The local oracle's view of the next epoch's boundary hash, if it has observed it. Used to
        // ratify the hash carried in an EndEpoch command before voting. `None` means our oracle has
        // not yet crossed the boundary, so we cannot ratify and must abstain.
        expected_next_epoch_hash: Option<FixedHash>,
        change_set: &mut ProposedBlockChangeSet,
    ) -> Result<BlockDecision, HotStuffError> {
        let _timer =
            TraceTimer::info(LOG_TARGET, "Decide on local block").with_iterations(valid_block.block().commands().len());
        debug!(
            target: LOG_TARGET,
            "🔥 LOCAL PROPOSAL READY: {}",
            valid_block,
        );

        let block_qc_id = valid_block.block().justify().calculate_id();
        // The QC is valid, update high QC - Regardless if we accept the proposal commands.
        // Update high TC
        let maybe_high_tc = valid_block
            .block()
            .timeout_certificate()
            .map(|tc| tc.update_highest(tx))
            .transpose()?;

        let mut commit_blocks = Vec::new();
        let mut finalized_transactions = Vec::new();
        // Update nodes
        let high_qc = valid_block.block().update_nodes(
            tx,
            |tx, _prev_locked, block, _justify_qc| self.on_lock_block(tx, block),
            |tx, mut commit_block| {
                let committed = self.on_commit(tx, &block_qc_id, &commit_block)?;
                // NOTE: update the commit QC in the local copy so that foreign proposals can obtain the commit QC
                // on_commit already sets the persisted commit_qc for the block
                commit_block.set_commit_qc(block_qc_id);
                if !commit_block.is_dummy() {
                    commit_blocks.push(commit_block);
                }
                if !committed.is_empty() {
                    finalized_transactions.push(committed);
                }
                Ok(())
            },
        )?;

        // Process newly justified block
        let mut justified_block = Block::get_justified_block(&**tx, valid_block.justify(), valid_block.epoch())?;
        // This comes before decide so that all evidence can be in place before LocalPrepare and LocalAccept
        let processed_blocks =
            process_newly_justified_block(&**tx, &justified_block, block_qc_id, local_committee_info, change_set)?;
        for mut block in processed_blocks {
            block.add_justify_qc(tx, &block_qc_id)?;
        }
        justified_block.add_justify_qc(tx, &block_qc_id)?;
        // Even if we do not yet see the next epoch (e.g. race condition), if a majority have, we allow the
        // epoch end to be proposed.
        can_propose_epoch_end |= justified_block.is_epoch_end();

        if self.should_vote(&**tx, valid_block.block())? {
            let parent = valid_block.block().get_parent(&**tx)?;

            self.decide_what_to_vote(
                &**tx,
                &parent,
                valid_block.block(),
                local_committee_info,
                proposer_claim_public_key_bytes,
                can_propose_epoch_end,
                expected_next_epoch_hash,
                change_set,
            )?;
        } else {
            change_set.set_no_vote(NoVoteReason::AlreadyVotedAtHeight);
        }

        let quorum_decision = change_set.quorum_decision();
        if change_set.is_accept() {
            info!(
                target: LOG_TARGET,
                "✅ Saving changeset: {}",
                change_set
            );
            change_set.save(tx)?;
        } else {
            warn!(
                target: LOG_TARGET,
                "❌ NOT voting on block {}. Change set: {}",
                valid_block.block(),
                change_set,
            );
        }

        Ok(BlockDecision {
            local_decision: quorum_decision,
            commit_blocks,
            finalized_transactions,
            high_pc: high_qc,
            new_high_tc: maybe_high_tc,
            no_vote_reason: change_set.no_vote_reason().cloned(),
        })
    }

    /// if b_new .height > vheight && (b_new extends b_lock || b_new .justify.node.height > b_lock .height)
    ///
    /// If we have not previously voted on this block and the node extends the current locked node, then we vote
    fn should_vote<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
    ) -> Result<bool, ProposalValidationError> {
        let Some(last_voted) = LastVoted::get(tx, block.epoch()).optional()? else {
            // Never voted, then validated.block.height() > last_voted.height (0)
            return Ok(true);
        };

        // if b_new .height > vheight And ...
        if block.height() <= last_voted.height() {
            info!(
                target: LOG_TARGET,
                "❌ NOT voting on block {}. Block height is not greater than last voted height {}",
                block,
                last_voted.height(),
            );
            return Ok(false);
        }

        Ok(true)
    }

    #[allow(clippy::too_many_lines)]
    fn decide_what_to_vote<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        parent: &Block,
        block: &Block,
        local_committee_info: &CommitteeInfo,
        proposer_claim_public_key_bytes: &RistrettoPublicKeyBytes,
        can_propose_epoch_end: bool,
        expected_next_epoch_hash: Option<FixedHash>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<(), HotStuffError> {
        // Reject (no-vote) a block whose total transaction execution weight exceeds the network cap,
        // before executing any of its commands. This bounds how long a replica can be made to spend
        // executing a single block, so a misbehaving leader cannot push replicas past the block time by
        // packing more than the propose-time budget. A block with a single transaction command is exempt
        // so an individually-heavy transaction stays committable (its static weight also over-estimates
        // its execution). CONSENSUS RULE: `max_block_validation_weight` must be uniform network-wide.
        let max_validation_weight = self.config.consensus_constants.max_block_validation_weight;
        let mut block_execution_weight = 0u64;
        let mut num_transaction_commands = 0usize;
        for cmd in block.commands() {
            let Some(atom) = cmd.transaction() else {
                continue;
            };
            num_transaction_commands += 1;
            let weight = atom
                .get_transaction(tx)?
                .transaction()
                .calculate_transaction_weight()
                .as_u64();
            block_execution_weight =
                block_execution_weight.saturating_add(weight.saturating_mul(cmd.execution_weight_percent()) / 100);
        }
        if exceeds_block_validation_weight(block_execution_weight, num_transaction_commands, max_validation_weight) {
            let reason = NoVoteReason::BlockWeightExceeded {
                total_weight: block_execution_weight,
                max_weight: max_validation_weight,
            };
            warn!(target: LOG_TARGET, "❌ NO VOTE: {reason}");
            proposed_block_change_set.set_no_vote(reason);
            return Ok(());
        }

        // Store used for transactions that have inputs without specific versions.
        // It lives through the entire block so multiple transactions can be sequenced together in the same block
        let mut substate_store =
            PendingSubstateStore::new(tx, block.as_leaf(), self.config.consensus_constants.num_preshards);
        let mut total_leader_fee = 0;
        let mut total_exhaust_burn = parent.header().total_accumulated_exhaust_burn();
        let max_validation_execution_points = self.config.consensus_constants.max_block_validation_execution_points;
        let mut block_execution_points = 0u64;

        for cmd in block.commands() {
            match cmd {
                Command::LocalOnly(atom) => {
                    if let Some(reason) = self.evaluate_local_only_command(
                        tx,
                        block,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        proposed_block_change_set,
                        &mut total_leader_fee,
                        &mut total_exhaust_burn,
                    )? {
                        proposed_block_change_set.set_no_vote(reason);
                        return Ok(());
                    }
                },
                Command::LocalPrepare(atom) => {
                    if let Some(reason) = self.evaluate_local_prepare_command(
                        tx,
                        block,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        proposed_block_change_set,
                    )? {
                        proposed_block_change_set.set_no_vote(reason);
                        return Ok(());
                    }
                },
                Command::LocalAccept(atom) => {
                    if let Some(reason) = self.evaluate_local_accept_command(
                        tx,
                        block,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        proposed_block_change_set,
                    )? {
                        proposed_block_change_set.set_no_vote(reason);
                        return Ok(());
                    }
                },
                Command::AllAccept(atom) => {
                    if let Some(reason) = self.evaluate_all_accept_command(
                        tx,
                        block,
                        atom,
                        local_committee_info,
                        &mut substate_store,
                        proposed_block_change_set,
                        &mut total_leader_fee,
                        &mut total_exhaust_burn,
                    )? {
                        proposed_block_change_set.set_no_vote(reason);
                        return Ok(());
                    }
                },
                Command::SomeAccept(atom) => {
                    if let Some(reason) =
                        self.evaluate_some_accept_command(tx, block, atom, proposed_block_change_set)?
                    {
                        proposed_block_change_set.set_no_vote(reason);
                        return Ok(());
                    }
                },
                Command::ForeignProposal(fp_atom) => {
                    if let Some(reason) = self.evaluate_foreign_proposal_command(
                        tx,
                        block,
                        fp_atom,
                        local_committee_info,
                        fp_atom.shard_group,
                        &mut substate_store,
                        proposed_block_change_set,
                    )? {
                        proposed_block_change_set.set_no_vote(reason);
                        return Ok(());
                    }

                    continue;
                },
                Command::EndEpoch(atom) => {
                    if !can_propose_epoch_end {
                        warn!(
                            target: LOG_TARGET,
                            "❌ EpochEvent::End command received for block {} but it is not the next epoch",
                            block.id(),
                        );
                        proposed_block_change_set.set_no_vote(NoVoteReason::NotEndOfEpoch);
                        return Ok(());
                    }
                    if block.commands().len() > 1 {
                        warn!(
                            target: LOG_TARGET,
                            "❌ EpochEvent::End command in block {} but block contains other commands",
                            block.id()
                        );
                        proposed_block_change_set.set_no_vote(NoVoteReason::EndOfEpochWithOtherCommands);
                        return Ok(());
                    }

                    // Ratify the next epoch's hash against our own (lagged, reorg-stable) oracle. Voting
                    // for the EOE block is how the committee agrees on the next epoch's boundary hash;
                    // by gating on the LOCAL oracle (not on the majority signal that may have set
                    // can_propose_epoch_end above), a node never lends quorum to a hash it has not
                    // itself observed. If our oracle has not crossed the boundary, abstain until it has.
                    match expected_next_epoch_hash {
                        Some(local) if local == *atom.next_epoch_hash() => {},
                        Some(local) => {
                            warn!(
                                target: LOG_TARGET,
                                "❌ NO VOTE: EndEpoch in block {} proposes next-epoch hash {} but our oracle has {}",
                                block.id(),
                                atom.next_epoch_hash(),
                                local,
                            );
                            proposed_block_change_set.set_no_vote(NoVoteReason::EndOfEpochHashMismatch {
                                local,
                                proposed: *atom.next_epoch_hash(),
                            });
                            return Ok(());
                        },
                        None => {
                            warn!(
                                target: LOG_TARGET,
                                "❌ NO VOTE: EndEpoch in block {} but our oracle has not yet observed the next epoch boundary block",
                                block.id(),
                            );
                            proposed_block_change_set.set_no_vote(NoVoteReason::EndOfEpochHashNotObserved);
                            return Ok(());
                        },
                    }

                    continue;
                },
            }

            // CONSENSUS RULE: a block may not execute more than `max_block_validation_execution_points`. The
            // total covers WASM metering and native crypto verification alike — both are real CPU time on every
            // replica, and bounding only the WASM half would leave stealth and confidential verification
            // ungoverned at the block level. It is accumulated as commands execute and the first command that
            // pushes it over the limit stops block execution (no-vote), bounding the compute a misbehaving
            // leader can extract from replicas. Both halves are deterministic — metering is, and the native
            // price is a pure function of the declared statement — so every replica computes identical totals
            // and stops at the same command. Commands executed for this block have an execution in the change set;
            // finalization commands (AllAccept/SomeAccept) reuse a prior block's execution and add nothing.
            if let Some(atom) = cmd.transaction() &&
                let Some(execution) = proposed_block_change_set.transaction_execution(atom.id())
            {
                block_execution_points =
                    block_execution_points.saturating_add(execution.result().total_execution_points());
                if block_execution_points > max_validation_execution_points {
                    let reason = NoVoteReason::BlockExecutionPointsExceeded {
                        total_points: block_execution_points,
                        max_points: max_validation_execution_points,
                    };
                    warn!(target: LOG_TARGET, "❌ NO VOTE: {reason}");
                    proposed_block_change_set.set_no_vote(reason);
                    return Ok(());
                }
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
            proposed_block_change_set.set_no_vote(NoVoteReason::TotalLeaderFeeDisagreement);
            return Ok(());
        }

        if total_exhaust_burn != block.header().total_accumulated_exhaust_burn() {
            warn!(
                target: LOG_TARGET,
                "❌ Exhaust burn disagreement for block {}. Leader proposed {}, we calculated {}",
                block,
                block.header().total_accumulated_exhaust_burn(),
                total_exhaust_burn
            );
            proposed_block_change_set.set_no_vote(NoVoteReason::TotalExhaustBurnDisagreement);
            return Ok(());
        }

        // Apply leader fee to substate store before we calculate the state root
        if total_leader_fee > 0 {
            apply_leader_fee_to_substate_store(
                &mut substate_store,
                proposer_claim_public_key_bytes,
                local_committee_info.num_preshards(),
                local_committee_info.shard_group().start(),
                total_leader_fee,
            )?;
        }

        let pending = PendingShardStateTreeDiff::get_all_up_to_commit_block(tx, block.parent())?;
        let (expected_merkle_root, tree_diffs) = calculate_state_merkle_root(
            tx,
            block.shard_group(),
            pending,
            substate_store
                .changes()
                .iter()
                // Calculate for local shards only or the global shard
                .filter(|ch| block.shard_group().contains_or_global(&ch.shard())),
            block.network(),
            block.epoch(),
        )?;
        if expected_merkle_root != *block.state_merkle_root() {
            warn!(
                target: LOG_TARGET,
                "❌ State Merkle root disagreement for block {}. Leader proposed {}, we calculated {}",
                block,
                block.state_merkle_root(),
                expected_merkle_root
            );
            let (diff, locks) = substate_store.into_parts();
            let diff = BlockDiff::new(*block.id(), diff);
            proposed_block_change_set
                .set_no_vote(NoVoteReason::StateMerkleRootMismatch)
                // These are set for debugging purposes but aren't actually committed
                .set_block_diff_to_commit(diff.into_filtered(local_committee_info.shard_group()))
                .set_substate_locks(locks);
            return Ok(());
        }

        let (diff, locks) = substate_store.into_parts();
        let diff = BlockDiff::new(*block.id(), diff);
        proposed_block_change_set
            .set_block_diff_to_commit(diff)
            .set_state_tree_diffs(tree_diffs)
            .set_substate_locks(locks)
            .set_quorum_decision(QuorumDecision::Accept);

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate_local_only_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
        total_leader_fee: &mut u64,
        total_exhaust_burn: &mut u128,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        let _timer = TraceTimer::info(LOG_TARGET, "Evaluate LocalOnly command");
        let Some(mut pool_tx) = proposed_block_change_set
            .get_transaction_pool_record(tx, &block.as_leaf(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if !pool_tx.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "❌ Stage disagreement for tx {} in block {}. Leader proposed LocalOnly, local stage is {}",
                pool_tx.id(),
                block,
                pool_tx.current_stage(),
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                stage: pool_tx.current_stage(),
                expected: TransactionPoolStage::New,
            }));
        }
        let locked_epoch = block.to_locked_epoch();
        // LocalOnly so we can lock the epoch here
        pool_tx.update_locked_epoch(locked_epoch.clone());

        let prepared = self
            .transaction_manager
            .prepare(
                substate_store,
                local_committee_info,
                &pool_tx,
                block.as_leaf(),
                proposed_block_change_set,
            )
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        match prepared {
            PreparedTransaction::LocalOnly(local) => {
                match *local {
                    LocalPreparedTransaction::Accept { execution, .. } => {
                        pool_tx
                            .set_local_decision(execution.decision())
                            .set_transaction_fee(execution.transaction_fee())
                            .set_exhaust_burn(execution.exhaust_burn())
                            .set_evidence(execution.to_evidence(
                                local_committee_info.num_preshards(),
                                local_committee_info.num_committees(),
                            ));

                        info!(
                            target: LOG_TARGET,
                            "👨‍🔧 LocalOnly: Prepare for transaction {} ({}) in block {}",
                            pool_tx.id(),
                            pool_tx.current_decision(),
                            block,
                        );

                        // If the leader proposed to commit a transaction that we want to abort, we abstain from voting
                        if pool_tx.current_decision() != atom.decision {
                            // If we disagree with any local decision we abstain from voting
                            warn!(
                                target: LOG_TARGET,
                                "❌ Prepare decision disagreement for tx {} in block {}. Leader proposed {}, we decided {}",
                                pool_tx.id(),
                                block,
                                atom.decision,
                                pool_tx.current_decision()
                            );
                            return Ok(Some(NoVoteReason::DecisionDisagreement {
                                local: pool_tx.current_decision(),
                                remote: atom.decision,
                            }));
                        }

                        if pool_tx.transaction_fee() != atom.transaction_fee {
                            warn!(
                                target: LOG_TARGET,
                                "❌ LocalOnly transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                                block,
                                atom.transaction_fee,
                                pool_tx.transaction_fee()
                            );
                            return Ok(Some(NoVoteReason::FeeDisagreement));
                        }

                        if pool_tx.current_decision().is_commit() {
                            if let Some(diff) = execution.result().finalize.any_accept() {
                                substate_store.put_diff(diff)?;
                            }

                            if atom.leader_fee.is_none() {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Leader fee for tx {} is None for LocalOnly command in block {}",
                                    atom.id,
                                    block,
                                );
                                return Ok(Some(NoVoteReason::NoLeaderFee));
                            }

                            let calculated_leader_fee =
                                pool_tx.calculate_leader_fee(NonZeroU64::new(1).expect("1 > 0"));
                            if calculated_leader_fee != *atom.leader_fee.as_ref().expect("None already checked") {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ LocalOnly leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                                    block,
                                    atom.leader_fee.as_ref().expect("None already checked"),
                                    calculated_leader_fee
                                );

                                return Ok(Some(NoVoteReason::LeaderFeeDisagreement));
                            }

                            *total_leader_fee += calculated_leader_fee.fee();
                            // A LocalOnly transaction's evidence must contain exactly the local shard group, so its
                            // portion of the exhaust burn is the entire burn.
                            if pool_tx.evidence().num_shard_groups() != 1 {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ NO VOTE: LocalOnly transaction {} in block {} has evidence for {} shard groups",
                                    pool_tx.id(),
                                    block,
                                    pool_tx.evidence().num_shard_groups(),
                                );
                                return Ok(Some(NoVoteReason::LocalOnlyProposedForMultiShard));
                            }
                            let Some(exhaust_burn_portion) = pool_tx.evidence().exhaust_burn_portion(
                                calculated_leader_fee.exhaust_burn(),
                                local_committee_info.shard_group(),
                            ) else {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ NO VOTE: local shard group {} is not in the evidence for LocalOnly transaction {} in block {}",
                                    local_committee_info.shard_group(),
                                    pool_tx.id(),
                                    block,
                                );
                                return Ok(Some(NoVoteReason::InvalidEvidence {
                                    reason: InvalidEvidenceReason::MissingInvolvedShardGroup {
                                        shard_group: local_committee_info.shard_group(),
                                    },
                                }));
                            };
                            *total_exhaust_burn += u128::from(exhaust_burn_portion);
                        }

                        proposed_block_change_set.add_transaction_execution(*pool_tx.id(), execution)?;
                    },
                    LocalPreparedTransaction::EarlyAbort { execution, .. } => {
                        if atom.decision.is_commit() {
                            warn!(
                                target: LOG_TARGET,
                                "❌ Failed to lock inputs/outputs for transaction {} but leader proposed COMMIT. Not voting for block {}",
                                pool_tx.id(),
                                block,
                            );
                            return Ok(Some(NoVoteReason::DecisionDisagreement {
                                local: Decision::Abort(AbortReason::LockInputsOutputsFailed),
                                remote: Decision::Commit,
                            }));
                        }

                        // They want to ABORT a successfully executed transaction because of a lock conflict, which
                        // we also have.
                        info!(
                            target: LOG_TARGET,
                            "⚠️ Proposer chose to ABORT and we chose to ABORT due to lock conflict for transaction {} in block {}",
                            block,
                            pool_tx.id(),
                        );
                        pool_tx
                            .set_local_decision(execution.decision())
                            .set_transaction_fee(execution.transaction_fee())
                            .set_exhaust_burn(execution.exhaust_burn())
                            .set_evidence(execution.to_evidence(
                                local_committee_info.num_preshards(),
                                local_committee_info.num_committees(),
                            ));
                        proposed_block_change_set.add_transaction_execution(*pool_tx.id(), execution)?;
                    },
                }
            },
            PreparedTransaction::MultiShard(_) => {
                warn!(
                    target: LOG_TARGET,
                    "❌ transaction {} in block {} is not Local-Only but was proposed as LocalOnly",
                    atom.id(),
                    block,
                );
                return Ok(Some(NoVoteReason::LocalOnlyProposedForMultiShard));
            },
        }

        pool_tx.set_next_stage_and_readiness(TransactionPoolStage::LocalOnly, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(pool_tx)?;
        Ok(None)
    }

    #[allow(clippy::too_many_lines)]
    fn initial_prepare_multishard<TTx: StateStoreReadTransaction>(
        &self,
        tx_rec: &mut TransactionPoolRecord,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        let _timer = TraceTimer::info(LOG_TARGET, "Evaluate Prepare command");

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Transaction {} in block {}",
            tx_rec.id(),
            block,
        );

        if !tx_rec.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "❌ Stage disagreement for tx {} in block {}. Leader proposed Prepare, local stage is {}",
                tx_rec.id(),
                block,
                tx_rec.current_stage(),
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                stage: tx_rec.current_stage(),
                expected: TransactionPoolStage::New,
            }));
        }

        // Prepare phase, ensure we set a locked_epoch if not already done
        tx_rec.update_locked_epoch(block.to_locked_epoch());

        // Foreign block could have already resulted in an ABORT execution
        let maybe_execution = proposed_block_change_set.take_transaction_execution(tx_rec.id());
        let prepared = if maybe_execution.as_ref().is_some_and(|e| e.decision().is_abort()) {
            let execution = maybe_execution.expect("is_some_and");
            info!(
                target: LOG_TARGET,
                "👨‍🔧 PREPARE: Transaction {} in block {} is already ABORTED by foreign block",
                tx_rec.id(),
                execution.block_id()
            );
            PreparedTransaction::new_multishard_executed(execution.into_transaction_execution(), LockStatus::new())
        } else {
            self.transaction_manager
                .prepare(
                    substate_store,
                    local_committee_info,
                    tx_rec,
                    block.as_leaf(),
                    proposed_block_change_set,
                )
                .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?
        };

        match prepared {
            PreparedTransaction::LocalOnly(_) => {
                warn!(
                    target: LOG_TARGET,
                    "❌ transaction {} in block {} is Local-Only but was proposed as Prepare",
                    atom.id(),
                    block,
                );
                return Ok(Some(NoVoteReason::MultiShardProposedForLocalOnly));
            },
            PreparedTransaction::MultiShard(multishard) => {
                // TODO: Because on_propose does not process foreign proposals before proposing, the decision abort
                // reason may mismatch (e.g. ExecutionFailure != ForeignShardGroupDecidedToAbort)
                // This is why we use is_same_outcome here. We should try to process foreign proposals in
                // on_propose
                if !multishard.current_decision().is_same_outcome(atom.decision) {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Leader proposed {} for transaction {} but we decided {} in block {}",
                        atom.decision,
                        atom.id,
                        multishard.current_decision(),
                        block,
                    );
                    return Ok(Some(NoVoteReason::DecisionDisagreement {
                        local: multishard.current_decision(),
                        remote: atom.decision,
                    }));
                }

                // TODO: this is kinda hacky - we may not be involved in the transaction after ABORT execution,
                // but this would be invalid so we ensure that we are added to evidence. Ideally, we wouldn't
                // sequence this transaction at all - investigate.
                tx_rec
                    .evidence_mut()
                    .add_shard_group(local_committee_info.shard_group());

                match multishard.into_evidence_or_execution() {
                    EvidenceOrExecution::Execution { execution } => {
                        debug!(
                            target: LOG_TARGET,
                            "👨‍🔧 PREPARE: Transaction {} in block {} is executed",
                            tx_rec.id(),
                            block,
                        );
                        // CASE: All inputs are local and outputs are foreign (i.e. the transaction is
                        // executed), or we're output-only and have received all pledges.
                        tx_rec.update_from_execution(
                            local_committee_info.num_preshards(),
                            local_committee_info.num_committees(),
                            &execution,
                        );
                        if execution.decision().is_commit() {
                            let involves_inputs = tx_rec.evidence().has_inputs(local_committee_info.shard_group());
                            if !involves_inputs {
                                // Output only
                                let num_involved_shard_groups = tx_rec.evidence().num_shard_groups();
                                let involved = NonZeroU64::new(num_involved_shard_groups as u64).ok_or_else(|| {
                                    HotStuffError::InvariantError("Number of involved shard groups is 0".to_string())
                                })?;
                                let leader_fee = tx_rec.calculate_leader_fee(involved);
                                tx_rec.set_leader_fee(leader_fee);
                            }
                        }
                        proposed_block_change_set.add_transaction_execution(*tx_rec.id(), *execution)?;
                    },
                    EvidenceOrExecution::Evidence { evidence } => {
                        debug!(
                            target: LOG_TARGET,
                            "👨‍🔧 PREPARE: Transaction {} in block {} is not executed. Using partial evidence.",
                            tx_rec.id(),
                            block,
                        );
                        // CASE: All local inputs were resolved. We need to continue with consensus to get the
                        // foreign inputs/outputs.
                        tx_rec.set_local_decision(Decision::Commit);
                        // Set partial evidence for local inputs using what we know.
                        tx_rec.merge_evidence(evidence);
                        // tx_rec epoch will be locked by foreign proposal(s) when we get it
                    },
                }
            },
        }

        Ok(None)
    }

    fn evaluate_local_prepare_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        let Some(mut tx_rec) = proposed_block_change_set
            .get_transaction_pool_record(tx, &block.as_leaf(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if !tx_rec.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ LocalPrepare Stage disagreement in block {} for transaction {}. Leader proposed LocalPrepare, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.id(),
                tx_rec.current_stage()
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                expected: TransactionPoolStage::New,
                stage: tx_rec.current_stage(),
            }));
        }

        if let Some(reason) = self.initial_prepare_multishard(
            &mut tx_rec,
            block,
            atom,
            local_committee_info,
            substate_store,
            proposed_block_change_set,
        )? {
            return Ok(Some(reason));
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ LocalPrepared transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(Some(NoVoteReason::FeeDisagreement));
        }

        if atom.evidence.get(&local_committee_info.shard_group()) !=
            tx_rec.evidence().get(&local_committee_info.shard_group())
        {
            warn!(
                target: LOG_TARGET,
                "❌ LocalPrepared evidence disagreement tx {} in block {}. Leader proposed {}, local {}",
                tx_rec.id(),
                block,
                atom.evidence,
                tx_rec.evidence()
            );
            return Ok(Some(NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MismatchedEvidence,
            }));
        }

        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(tx_rec)?;

        Ok(None)
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate_local_accept_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        let Some(mut tx_rec) = proposed_block_change_set
            .get_transaction_pool_record(tx, &block.as_leaf(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if tx_rec.current_stage().is_new() {
            // CASE: This was sequenced immediately as LocalAccept, which can only mean either we are aborting or we are
            // an output-only Shard Group
            if let Some(reason) = self.initial_prepare_multishard(
                &mut tx_rec,
                block,
                atom,
                local_committee_info,
                substate_store,
                proposed_block_change_set,
            )? {
                return Ok(Some(reason));
            }
            if !tx_rec.current_decision().is_abort() &&
                !tx_rec
                    .evidence()
                    .is_committee_output_only(local_committee_info.shard_group())
            {
                warn!(
                    target: LOG_TARGET,
                    "❌ LocalAccept: transaction {} in block {} is not output-only",
                    tx_rec.id(),
                    block,
                );
                return Ok(Some(NoVoteReason::OutputOnlyDisagreement {
                    transaction_id: *tx_rec.id(),
                    shard_group: local_committee_info.shard_group(),
                    command: "LocalAccept",
                    stage: tx_rec.current_stage(),
                }));
            }
        } else if tx_rec.current_decision().is_commit() {
            // CASE: We are in the LocalPrepared stage, and want to proceed to LocalAccept

            if !tx_rec.current_stage().is_local_prepared() {
                warn!(
                    target: LOG_TARGET,
                    "❌ LocalAccept: transaction {} in block {} is not in COMMITTED LocalPrepared stage (committed stage: {}, current stage: {})",
                    tx_rec.id(),
                    block,
                    tx_rec.committed_stage(),
                    tx_rec.current_stage(),
                );
                return Ok(Some(NoVoteReason::StageDisagreement {
                    expected: TransactionPoolStage::LocalPrepared,
                    stage: tx_rec.current_stage(),
                }));
            }

            let transaction = tx_rec.get_transaction(tx)?;
            if !transaction.has_all_required_input_pledges(tx, local_committee_info)? {
                warn!(
                    target: LOG_TARGET,
                    "❌ NO VOTE AllPrepare: transaction {} in block {} has not received all foreign input pledges",
                    tx_rec.id(),
                    block,
                );
                return Ok(Some(NoVoteReason::NotAllForeignInputPledges));
            }
            let transaction_id = *tx_rec.id();

            let locked_epoch = tx_rec.locked_epoch().ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "Locked epoch not set for transaction {} in LocalAccept stage",
                    tx_rec.id()
                ))
            })?;
            let execution = self.execute_transaction(
                tx,
                block.as_leaf(),
                locked_epoch.clone(),
                transaction,
                proposed_block_change_set,
            )?;
            let execution = execution.into_transaction_execution();

            // TODO: can we modify input locks at this point? For multi-shard input transactions, we locked all inputs
            // as Write due to lack of information. We now know what locks are necessary, and this
            // block has the correct evidence so this should be fine.
            tx_rec.update_from_execution(
                local_committee_info.num_preshards(),
                local_committee_info.num_committees(),
                &execution,
            );

            if execution.decision().is_commit() {
                // Lock all local outputs
                let local_outputs = execution
                    .resulting_outputs()
                    .iter()
                    .filter(|o| local_committee_info.includes_substate_id(o.substate_id()));
                let lock_status = substate_store.try_lock_all(*tx_rec.id(), local_outputs, false)?;
                if let Some(err) = lock_status.failures().first() {
                    if atom.decision.is_commit() {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ NO VOTE LocalAccept: Lock failure: {} but leader decided COMMIT for tx {} in block {}. Leader proposed COMMIT, we decided ABORT",
                            err,
                            tx_rec.id(),
                            block,
                        );
                        return Ok(Some(NoVoteReason::DecisionDisagreement {
                            local: Decision::Abort(AbortReason::LockOutputsFailed),
                            remote: Decision::Commit,
                        }));
                    }

                    info!(
                        target: LOG_TARGET,
                        "⚠️ Failed to lock outputs for transaction {} in block {}. Error: {}",
                        tx_rec.id(),
                        block,
                        err
                    );

                    let execution = TransactionExecution::abort(
                        &transaction_id,
                        RejectReason::FailedToLockOutputs(err.to_string()),
                    );

                    tx_rec
                        .set_local_decision(execution.decision())
                        .set_transaction_fee(0)
                        .set_exhaust_burn(0)
                        .no_leader_fee()
                        .set_next_stage_and_readiness(TransactionPoolStage::LocalAccepted, block.shard_group())?;

                    proposed_block_change_set
                        .add_transaction_execution(*tx_rec.id(), execution)?
                        .set_next_transaction_update(tx_rec)?;

                    return Ok(None);
                }
            }

            info!(
                target: LOG_TARGET,
                "👨‍🔧 LocalAccept: Executed transaction {} in block {} with decision {}",
                tx_rec.id(),
                block,
                execution.decision()
            );
            proposed_block_change_set.add_transaction_execution(*tx_rec.id(), execution)?;
        } else {
            // Abort - nothing to do here
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE LocalAccept: transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(Some(NoVoteReason::FeeDisagreement));
        }

        if tx_rec.current_decision().is_commit() {
            let Some(ref leader_fee) = atom.leader_fee else {
                warn!(
                    target: LOG_TARGET,
                    "❌ NO VOTE: Leader fee in tx {} not set for LocalAccept command in block {}",
                    atom.id,
                    block,
                );
                return Ok(Some(NoVoteReason::NoLeaderFee));
            };

            // Check the leader fee in the local accept phase. The fee only applied (is added to the block fee) for
            // AllAccept
            let num_involved_shard_groups = tx_rec.evidence().num_shard_groups();
            let involved = NonZeroU64::new(num_involved_shard_groups as u64)
                .ok_or_else(|| HotStuffError::InvariantError("Number of involved shard groups is 0".to_string()))?;
            let calculated_leader_fee = tx_rec.calculate_leader_fee(involved);
            if calculated_leader_fee != *leader_fee {
                warn!(
                    target: LOG_TARGET,
                    "❌ NO VOTE: LocalAccept leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                    block,
                    atom.leader_fee.as_ref().expect("None already checked"),
                    calculated_leader_fee
                );

                return Ok(Some(NoVoteReason::LeaderFeeDisagreement));
            }

            tx_rec.set_leader_fee(calculated_leader_fee);
        } else if atom.leader_fee.is_some() {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Leader fee in tx {} is set for LocalAccept ABORT command in block {}",
                atom.id,
                block,
            );
            return Ok(Some(NoVoteReason::LeaderFeeDisagreement));
        } else {
            // Ok
        }

        // on_propose does not process foreign proposals, so the QC evidence may not match the evidence here.
        // We only check that the input/output pledges match
        if !tx_rec.evidence().eq_pledges(&atom.evidence) {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Evidence mismatch for LocalAccept transaction {} in block {}. Leader proposed evidence {}, but we calculated {}",
                tx_rec.id(),
                block,
                atom.evidence,
                tx_rec.evidence()
            );
            return Ok(Some(NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MismatchedEvidence,
            }));
        }

        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::LocalAccepted, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(tx_rec)?;

        Ok(None)
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate_all_accept_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &TransactionAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
        total_leader_fee: &mut u64,
        total_exhaust_burn: &mut u128,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        if atom.decision.is_abort() {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept command received for block {} but requires that the transaction is COMMIT",
                block.id(),
            );
            return Ok(Some(NoVoteReason::AllAcceptMustBeCommit {
                transaction_id: atom.id,
                block_id: *block.id(),
            }));
        }

        let Some(mut tx_rec) = proposed_block_change_set
            .get_transaction_pool_record(tx, &block.as_leaf(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ NO VOTE: Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if !tx_rec.current_stage().is_local_accepted() {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept Stage disagreement in block {} for transaction {}. Leader proposed AllAccept, but local stage is {}",
                block,
                tx_rec.id(),
                tx_rec.current_stage()
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                expected: TransactionPoolStage::LocalAccepted,
                stage: tx_rec.current_stage(),
            }));
        }

        if tx_rec.current_decision().is_abort() {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept decision disagreement for transaction {} in block {}. Leader proposed COMMIT, we decided ABORT",
                tx_rec.id(),
                block,
            );
            return Ok(Some(NoVoteReason::DecisionDisagreement {
                local: atom.decision,
                remote: Decision::Commit,
            }));
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(Some(NoVoteReason::FeeDisagreement));
        }

        let Some(ref leader_fee) = atom.leader_fee else {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Leader fee in tx {} not set for AllAccept command in block {}",
                atom.id,
                block,
            );
            return Ok(Some(NoVoteReason::NoLeaderFee));
        };

        let local_leader_fee = tx_rec.leader_fee().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "evaluate_all_accept_command: Transaction {} has COMMIT decision and is at LocalAccepted stage but \
                 leader fee is missing",
                tx_rec.id()
            ))
        })?;

        if local_leader_fee != leader_fee {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Leader fee disagreement for tx {} in block {}. Leader proposed {}, we calculated {}",
                atom.id,
                block,
                leader_fee,
                local_leader_fee
            );
            return Ok(Some(NoVoteReason::LeaderFeeDisagreement));
        }

        // TODO: investigate, this fails sometimes
        // if !tx_rec.evidence().all_objects_accepted() {
        //     warn!(
        //         target: LOG_TARGET,
        //         "❌ NO VOTE: AllAccept disagreement for transaction {} in block {}. Leader proposed that all shard
        // groups have accepted the atom but locally this is not the case",         tx_rec.transaction_id(),
        //         block,
        //     );
        //     return Ok(Some(NoVoteReason::NotAllInputsOutputsAccepted));
        // }

        if !tx_rec.has_all_required_foreign_pledges(tx, local_committee_info)? {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept disagreement for transaction {} in block {}. Leader proposed that all foreign pledges have been received but locally this is not the case",
                tx_rec.id(),
                block,
            );
            return Ok(Some(NoVoteReason::NotAllForeignInputPledges));
        }

        // TODO: on_propose does not process foreign proposals so we cannot rely on this check
        // if !atom.evidence.all_shard_groups_accepted() {
        //     warn!(
        //         target: LOG_TARGET,
        //         "❌ NO VOTE: AllAccept disagreement for transaction {} in block {}. Leader proposed an atom which did
        // not indicate that all shard groups have accepted the transaction",         tx_rec.transaction_id(),
        //         block,
        //     );
        //     return Ok(Some(NoVoteReason::NotAllInputsOutputsAccepted));
        // }

        if *tx_rec.evidence() != atom.evidence {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept disagreement for transaction {} in block {}. Leader proposed evidence {}, but we calculated {}",
                tx_rec.id(),
                block,
                atom.evidence,
                tx_rec.evidence()
            );
            return Ok(Some(NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MismatchedEvidence,
            }));
        }

        let execution = BlockTransactionExecution::get_pending_for_block(tx, tx_rec.id(), &block.as_leaf())
            .optional()?
            .ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "evaluate_all_accept_command: Transaction {} has COMMIT decision but execution is missing",
                    tx_rec.id()
                ))
            })?;

        let diff = execution.result().finalize.any_accept().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "evaluate_local_accept_command: Transaction {} has COMMIT decision but execution failed when proposing",
                tx_rec.id(),
            ))
        })?;

        *total_leader_fee += leader_fee.fee();
        // Compute the portion from the local record's evidence: its key order is locally maintained (sorted), whereas
        // the atom's wire-decoded key order is not consensus-checked (evidence equality is order-independent).
        let Some(exhaust_burn_portion) = tx_rec
            .evidence()
            .exhaust_burn_portion(leader_fee.exhaust_burn(), local_committee_info.shard_group())
        else {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: local shard group {} is not in the evidence for transaction {} in block {}",
                local_committee_info.shard_group(),
                atom.id(),
                block,
            );
            return Ok(Some(NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MissingInvolvedShardGroup {
                    shard_group: local_committee_info.shard_group(),
                },
            }));
        };
        *total_exhaust_burn += u128::from(exhaust_burn_portion);

        substate_store.put_diff(&filter_diff_for_committee(local_committee_info, diff))?;

        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::AllAccepted, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(tx_rec)?;

        Ok(None)
    }

    fn evaluate_some_accept_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &TransactionAtom,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        if atom.decision.is_commit() {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept command received for block {} but requires that the atom is ABORT",
                block.id(),
            );
            return Ok(Some(NoVoteReason::SomeAcceptAtomMustBeAbort {
                transaction_id: atom.id,
                block_id: *block.id(),
            }));
        }

        let Some(mut tx_rec) = proposed_block_change_set
            .get_transaction_pool_record(tx, &block.as_leaf(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if !tx_rec.current_stage().is_local_accepted() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ Stage disagreement in block {} for transaction {}. Leader proposed SomeAccept, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.id(),
                tx_rec.current_stage()
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                expected: TransactionPoolStage::LocalAccepted,
                stage: tx_rec.current_stage(),
            }));
        }

        // We check that the leader decision is the same as our local decision (this will change to ABORT once we've
        // received the foreign LocalAccept).
        if tx_rec.current_decision().is_commit() {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept decision disagreement for transaction {} in block {}. Leader proposed ABORT, we decided COMMIT",
                tx_rec.id(),
                block,
            );
            return Ok(Some(NoVoteReason::DecisionDisagreement {
                local: Decision::Commit,
                remote: atom.decision,
            }));
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(Some(NoVoteReason::FeeDisagreement));
        }

        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::SomeAccepted, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(tx_rec)?;

        Ok(None)
    }

    fn evaluate_foreign_proposal_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        local_block: &Block,
        fp_atom: &ForeignProposalAtom,
        local_committee_info: &CommitteeInfo,
        foreign_shard_group: ShardGroup,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        if proposed_block_change_set
            .proposed_foreign_proposals()
            .contains(&fp_atom.block_id)
        {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Foreign proposal {block_id} has already been proposed in this block.",
                block_id = fp_atom.block_id,
            );
            return Ok(Some(NoVoteReason::ForeignProposalAlreadyProposed));
        }

        let Some(fp) = fp_atom.get_proposal(tx).optional()? else {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Foreign proposal {block_id} has not been received.",
                block_id = fp_atom.block_id,
            );
            return Ok(Some(NoVoteReason::ForeignProposalNotReceived));
        };

        // Case: cannot re-propose if it is already committed
        // TODO: if this is already proposed we need to reject if it is already proposed in the current block's
        // commit->leaf chain Currently we allow it to be proposed again
        if matches!(fp.status(), ForeignProposalStatus::Confirmed) {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Foreign proposal {block_id} has status {status}.",
                block_id = fp_atom.block_id,
                status = fp.status(),
            );
            return Ok(Some(NoVoteReason::ForeignProposalAlreadyConfirmed));
        }

        if let Err(err) = process_foreign_block(
            tx,
            &local_block.as_leaf(),
            fp.proposal(),
            local_committee_info,
            substate_store,
            proposed_block_change_set,
        ) {
            // TODO: split validation errors from HotStuff errors so that we can selectively crash or not vote
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Failed to process foreign proposal {foreign_block_id} from {shard_group} for local block {block} Error: {error}",
                block = local_block,
                foreign_block_id = fp_atom.block_id,
                error = err,
                shard_group = foreign_shard_group,
            );
            return Ok(Some(NoVoteReason::ForeignProposalProcessingFailed));
        }

        proposed_block_change_set.set_foreign_proposal_proposed_in(fp_atom.block_id);

        Ok(None)
    }

    fn execute_transaction<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: LeafBlock,
        locked_epoch: LockedEpoch,
        transaction: TransactionRecord,
        change_set: &ProposedBlockChangeSet,
    ) -> Result<BlockTransactionExecution, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "👨‍🔧 DECIDE: Executing transaction {} in block {}",
            transaction.id(),
            block,
        );
        // Might have been executed already in on propose
        if let Some(execution) =
            BlockTransactionExecution::get_pending_for_block(tx, transaction.id(), &block).optional()?
        {
            return Ok(execution);
        }

        let mut pledged = PledgedTransaction::load_pledges(tx, transaction)?;
        let transaction_id = *pledged.id();
        pledged
            .foreign_pledges
            .extend(change_set.get_foreign_pledges(&transaction_id).cloned());

        let execution = self
            .transaction_manager
            .execute(locked_epoch, pledged)
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        Ok(execution.for_block(block, transaction_id))
    }

    fn on_commit(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        commit_qc_id: &PcId,
        block: &Block,
    ) -> Result<Vec<TransactionPoolRecord>, HotStuffError> {
        let committed_transactions = self.finalize_block(tx, commit_qc_id, block)?;
        debug!(
            target: LOG_TARGET,
            "✅ COMMIT block {}",
            block,
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
        new_locked_block: &Block,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔒️ LOCKED BLOCK: {}",
            new_locked_block,
        );

        // Release all locks for Aborted transactions since these can never be committed after accept
        SubstateRecord::unlock_all(
            tx,
            new_locked_block
                .all_local_accept()
                .filter(|a| a.decision.is_abort())
                .map(|t| &t.id),
        )?;

        // Remove the orphaned chains
        // This also releases any locks for fork blocks.
        new_locked_block.remove_orphaned_blocks(tx)?;
        new_locked_block.lock_executions(tx)?;

        Ok(())
    }

    fn publish_event(&self, event: HotstuffEvent) {
        if let Some(sender) = self.tx_events.upgrade() {
            let _ignore = sender.send(event);
        }
    }

    fn finalize_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        commit_qc_id: &PcId,
        block: &Block,
    ) -> Result<Vec<TransactionPoolRecord>, HotStuffError> {
        if block.is_dummy() {
            block.increment_leader_failure_count(
                tx,
                self.config.consensus_constants.missed_proposal_recovery_threshold,
            )?;

            // Nothing to do here for empty dummy blocks. Just mark the block as committed.
            block.commit_block_without_state_changes(tx, commit_qc_id)?;
            return Ok(vec![]);
        }

        info!(target: LOG_TARGET, "🌳 Finalizing block {}", block);

        // This moves the stage update from pending to current for all transactions on the commit block
        self.transaction_pool.confirm_all_transitions(tx, &block.as_leaf())?;

        for atom in block.all_foreign_proposals() {
            // TODO: we need to keep these ATM to send them if a node needs to catch up
            atom.set_status(tx, ForeignProposalStatus::Confirmed, None)?;
        }

        // NOTE: this must happen before we commit the substate diff because the state transitions use this version
        let pending = block.remove_pending_tree_diff_and_return(tx)?;
        let mut state_tree = ShardedStateTree::new(tx);
        let version_updates = state_tree.commit_diffs(pending)?;
        let tx = state_tree.into_transaction();

        {
            let _timer = TraceTimer::debug(LOG_TARGET, "commit_block");
            block.commit_block(tx, commit_qc_id, &version_updates)?;
        }

        let finalized_transactions = {
            let _timer = TraceTimer::debug(LOG_TARGET, "remove finalized transactions");
            self.transaction_pool
                .remove_all(tx, block.all_finalising_transactions_ids())?
        };

        // Whenever we commit a block that will result in an abort for a transaction, we can remove lock conflicts to
        // allow other "blocked" transactions to be proposed.
        TransactionLockConflicts::remove_for_transactions(tx, block.all_aborting_transaction_ids())?;
        TransactionLockConflicts::remove_for_block(tx, block.id())?;

        if !finalized_transactions.is_empty() {
            let _timer = TraceTimer::debug(LOG_TARGET, "unlock and finalized transactions")
                .with_iterations(finalized_transactions.len());
            // Remove locks for finalized transactions
            SubstateRecord::unlock_all(tx, finalized_transactions.iter().map(|t| t.id()))?;
            TransactionRecord::finalize_all(tx, block.epoch(), &finalized_transactions)?;

            debug!(
                target: LOG_TARGET,
                "✅ {} transactions finalized",
                finalized_transactions.len(),
            );
        }

        let total_transaction_fee = block.calculate_total_transaction_fee();
        if total_transaction_fee > 0 {
            info!(
                target: LOG_TARGET,
                "🪙 Validator fee ({}, Total Fees Paid = {}) for block {}",
                block.total_leader_fee(),
                total_transaction_fee,
                block,
            );
        }

        tx.validator_epoch_stats_updates(
            block.justify().epoch(),
            block.justify().signatures().iter().map(|s| s.public_key()).map(|pk| {
                ValidatorStatsUpdate::new(pk)
                    .increment_participation_share()
                    .decrement_missed_proposal()
            }),
        )?;
        block.clear_leader_failure_count(tx)?;

        Ok(finalized_transactions)
    }
}

/// Consensus rule: a block is rejected (no-vote) when its total transaction execution weight exceeds
/// `max_validation_weight`, EXCEPT when it carries at most one transaction command. The single-command
/// exemption preserves liveness: an individually-heavy transaction must remain committable (and the
/// static weight over-estimates its real execution). This bounds the work a leader can force replicas to
/// do per block. Must be evaluated identically on every node — keep it a pure function of these inputs.
fn exceeds_block_validation_weight(
    block_execution_weight: u64,
    num_transaction_commands: usize,
    max_validation_weight: u64,
) -> bool {
    num_transaction_commands > 1 && block_execution_weight > max_validation_weight
}

#[cfg(test)]
mod tests {
    use super::*;

    mod exceeds_block_validation_weight {
        use super::*;

        #[test]
        fn within_budget_is_allowed() {
            assert!(!exceeds_block_validation_weight(10_000, 50, 15_000));
            // Exactly at the cap is allowed.
            assert!(!exceeds_block_validation_weight(15_000, 50, 15_000));
        }

        #[test]
        fn over_budget_with_multiple_commands_is_rejected() {
            assert!(exceeds_block_validation_weight(15_001, 2, 15_000));
            assert!(exceeds_block_validation_weight(31_000, 500, 15_000));
        }

        #[test]
        fn single_heavy_transaction_is_always_allowed() {
            // A single transaction heavier than the whole cap must stay committable (liveness).
            assert!(!exceeds_block_validation_weight(1_000_000, 1, 15_000));
            assert!(!exceeds_block_validation_weight(0, 0, 15_000));
        }
    }
}
