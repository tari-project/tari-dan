//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, fmt::Display, num::NonZeroU64, time::Instant};

use log::*;
use ootle_byte_type::ToByteType;
use tari_consensus_types::{BlockId, HighPc, HighestSeenBlock, LeafBlock, ProposalCertificate, TimeoutCertificate};
use tari_crypto::tari_utilities::epoch_time::EpochTime;
use tari_engine_types::{commit_result::RejectReason, fees::FeeScaleSchedule};
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    ExtraFieldKey,
    NodeHeight,
    ProtocolVersion,
    ValidatorSetHash,
    committee::CommitteeInfo,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{
        Block,
        BlockCommands,
        BlockHeader,
        BlockTransactionExecution,
        BookkeepingModel,
        Command,
        EndEpochAtom,
        ForeignProposal,
        ForeignProposalRecord,
        ForeignProposalStatus,
        LockedEpoch,
        MultiShardAtom,
        PendingShardStateTreeDiff,
        ProposalFilter,
        TransactionExecution,
        TransactionPool,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
    },
};
use tari_ootle_transaction::TransactionId;
use tari_sidechain::FixedHash;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::task;

use crate::{
    hotstuff::{
        EpochEndDrain,
        HotstuffConfig,
        abort_if_local_prepared_expired,
        abort_local_prepared_for_epoch_end,
        apply_leader_fee_to_substate_store,
        block_change_set::ProposedBlockChangeSet,
        calculate_state_merkle_root,
        command_derivation::{DerivedCommand, derive_local_only_command, derive_new_multishard_command},
        epoch_state::EpochState,
        error::HotStuffError,
        exceeds_block_foreign_proposal_bytes,
        exhaust_burn_rate::resolve_epoch_exhaust_burn_rate,
        filter_diff_for_committee,
        foreign_proposal_processor::process_foreign_block,
        local_pledged_inputs,
        process_newly_justified_block,
        substate_store::{InputPledge, PendingSubstateStore},
        transaction_manager::{
            ConsensusTransactionManager,
            ExecutionMemo,
            PledgedTransaction,
            PreparedTransaction,
            SpeculationCandidate,
            TransactionLockConflicts,
        },
    },
    messages::{HotstuffMessage, ProposalMessage},
    tracing::TraceTimer,
    traits::{CertificateStore, ConsensusSpec, OutboundMessaging, ValidatorSignerService, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_propose";

/// One block in this many is a heavy turn (see `select_proposal_batch`): a transaction weighing more than half the
/// block's remaining transaction budget is packed first, and one heavier than that whole budget may take the block
/// alone, even when lighter ready transactions are waiting. This caps the share of blocks oversized transactions can
/// take from transactions that fit.
const HEAVY_TRANSACTION_TURN_INTERVAL: u64 = 10;

/// The leader samples at most this many ready pool records per transaction slot when choosing a block's
/// transactions. A window several times the batch size keeps the choice random, and the window moves to a random
/// point of the pool on every proposal.
const PROPOSAL_CANDIDATES_PER_TRANSACTION: usize = 8;

/// The leader reads at most this many pool records, ready or not, per candidate it samples. Together with
/// [`PROPOSAL_CANDIDATES_PER_TRANSACTION`] this bounds the cost of choosing a batch however large the pool grows.
/// Records that are not ready, such as those waiting on other shard groups, count as reads, so when they fill the
/// window a proposal samples fewer candidates.
const PROPOSAL_READS_PER_CANDIDATE: usize = 4;

/// What an end-of-epoch block commits the next epoch to.
#[derive(Debug, Clone, Copy)]
struct NextEpochCommitment {
    epoch_hash: FixedHash,
    validator_set_hash: ValidatorSetHash,
}

struct NextBlock {
    block: Block,
    foreign_proposals: Vec<ForeignProposal>,
    executed_transactions: HashMap<TransactionId, TransactionExecution>,
    lock_conflicts: TransactionLockConflicts,
    /// Foreign proposals this node rejected while proposing. Recorded by the caller, which holds the write
    /// transaction, so that they leave the unconfirmed index and are not selected into a later block.
    invalid_foreign_proposals: Vec<BlockId>,
}

#[derive(Debug, Clone)]
pub struct OnPropose<TConsensusSpec: ConsensusSpec> {
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    transaction_manager: ConsensusTransactionManager<TConsensusSpec::TransactionExecutor, TConsensusSpec::StateStore>,
    signing_service: TConsensusSpec::SignerService,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
}

impl<TConsensusSpec> OnPropose<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        config: HotstuffConfig,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        transaction_manager: ConsensusTransactionManager<
            TConsensusSpec::TransactionExecutor,
            TConsensusSpec::StateStore,
        >,
        signing_service: TConsensusSpec::SignerService,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
    ) -> Self {
        Self {
            config,
            store,
            epoch_manager,
            transaction_pool,
            transaction_manager,
            signing_service,
            outbound_messaging,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        next_height: NodeHeight,
        local_claim_public_key: RistrettoPublicKeyBytes,
        highest_seen_block: HighestSeenBlock,
        dummy_block: Option<LeafBlock>,
        propose_high_tc: Option<TimeoutCertificate>,
        propose_epoch_end: bool,
        epoch_end_drain: EpochEndDrain,
    ) -> Result<(), HotStuffError> {
        let epoch = epoch_state.epoch();
        let local_committee_info = *epoch_state.local_committee_info();
        let local_committee = epoch_state.local_committee();
        let _timer = TraceTimer::info(LOG_TARGET, "OnPropose");

        // The EndEpoch block commits the committee to the *next* epoch's boundary hash and validator set. Only
        // propose it once our (lagged, reorg-stable) oracle has observed that epoch and our epoch manager has
        // assigned its committees — otherwise we have nothing to commit to and would be locking values the
        // committee hasn't ratified. A later round proposes it once both are known.
        let next_epoch_commitment = if propose_epoch_end {
            self.resolve_next_epoch_commitment(epoch).await?
        } else {
            None
        };

        let on_propose = self.clone();

        let (next_block, foreign_proposals) = task::spawn_blocking(move || {
            // Durable because the block is broadcast once this returns: a restarted leader with no record of
            // `LastProposed` could propose a competing block at the same height.
            on_propose.store.with_durable_write_tx(|tx| {
                let high_qc = HighPc::get(&**tx, epoch)?;
                let high_qc_cert = ProposalCertificate::get(&**tx, epoch, high_qc.id())?;
                let next_block = on_propose.build_next_block(
                    &**tx,
                    epoch,
                    next_height,
                    highest_seen_block,
                    dummy_block,
                    high_qc_cert,
                    &local_committee_info,
                    &local_claim_public_key,
                    propose_high_tc,
                    next_epoch_commitment,
                    epoch_end_drain,
                )?;

                let NextBlock {
                    block: next_block,
                    foreign_proposals,
                    executed_transactions,
                    lock_conflicts,
                    invalid_foreign_proposals,
                } = next_block;

                lock_conflicts.save_for_block(tx, next_block.id())?;

                // Invalid leaves the unconfirmed index, so a proposal rejected here is not selected into a
                // later block. Selection is what makes such a failure repeat: the batch query returns every
                // proposal still in that index, and a validation failure is deterministic across leaders.
                for block_id in invalid_foreign_proposals {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Marking foreign proposal {block_id} invalid: it failed validation while proposing"
                    );
                    ForeignProposalRecord::set_status_by_id(tx, &block_id, ForeignProposalStatus::Invalid, None)?;
                }

                // Add executions for this block
                if !executed_transactions.is_empty() {
                    debug!(
                        target: LOG_TARGET,
                        "Saving {} executed transaction(s) for block {}",
                        executed_transactions.len(),
                        next_block.id()
                    );
                }
                for (transaction_id, executed) in executed_transactions {
                    executed
                        .for_block(next_block.as_leaf(), transaction_id)
                        .insert_if_required(tx)?;
                }

                next_block.as_last_proposed().set(tx)?;

                Ok::<_, HotStuffError>((next_block, foreign_proposals))
            })
        })
        .await??;

        info!(
            target: LOG_TARGET,
            "🌿 [{}] PROPOSING new block {} to {} validators. justifies: {}, parent: {}",
            self.signing_service.public_key(),
            next_block,
            local_committee.len(),
            next_block.justify().height(),
            next_block.parent()
        );

        self.broadcast_local_proposal(next_block, foreign_proposals, &local_committee_info)
            .await?;

        Ok(())
    }

    pub async fn broadcast_local_proposal(
        &mut self,
        next_block: Block,
        foreign_proposals: Vec<ForeignProposal>,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let epoch = next_block.epoch();
        let leaf_block = next_block.as_leaf();
        let msg = HotstuffMessage::new_proposal(ProposalMessage {
            block: next_block,
            foreign_proposals,
        });

        // Broadcast to local and foreign committees
        self.outbound_messaging.send_self(msg.clone()).await?;

        // If we are the only VN in this committee, no need to multicast
        if local_committee_info.num_shard_group_members() <= 1 {
            info!(
                target: LOG_TARGET,
                "🌿 This node is the only member of the local committee. No need to multicast proposal {leaf_block}",
            );
        } else {
            let committee = self
                .epoch_manager
                .get_committee_by_shard_group(epoch, local_committee_info.shard_group())
                .await?;

            info!(
                target: LOG_TARGET,
                "🌿 Broadcasting local proposal to {}/{} local committee members {}",
                committee.len(), local_committee_info.num_shard_group_members(), leaf_block,
            );

            if let Err(err) = self
                .outbound_messaging
                .multicast(committee.address_iter().cloned(), msg)
                .await
            {
                warn!(
                    target: LOG_TARGET,
                    "Failed to multicast proposal to local committee: {}",
                    err
                );
            }
        }

        Ok(())
    }

    /// Returns Ok(None) if the command cannot be sequenced yet due to lock conflicts.
    #[allow(clippy::too_many_arguments)]
    fn transaction_pool_record_to_command<TTx: StateStoreReadTransaction>(
        &self,
        state_anchor: &LeafBlock,
        locked_epoch: &LockedEpoch,
        pool_tx: TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        change_set: &ProposedBlockChangeSet,
        substate_store: &mut PendingSubstateStore<TTx>,
        memo: &mut ExecutionMemo,
        executed_transactions: &mut HashMap<TransactionId, TransactionExecution>,
        lock_conflicts: &mut TransactionLockConflicts,
    ) -> Result<Option<Command>, HotStuffError> {
        match pool_tx.current_stage() {
            TransactionPoolStage::New => self.prepare_transaction(
                state_anchor,
                locked_epoch,
                pool_tx,
                local_committee_info,
                change_set,
                substate_store,
                memo,
                executed_transactions,
                lock_conflicts,
            ),
            // Leader thinks all foreign PREPARE pledges have been received (condition for LocalPrepared stage to be
            // ready)
            TransactionPoolStage::LocalPrepared => self.local_accept_transaction(
                state_anchor,
                local_committee_info,
                change_set,
                pool_tx,
                substate_store,
                executed_transactions,
            ),

            // Leader thinks that all foreign ACCEPT pledges have been received and, we are ready to accept the result
            // (COMMIT/ABORT)
            TransactionPoolStage::LocalAccepted => {
                self.accept_transaction(state_anchor, &pool_tx, local_committee_info, substate_store)
            },
            // Not reachable as there is nothing to propose for these stages. To confirm that all local nodes
            // agreed with the Accept, more (possibly empty) blocks with QCs will be
            // proposed and accepted, otherwise the Accept block will not be committed.
            TransactionPoolStage::AllAccepted |
            TransactionPoolStage::SomeAccepted |
            TransactionPoolStage::LocalOnly => {
                unreachable!(
                    "It is invalid for TransactionPoolStage::{} to be ready to propose",
                    pool_tx.current_stage()
                )
            },
        }
    }

    /// What an end-of-epoch block for `epoch` commits the next epoch to, or `None` while this node does not yet
    /// know the next epoch's boundary hash or committees.
    async fn resolve_next_epoch_commitment(&self, epoch: Epoch) -> Result<Option<NextEpochCommitment>, HotStuffError> {
        let next_epoch = epoch + Epoch(1);
        let Some(epoch_hash) = self.epoch_manager.get_epoch_hash(next_epoch).await.optional()? else {
            info!(
                target: LOG_TARGET,
                "⏳ Not proposing EndEpoch for {epoch}: oracle has not yet observed the boundary block for {next_epoch}"
            );
            return Ok(None);
        };
        let Some(validator_set_hash) = self.epoch_manager.get_validator_set_hash(next_epoch).await.optional()? else {
            info!(
                target: LOG_TARGET,
                "⏳ Not proposing EndEpoch for {epoch}: this node has not assigned the committees for {next_epoch}"
            );
            return Ok(None);
        };
        Ok(Some(NextEpochCommitment {
            epoch_hash,
            validator_set_hash,
        }))
    }

    #[allow(clippy::too_many_lines)]
    fn build_next_block<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        epoch: Epoch,
        next_height: NodeHeight,
        highest_seen_block: HighestSeenBlock,
        dummy_block: Option<LeafBlock>,
        high_qc_certificate: ProposalCertificate,
        local_committee_info: &CommitteeInfo,
        local_claim_public_key_bytes: &RistrettoPublicKeyBytes,
        propose_high_tc: Option<TimeoutCertificate>,
        // `Some` if we are proposing an end-of-epoch block; carries what it commits the next epoch to.
        end_epoch: Option<NextEpochCommitment>,
        epoch_end_drain: EpochEndDrain,
    ) -> Result<NextBlock, HotStuffError> {
        let high_qc_id = high_qc_certificate.calculate_id();
        let justify_block = Block::get_justified_block(tx, &high_qc_certificate, epoch)?;
        let end_epoch = match end_epoch {
            Some(commitment) if epoch_end_drain.is_active() => {
                let is_draining = self.transaction_pool.any_in_chain(tx, justify_block.id(), |rec| {
                    epoch_end_drain.holds_epoch_end_for(rec.current_stage())
                })?;
                if is_draining {
                    info!(
                        target: LOG_TARGET,
                        "🌊 Not proposing EndEpoch for {epoch}: cross-shard transactions are still in flight \
                         ({epoch_end_drain:?})",
                    );
                    None
                } else {
                    Some(commitment)
                }
            },
            end_epoch => end_epoch,
        };
        let parent_block = dummy_block.unwrap_or_else(|| justify_block.as_leaf());
        let highest_seen_block = Block::get(tx, highest_seen_block.block_id())?;
        // The epoch end is read on the chain the candidate extends, which is the justify block's. A block that
        // this node has seen but the high certificate does not reach is on a branch the candidate abandons, and an
        // EndEpoch there binds nothing.
        let is_end_of_epoch_in_chain = justify_block.is_epoch_end_proposed_in_chain(tx)?;

        let should_not_propose_commands = is_end_of_epoch_in_chain || end_epoch.is_some();

        let mut total_leader_fee = 0u64;
        // The block the candidate extends from, and the point at which every speculative state and pool read for
        // this proposal is taken. The candidate's parent is the justify block itself, or the last of the dummy
        // blocks that fill a timeout gap - empty blocks carrying justify_block's accumulated_data and state forward
        // (see `calculate_last_dummy_block`). Either way the parent chain runs back to justify_block and never
        // through a locally-accepted block that the high QC does not certify. A validator recomputes that same
        // chain, so anything read at a block off it is state it does not have: speculative substate changes and
        // leader-fee burn surface as an exhaust-burn or state Merkle-root mismatch, and pool records read there
        // carry stages and decisions from blocks the candidate abandons.
        //
        // The locked epoch below and the candidate's own epoch hash are read at `highest_seen_block`. Both are
        // constant across an epoch, and the candidate is in the same epoch as every block this node has seen.
        let state_anchor = &justify_block;
        let state_anchor_leaf = state_anchor.as_leaf();
        let mut accumulated_data = *state_anchor.header().accumulated_data();

        let mut substate_store =
            PendingSubstateStore::init(tx, state_anchor_leaf, self.config.consensus_constants.num_preshards)?
                .with_pledge_budget(self.config.consensus_constants.max_block_pledge_bytes);

        let mut executed_transactions = HashMap::new();

        let proposal_filter = epoch_end_drain.proposal_filter(epoch, local_committee_info.shard_group());
        let batch = if should_not_propose_commands {
            ProposalBatch::default()
        } else {
            self.fetch_next_proposal_batch(tx, state_anchor_leaf, next_height, &proposal_filter)?
        };
        debug!(target: LOG_TARGET, "🌿 PROPOSE: {} (justify: {}) {batch}", highest_seen_block.height(), justify_block.height());

        let mut commands = if is_end_of_epoch_in_chain {
            vec![]
        } else if let Some(end_epoch) = end_epoch {
            vec![Command::EndEpoch(EndEpochAtom::new(end_epoch.epoch_hash))]
        } else {
            batch
                .foreign_proposals
                .iter()
                .map(|fp| Command::ForeignProposal(fp.to_atom()))
                .collect()
        };

        // NOTE: the block for the change set is not used.
        let mut change_set = ProposedBlockChangeSet::new(state_anchor_leaf);
        let mut invalid_foreign_proposals = Vec::new();
        let mut dropped_foreign_proposals = false;

        // No need to include evidence from justified block if no transactions are included in the next block
        if !batch.transactions.is_empty() {
            // A replica evaluates this block as: the newly justified block, then the commands in block order
            // (foreign proposals sort before the transaction commands, see `BlockCommands`), all against
            // a single change set. The commands generated below must be derived from that same sequence, or
            // the proposer commits to an atom no replica can reproduce and the block is unvotable.
            // TODO: we dont need to process transactions here that are not in the batch
            process_newly_justified_block(tx, &justify_block, high_qc_id, local_committee_info, &mut change_set)?;

            // A failed `process_foreign_block` leaves whatever it wrote before the error behind, and the
            // transaction commands below are derived from the change set, so that partial state would shape
            // atoms no replica reproduces. Restoring this snapshot is what keeps a rejected foreign proposal
            // from reaching them.
            let mut change_set_before_foreign_proposals =
                (!batch.foreign_proposals.is_empty()).then(|| change_set.clone());

            for fp in &batch.foreign_proposals {
                // Resolves pending transaction pool records along the chain the substate store is built on:
                // the justify block under a dummy chain, the extended leaf otherwise. A replica's store is
                // built on the block it is evaluating, whose parent chain runs back through any dummies to
                // the justify block.
                if let Err(err) =
                    process_foreign_block(tx, fp, local_committee_info, &mut substate_store, &mut change_set)
                {
                    // Dropping the proposals is safe for every error class and is what keeps a block whose
                    // commands no replica can reproduce off the wire. Propagating instead would take the
                    // whole consensus loop down over a remote committee's proposal.
                    warn!(
                        target: LOG_TARGET,
                        "⚠️❌ Foreign proposal {} failed to process while proposing: {err}. Dropping every \
                         foreign proposal from this block.",
                        fp.to_atom().block_id,
                    );

                    // Only a validation failure condemns the proposal: it is a property of the proposal
                    // itself, so every leader reaches the same verdict. Every other error is a statement
                    // about this node, and a proposal discarded on one would never be proposed again.
                    if err.validation_error().is_some() {
                        invalid_foreign_proposals.push(fp.to_atom().block_id);
                    }

                    change_set = change_set_before_foreign_proposals
                        .take()
                        .expect("snapshot is taken whenever there is a foreign proposal to restore from");
                    // The proposals already applied to the discarded change set are not re-applied, so none
                    // of them may be proposed here. They keep their `New` status and are proposed again next
                    // round.
                    commands.retain(|cmd| !matches!(cmd, Command::ForeignProposal(_)));
                    dropped_foreign_proposals = true;
                    break;
                }
            }

            // Add all (ABORT) executions that may have resulted from foreign proposals
            executed_transactions.extend(change_set.take_transaction_executions());
        }

        let locked_epoch = LockedEpoch::new(
            highest_seen_block.epoch(),
            highest_seen_block.epoch_hash().into_array().into(),
            highest_seen_block.exhaust_burn_rate(),
        );

        // batch is empty for is_empty, is_epoch_end and is_epoch_start blocks
        let batch_size = batch.transactions.len();
        let timer = TraceTimer::info(LOG_TARGET, "Generating commands").with_iterations(batch_size);
        let mut lock_conflicts = TransactionLockConflicts::new();
        // Local proposing heuristic (not a consensus rule, so no fork risk): stop executing transactions
        // once roughly half the block time has been spent, so that a batch of heavy transactions cannot
        // push proposing past the block time. The static weight budget bounds block size; this bounds the
        // actual execution latency that the static estimate cannot perfectly predict. Deferred
        // transactions remain in the pool and are proposed in a later block.
        let propose_exec_deadline = self.config.consensus_constants.pacemaker_block_time / 2;
        let propose_started_at = Instant::now();
        let mut memo = self.speculate_batch(
            &substate_store,
            local_committee_info,
            &state_anchor_leaf,
            &locked_epoch,
            &batch.transactions,
            &change_set,
        );
        // Accumulated to log observed execution throughput (weight/s) for calibrating `max_block_weight`
        // against the block time. See the calibration log emitted after the loop.
        let mut executed_weight = 0u64;
        let mut num_executed = 0usize;
        // Running total of WASM metering points consumed by transactions executed for this block. A
        // transaction's points are only known after it executes, so the block can overshoot the budget
        // by at most MAX_WASM_POINTS_PER_TRANSACTION — `max_block_validation_execution_points` allows for
        // this, so honest proposals are never rejected.
        let max_block_execution_points = self.config.consensus_constants.max_block_execution_points;
        let mut execution_points_total = 0u64;
        for (idx, mut transaction) in batch.transactions.into_iter().enumerate() {
            // Always attempt at least the first transaction before honouring the deadline so we make
            // progress even when a single transaction is slow to execute.
            if idx > 0 && propose_started_at.elapsed() >= propose_exec_deadline {
                warn!(
                    target: LOG_TARGET,
                    "⏱️ PROPOSE: execution soft deadline ({:.1?}) reached after {} of {} transaction(s); deferring {} to a later block",
                    propose_exec_deadline,
                    idx,
                    batch_size,
                    batch_size - idx,
                );
                break;
            }
            if idx > 0 && execution_points_total >= max_block_execution_points {
                warn!(
                    target: LOG_TARGET,
                    "🔋 PROPOSE: wasm points budget ({}/{}) reached after {} of {} transaction(s); deferring {} to a later block",
                    execution_points_total,
                    max_block_execution_points,
                    idx,
                    batch_size,
                    batch_size - idx,
                );
                break;
            }
            // Apply the transaction updates (if any) that the justified block and this block's foreign
            // proposals produced. This allows us to propose evidence relating to transactions in the
            // justified block, and to propose a transaction that a foreign proposal in this block has just
            // moved to ABORT with the decision that move implies.
            change_set.apply_transaction_update(&mut transaction);
            if proposal_filter.holds_back(&transaction) {
                debug!(
                    target: LOG_TARGET,
                    "🌊 PROPOSE: holding back cross-shard transaction {} until the next epoch",
                    transaction.id(),
                );
                continue;
            }
            let expired_abort = abort_if_local_prepared_expired(&mut transaction, epoch).or_else(|| {
                let is_waiting = proposal_filter.aborts_waiting_prepared() &&
                    !transaction.is_ready_for_pending_stage(local_committee_info.shard_group());
                is_waiting
                    .then(|| abort_local_prepared_for_epoch_end(&mut transaction))
                    .flatten()
            });
            // Replicas withhold their vote unless the updated record is ready to leave its stage, so readiness is
            // judged on the same record here.
            if !transaction.is_ready_for_pending_stage(local_committee_info.shard_group()) {
                debug!(
                    target: LOG_TARGET,
                    "🌿 PROPOSE: transaction {} is not ready to leave stage {} after this block's updates. Deferring.",
                    transaction.id(),
                    transaction.current_stage(),
                );
                continue;
            }
            // Capture before the record is moved. The processing work below (incl. execution) is incurred
            // whether or not a command is produced, so accumulate for every processed transaction.
            executed_weight += transaction.proposal_weight();
            num_executed += 1;
            let tx_id = *transaction.id();
            if let Some(execution) = expired_abort {
                executed_transactions.insert(tx_id, execution);
            }
            // Entries already present (e.g. foreign-proposal aborts from the change set) were not executed
            // for this block, so only count executions newly produced by the command conversion below.
            let had_execution = executed_transactions.contains_key(&tx_id);
            let maybe_command = self.transaction_pool_record_to_command(
                &state_anchor_leaf,
                // This locked epoch is used to set the transaction LockedEpoch if necessary
                &locked_epoch,
                transaction,
                local_committee_info,
                &change_set,
                &mut substate_store,
                &mut memo,
                &mut executed_transactions,
                &mut lock_conflicts,
            )?;
            if let Some(command) = maybe_command {
                // Count points only for transactions actually proposed in this block: the budget bounds
                // replica re-execution per block, mirroring the validation-side sum over block commands. A
                // skipped transaction (e.g. lock conflict) is counted in the later block that proposes it;
                // its propose-time execution cost here is bounded by the soft deadline above.
                if !had_execution && let Some(execution) = executed_transactions.get(&tx_id) {
                    execution_points_total =
                        execution_points_total.saturating_add(execution.result().total_execution_points());
                }
                total_leader_fee = total_leader_fee
                    .checked_add(
                        command
                            .committing()
                            .and_then(|tx| tx.leader_fee())
                            .map(|f| f.fee)
                            .unwrap_or(0),
                    )
                    .ok_or_else(|| {
                        HotStuffError::InvariantError("Leader fee overflow when summing for block".to_string())
                    })?;
                let exhaust_burn_portion = command
                    .exhaust_burn_portion(local_committee_info.shard_group())
                    .ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Local shard group {} is not in the evidence of committing command {command}",
                            local_committee_info.shard_group(),
                        ))
                    })?;
                accumulated_data.total_exhaust_burn += u128::from(exhaust_burn_portion);
                commands.push(command);
            }
        }
        timer.done();
        if batch_size > 0 {
            info!(target: LOG_TARGET, "⚡ Speculated executions for the proposal at height {next_height}: {memo}");
        }

        let commands = BlockCommands::init(commands).map_err(|e| {
            HotStuffError::InvariantError(format!(
                "Proposal at height {next_height} generated two commands for one block position: {e}"
            ))
        })?;

        // Calibration signal: observed propose-time execution throughput and the time a full
        // `max_block_weight` block would take at that rate. Use this (from real traffic, at debug level)
        // to tune `max_block_weight` so a full block executes well within the block time.
        let exec_elapsed = propose_started_at.elapsed();
        let exec_secs = exec_elapsed.as_secs_f64();
        if executed_weight > 0 && exec_secs > 0.0 {
            let weight_per_sec = executed_weight as f64 / exec_secs;
            let projected_full_block_secs = self.config.consensus_constants.max_block_weight as f64 / weight_per_sec;
            debug!(
                target: LOG_TARGET,
                "📊 PROPOSE calibration: executed {} command(s) / {} weight in {:.2?} (~{:.0} weight/s); a full \
                 max_block_weight={} block projects to ~{:.2}s of execution (block_time={:.0?})",
                num_executed,
                executed_weight,
                exec_elapsed,
                weight_per_sec,
                self.config.consensus_constants.max_block_weight,
                projected_full_block_secs,
                self.config.consensus_constants.pacemaker_block_time,
            );
        }

        debug!(
            target: LOG_TARGET,
            "command(s) for next block: [{}]",
            commands.display()
        );

        // Add proposer fee substate
        if total_leader_fee > 0 {
            // Apply leader fee to substate store before we calculate the state root
            apply_leader_fee_to_substate_store(
                &mut substate_store,
                local_claim_public_key_bytes,
                local_committee_info.num_preshards(),
                local_committee_info.shard_group().start(),
                total_leader_fee,
            )?;
        }

        let timer = TraceTimer::info(LOG_TARGET, "Propose calculate state root");
        let pending_tree_diffs =
            PendingShardStateTreeDiff::get_all_up_to_commit_block(tx, state_anchor_leaf.block_id())?;
        let (state_root, _) = calculate_state_merkle_root(
            tx,
            local_committee_info.shard_group(),
            pending_tree_diffs,
            substate_store.changes()
                .iter()
                // Calculate for local shards only and the global shard
                .filter(|ch| local_committee_info.shard_group().contains_or_global(&ch.shard())),
            self.config.network,
            epoch,
        )?;
        timer.done();

        // Every block names the rate its epoch runs at, carried forward from the block it extends, so a
        // transaction settles at a rate the block that sequenced it commits to. An end-of-epoch block
        // additionally names the rate the next epoch opens at: that is the value the quorum committing it
        // ratifies, and the next epoch's genesis block carries it forward from there. It also names the next
        // epoch's validator set hash, which the same quorum ratifies, so that each end-of-epoch block hands the
        // chain over from one epoch's committees to the next, and the fee scale the next epoch charges at.
        let mut extra_data = ExtraData::new();
        extra_data.insert_bps(
            ExtraFieldKey::ExhaustBurnRate,
            highest_seen_block.exhaust_burn_rate().as_bps(),
        );
        // Only the block carrying the `EndEpoch` command names the next epoch. The blocks that extend it before the
        // epoch changes still hold the commitment, but carry no commands.
        if let Some(end_epoch) = end_epoch.filter(|_| !is_end_of_epoch_in_chain) {
            let next_epoch = epoch + Epoch(1);
            let next_epoch_rate = resolve_epoch_exhaust_burn_rate(tx, self.config.network, next_epoch)?;
            extra_data
                .insert_bps(ExtraFieldKey::NextEpochExhaustBurnRate, next_epoch_rate.as_bps())
                .insert_validator_set_hash(ExtraFieldKey::NextEpochValidatorSetHash, &end_epoch.validator_set_hash)
                .insert_bps(
                    ExtraFieldKey::NextEpochFeeScale,
                    FeeScaleSchedule::at(self.config.network, next_epoch).as_bps(),
                );
        }

        let mut header = BlockHeader::create_unsigned(
            self.config.network,
            ProtocolVersion::at(self.config.network, epoch),
            *parent_block.block_id(),
            high_qc_id,
            propose_high_tc.as_ref().map(|tc| tc.calculate_id()),
            next_height,
            epoch,
            local_committee_info.shard_group(),
            self.signing_service.public_key().to_byte_type(),
            state_root,
            &commands,
            total_leader_fee,
            EpochTime::now().as_u64(),
            *highest_seen_block.epoch_hash(),
            accumulated_data,
            extra_data,
        )?;

        let signature = self.signing_service.sign(&header);
        header.set_signature(signature.to_byte_type());

        let next_block = Block::new(header, high_qc_certificate, commands, propose_high_tc);

        Ok(NextBlock {
            block: next_block,
            // A proposal the block no longer carries a command for must not ride along on the wire: a
            // replica that has not seen it stores it as `New` on receipt, which re-seeds the committee with
            // a proposal this node has just condemned.
            foreign_proposals: if dropped_foreign_proposals {
                Vec::new()
            } else {
                batch.foreign_proposals
            },
            executed_transactions,
            lock_conflicts,
            invalid_foreign_proposals,
        })
    }

    /// Speculates the batch's transactions that are New, as the commands generated for them will see them once the
    /// block's foreign proposals are applied.
    fn speculate_batch<TTx: StateStoreReadTransaction>(
        &self,
        substate_store: &PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        state_anchor: &LeafBlock,
        locked_epoch: &LockedEpoch,
        transactions: &[TransactionPoolRecord],
        change_set: &ProposedBlockChangeSet,
    ) -> ExecutionMemo {
        let candidates = transactions
            .iter()
            .filter_map(|pool_tx| {
                let mut pool_tx = pool_tx.clone();
                change_set.apply_transaction_update(&mut pool_tx);
                if !pool_tx.current_stage().is_new() || pool_tx.current_decision().is_abort() {
                    return None;
                }
                pool_tx.update_locked_epoch(locked_epoch.clone());
                Some(SpeculationCandidate {
                    transaction_id: *pool_tx.id(),
                    locked_epoch: pool_tx.locked_epoch()?.clone(),
                })
            })
            .collect::<Vec<_>>();
        self.transaction_manager
            .speculate(substate_store, local_committee_info, state_anchor.epoch, &candidates)
    }

    #[allow(clippy::too_many_lines)]
    fn fetch_next_proposal_batch<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        state_anchor_leaf: LeafBlock,
        next_height: NodeHeight,
        proposal_filter: &ProposalFilter,
    ) -> Result<ProposalBatch, HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "fetch_next_proposal_batch");
        // A block is budgeted by total command weight (`max_block_weight`), not a flat command count.
        // Foreign proposals consume part of that budget before local transactions fill the rest. A foreign proposal is
        // weighted by the substate pledges it carries (the dominant processing cost when applying it at propose
        // time), on the same scale as transaction input weight, rather than a flat 10x multiplier.
        const MAX_FOREIGN_PROPOSALS_PER_BLOCK: usize = 10;
        const FP_BASE_WEIGHT: u64 = 50;
        const FP_PLEDGE_WEIGHT: u64 = 15;

        let max_block_weight = self.config.consensus_constants.max_block_weight;
        let max_commands = self.config.consensus_constants.max_commands_in_block;

        let mut foreign_proposals =
            ForeignProposalRecord::get_all_new(tx, state_anchor_leaf.block_id(), MAX_FOREIGN_PROPOSALS_PER_BLOCK)?;

        if !foreign_proposals.is_empty() {
            debug!(
                target: LOG_TARGET,
                "🌿 Found {} foreign proposals for next block",
                foreign_proposals.len()
            );
        }

        // The block travels with every foreign proposal it includes, so they are taken in order for as long as they
        // fit the byte budget. The rest stay new for a later block.
        let max_foreign_proposal_bytes = self.config.consensus_constants.max_block_foreign_proposal_bytes;
        let mut foreign_proposal_bytes = 0usize;
        let num_fitting = foreign_proposals
            .iter()
            .enumerate()
            .take_while(|(idx, fp)| {
                let total_bytes = foreign_proposal_bytes.saturating_add(tari_bor::encoded_len(fp.proposal()));
                if exceeds_block_foreign_proposal_bytes(total_bytes, idx + 1, max_foreign_proposal_bytes) {
                    return false;
                }
                foreign_proposal_bytes = total_bytes;
                true
            })
            .count();
        if num_fitting < foreign_proposals.len() {
            info!(
                target: LOG_TARGET,
                "📦 PROPOSE: foreign proposal budget ({max_foreign_proposal_bytes} bytes) reached after {num_fitting} \
                 of {} foreign proposal(s); deferring the rest to a later block",
                foreign_proposals.len(),
            );
            foreign_proposals.truncate(num_fitting);
        }

        let foreign_proposal_weight: u64 = foreign_proposals
            .iter()
            .map(|fp| FP_BASE_WEIGHT + fp.block_pledge().len() as u64 * FP_PLEDGE_WEIGHT)
            .sum();

        let remaining_weight = subtract_weight_checked(Some(max_block_weight), foreign_proposal_weight);

        // Bound the transaction count so the total command count (foreign proposals + transactions)
        // stays under the hard command cap regardless of how light the transactions are.
        let max_tx_count = max_commands.saturating_sub(foreign_proposals.len());
        let heavy_turn = next_height.as_u64().is_multiple_of(HEAVY_TRANSACTION_TURN_INTERVAL);
        let max_candidates = max_tx_count.saturating_mul(PROPOSAL_CANDIDATES_PER_TRANSACTION);

        let transactions = remaining_weight
            .filter(|_| max_tx_count > 0)
            .map(|weight_budget| {
                self.transaction_pool.get_batch_for_next_block(
                    tx,
                    weight_budget,
                    max_tx_count,
                    max_candidates,
                    max_candidates.saturating_mul(PROPOSAL_READS_PER_CANDIDATE),
                    heavy_turn,
                    state_anchor_leaf.block_id(),
                    proposal_filter,
                )
            })
            .transpose()?
            .unwrap_or_default();

        Ok(ProposalBatch {
            foreign_proposals: foreign_proposals.into_iter().map(|fp| fp.into_proposal()).collect(),
            transactions,
            commands: vec![],
        })
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn prepare_transaction<TTx: StateStoreReadTransaction>(
        &self,
        state_anchor: &LeafBlock,
        locked_epoch: &LockedEpoch,
        mut pool_tx: TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        change_set: &ProposedBlockChangeSet,
        substate_store: &mut PendingSubstateStore<TTx>,
        memo: &mut ExecutionMemo,
        executed_transactions: &mut HashMap<TransactionId, TransactionExecution>,
        lock_conflicts: &mut TransactionLockConflicts,
    ) -> Result<Option<Command>, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "👨‍🔧 PROPOSE: PREPARE transaction {}",
            pool_tx.id(),
        );

        // Update locked epoch if needed
        pool_tx.update_locked_epoch(locked_epoch.clone());

        let prepared = self
            .transaction_manager
            .prepare_for_proposal(
                substate_store,
                local_committee_info,
                &pool_tx,
                *state_anchor,
                change_set,
                memo,
            )
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        if prepared.lock_status().is_deferrable_conflict() {
            warn!(
                target: LOG_TARGET,
                "⚠️ Transaction {} has lock conflicts, but no hard conflicts. Skipping proposing this transaction...",
                pool_tx.id(),
            );

            lock_conflicts.add(*pool_tx.id(), prepared.into_lock_status().into_lock_conflicts());
            return Ok(None);
        }

        match prepared {
            PreparedTransaction::PledgeBudgetReached { total_bytes, max_bytes } => {
                info!(
                    target: LOG_TARGET,
                    "📦 PROPOSE: transaction {} would take the block's pledge to {total_bytes} bytes, past the \
                     {max_bytes} budget. Deferring it to a later block",
                    pool_tx.id(),
                );
                Ok(None)
            },
            PreparedTransaction::InputsLocked { .. } => Err(HotStuffError::InvariantError(format!(
                "Transaction {} was prepared as having its inputs locked without a deferrable lock conflict",
                pool_tx.id(),
            ))),
            PreparedTransaction::LocalOnly(local) => {
                let pool_tx_id = *pool_tx.id();
                let derived = derive_local_only_command(pool_tx, *local, local_committee_info).map_err(|refusal| {
                    HotStuffError::InvariantError(format!(
                        "PROPOSE: LocalOnly transaction {} cannot be sequenced: {refusal}",
                        pool_tx_id,
                    ))
                })?;
                info!(
                    target: LOG_TARGET,
                    "🏠️ Transaction {} is local only, proposing {}",
                    pool_tx_id,
                    derived.command,
                );

                if let Some(diff) = derived.committed_diff() {
                    if substate_store.diff_exceeds_shard_output_bytes(
                        diff,
                        self.config.consensus_constants.max_block_shard_output_bytes,
                    ) {
                        info!(
                            target: LOG_TARGET,
                            "📦 PROPOSE: shard output budget reached, deferring LocalOnly transaction {} to a later block",
                            pool_tx_id,
                        );
                        return Ok(None);
                    }
                    if let Err(err) = substate_store.put_diff(diff) {
                        // A lock failure or DOWN input is an expected conflict under contention - skip the
                        // transaction this round. Any other store error is fatal and must be propagated.
                        let lock_err = err.ok_lock_failed()?;
                        warn!(
                            target: LOG_TARGET,
                            "🔒 Skipping LocalOnly transaction {} while proposing due to input conflict: {}",
                            pool_tx_id,
                            lock_err,
                        );
                        return Ok(None);
                    }
                }

                let DerivedCommand { command, execution, .. } = derived;
                if let Some(execution) = execution {
                    executed_transactions.insert(pool_tx_id, execution);
                }
                Ok(Some(command))
            },

            PreparedTransaction::MultiShard(multishard) => {
                let pool_tx_id = *pool_tx.id();
                let DerivedCommand { command, execution, .. } =
                    derive_new_multishard_command(pool_tx, multishard, local_committee_info)?;
                info!(
                    target: LOG_TARGET,
                    "🌍 Transaction involves foreign shard groups, proposing {command}",
                );
                if let Some(execution) = execution {
                    executed_transactions.insert(pool_tx_id, execution);
                }
                Ok(Some(command.into_command()))
            },
        }
    }

    fn local_accept_transaction<TTx: StateStoreReadTransaction>(
        &self,
        state_anchor: &LeafBlock,
        local_committee_info: &CommitteeInfo,
        change_set: &ProposedBlockChangeSet,
        mut tx_rec: TransactionPoolRecord,
        substate_store: &mut PendingSubstateStore<TTx>,
        executed_transactions: &mut HashMap<TransactionId, TransactionExecution>,
    ) -> Result<Option<Command>, HotStuffError> {
        // Only set to abort if either the local or one or more foreign shards decided to ABORT
        if tx_rec.current_decision().is_abort() {
            return Ok(Some(Command::LocalAccept(tx_rec.get_current_multi_shard_atom())));
        }

        let locked_epoch = tx_rec.locked_epoch().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "PROPOSE: local_accept_transaction: Transaction {} is in LocalPrepared stage but has no locked epoch",
                tx_rec.id(),
            ))
        })?;

        let tx = substate_store.read_transaction();
        let transaction = tx_rec.get_transaction(tx)?;
        let execution = self.execute_transaction(
            substate_store,
            local_committee_info,
            state_anchor,
            transaction,
            change_set,
            locked_epoch.clone(),
        )?;

        if execution.decision().is_commit() {
            // Preparing the transaction bounded what its inputs alone may pledge, so only the block budget applies
            let pledge = substate_store.pledge_inputs(
                local_pledged_inputs(&tx_rec, local_committee_info.shard_group()),
                usize::MAX,
            )?;
            if let InputPledge::BlockBudgetReached { total_bytes, max_bytes } = pledge {
                info!(
                    target: LOG_TARGET,
                    "📦 PROPOSE: LocalAccept of transaction {} would take the block's pledge to {total_bytes} bytes, \
                     past the {max_bytes} budget. Deferring it to a later block",
                    tx_rec.id(),
                );
                return Ok(None);
            }
        }

        // Try to lock all local outputs
        let local_outputs = execution
            .resulting_outputs()
            .iter()
            .filter(|o| local_committee_info.includes_substate_id(o.substate_id()));
        let lock_status = substate_store.try_lock_all(*tx_rec.id(), local_outputs, false)?;
        if let Some(err) = lock_status.failures().first() {
            warn!(
                target: LOG_TARGET,
                "⚠️ Failed to lock outputs for transaction {}: {}",
                tx_rec.id(),
                err,
            );
            // If the transaction does not lock, we propose to abort it
            let execution =
                TransactionExecution::abort(tx_rec.id(), RejectReason::FailedToLockOutputs(err.to_string()));
            tx_rec.update_from_execution(
                local_committee_info.num_preshards(),
                local_committee_info.num_committees(),
                &execution,
            );

            executed_transactions.insert(*tx_rec.id(), execution);
            return Ok(Some(Command::LocalAccept(tx_rec.get_current_multi_shard_atom())));
        }

        tx_rec.update_from_execution(
            local_committee_info.num_preshards(),
            local_committee_info.num_committees(),
            &execution,
        );
        executed_transactions.insert(*tx_rec.id(), execution);
        // If we locally decided to ABORT, we are still saying that we think all prepared and, after execution decide to
        // ABORT. When we enter the acceptance phase, we will propose SomeAccept for this case.
        let atom = self.get_multi_shard_atom_with_leader_fee(&tx_rec)?;
        Ok(Some(Command::LocalAccept(atom)))
    }

    fn accept_transaction<TTx: StateStoreReadTransaction>(
        &self,
        state_anchor: &LeafBlock,
        tx_rec: &TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
    ) -> Result<Option<Command>, HotStuffError> {
        if tx_rec.current_decision().is_abort() {
            return Ok(Some(Command::SomeAccept(tx_rec.get_current_multi_shard_atom())));
        }

        let tx = substate_store.read_transaction();
        let execution = tx_rec
            .get_pending_execution_for_block(tx, state_anchor)
            .optional()?
            .ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "accept_transaction: Transaction {} has COMMIT decision but execution is missing",
                    tx_rec.id(),
                ))
            })?;
        let diff = execution.result().finalize.any_accept().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "local_accept_transaction: Transaction {} has COMMIT decision but execution failed when proposing",
                tx_rec.id(),
            ))
        })?;
        let filtered_diff = filter_diff_for_committee(local_committee_info, diff);
        if substate_store.diff_exceeds_shard_output_bytes(
            &filtered_diff,
            self.config.consensus_constants.max_block_shard_output_bytes,
        ) {
            info!(
                target: LOG_TARGET,
                "📦 PROPOSE: shard output budget reached, deferring AllAccept transaction {} to a later block",
                tx_rec.id(),
            );
            return Ok(None);
        }
        if let Err(err) = substate_store.put_diff(&filtered_diff) {
            // A lock failure or DOWN input is an expected conflict under contention - skip the transaction this
            // round. Any other store error is fatal and must be propagated.
            let lock_err = err.ok_lock_failed()?;
            warn!(
                target: LOG_TARGET,
                "🔒 Skipping Accept transaction {} while proposing due to input conflict: {}",
                tx_rec.id(),
                lock_err,
            );
            return Ok(None);
        }
        let atom = self.get_multi_shard_atom_with_leader_fee(tx_rec)?;
        Ok(Some(Command::AllAccept(atom)))
    }

    fn get_multi_shard_atom_with_leader_fee(
        &self,
        tx_rec: &TransactionPoolRecord,
    ) -> Result<MultiShardAtom, HotStuffError> {
        let mut atom = tx_rec.get_current_multi_shard_atom();
        if tx_rec.current_decision().is_commit() {
            let num_involved_shard_groups = tx_rec.evidence().num_shard_groups();
            let involved = NonZeroU64::new(num_involved_shard_groups as u64).ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "PROPOSE: Transaction {} involves zero shard groups",
                    tx_rec.id(),
                ))
            })?;
            let leader_fee = tx_rec.calculate_leader_fee(involved);
            atom.leader_fee = Some(leader_fee);
        }
        Ok(atom)
    }

    fn execute_transaction<TTx: StateStoreReadTransaction>(
        &self,
        substate_store: &PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        state_anchor: &LeafBlock,
        transaction: TransactionRecord,
        change_set: &ProposedBlockChangeSet,
        locked_epoch: LockedEpoch,
    ) -> Result<TransactionExecution, HotStuffError> {
        let tx = substate_store.read_transaction();
        // Should have been executed already if all inputs are local
        if let Some(execution) =
            BlockTransactionExecution::get_pending_for_block(tx, transaction.id(), state_anchor).optional()?
        {
            info!(
                target: LOG_TARGET,
                "👨‍🔧 PROPOSE: Using existing transaction execution {} ({})",
                transaction.id(), execution.decision(),
            );
            return Ok(execution.into_transaction_execution());
        }

        let mut pledged = PledgedTransaction::load_pledges(tx, state_anchor, transaction)?;
        let transaction_id = *pledged.id();
        pledged
            .foreign_pledges
            .extend(change_set.get_foreign_pledges(&transaction_id).cloned());

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PROPOSE: Executing transaction {} (pledges: {} local, {} foreign)",
            pledged.id(), pledged.local_pledges.len(), pledged.foreign_pledges.len(),
        );

        self.transaction_manager
            .execute(
                substate_store,
                local_committee_info,
                locked_epoch,
                state_anchor.epoch,
                pledged,
            )
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))
    }
}

#[derive(Default)]
struct ProposalBatch {
    pub foreign_proposals: Vec<ForeignProposal>,
    pub transactions: Vec<TransactionPoolRecord>,
    pub commands: Vec<Command>,
}

impl Display for ProposalBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} transaction(s), {} foreign proposal(s), {} command(s)",
            self.transactions.len(),
            self.foreign_proposals.len(),
            self.commands.len()
        )
    }
}

/// Subtract `by` from the remaining block weight budget, returning `None` once the budget is
/// exhausted (i.e. drops to zero or would underflow). Mirrors the previous count-based helper but
/// operates on the weight budget.
fn subtract_weight_checked(remaining_weight: Option<u64>, by: u64) -> Option<u64> {
    remaining_weight.and_then(|w| w.checked_sub(by)).filter(|w| *w > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod subtract_weight_checked {
        use super::*;

        #[test]
        fn it_subtracts_within_budget() {
            assert_eq!(subtract_weight_checked(Some(100), 40), Some(60));
        }

        #[test]
        fn it_returns_none_when_exhausted_or_underflowing() {
            // Reaching exactly zero exhausts the budget.
            assert_eq!(subtract_weight_checked(Some(100), 100), None);
            // Underflow (subtracting more than remaining) also exhausts it.
            assert_eq!(subtract_weight_checked(Some(40), 100), None);
            // Once None, it stays None.
            assert_eq!(subtract_weight_checked(None, 1), None);
        }
    }
}
