//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashMap},
    fmt::Display,
    num::NonZeroU64,
    time::Instant,
};

use log::*;
use ootle_byte_type::ToByteType;
use tari_common_types::types::FixedHash;
use tari_consensus_types::{Decision, HighPc, HighestSeenBlock, LeafBlock, ProposalCertificate, TimeoutCertificate};
use tari_crypto::tari_utilities::epoch_time::EpochTime;
use tari_engine_types::commit_result::RejectReason;
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    NodeHeight,
    ProtocolVersion,
    committee::CommitteeInfo,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{
        Block,
        BlockHeader,
        BlockTransactionExecution,
        BookkeepingModel,
        Command,
        EndEpochAtom,
        EvictNodeAtom,
        ForeignProposal,
        ForeignProposalRecord,
        LockedEpoch,
        PendingShardStateTreeDiff,
        TransactionAtom,
        TransactionExecution,
        TransactionPool,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
        ValidatorConsensusStats,
    },
};
use tari_ootle_transaction::TransactionId;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::task;

use crate::{
    hotstuff::{
        HotstuffConfig,
        apply_leader_fee_to_substate_store,
        block_change_set::ProposedBlockChangeSet,
        calculate_state_merkle_root,
        epoch_state::EpochState,
        error::HotStuffError,
        filter_diff_for_committee,
        foreign_proposal_processor::process_foreign_block,
        process_newly_justified_block,
        substate_store::PendingSubstateStore,
        transaction_manager::{
            ConsensusTransactionManager,
            EvidenceOrExecution,
            LocalPreparedTransaction,
            PledgedTransaction,
            PreparedTransaction,
            TransactionLockConflicts,
        },
    },
    messages::{HotstuffMessage, ProposalMessage},
    tracing::TraceTimer,
    traits::{CertificateStore, ConsensusSpec, OutboundMessaging, ValidatorSignerService, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_propose";

struct NextBlock {
    block: Block,
    foreign_proposals: Vec<ForeignProposal>,
    executed_transactions: HashMap<TransactionId, TransactionExecution>,
    lock_conflicts: TransactionLockConflicts,
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
    ) -> Result<(), HotStuffError> {
        let epoch = epoch_state.epoch();
        let local_committee_info = *epoch_state.local_committee_info();
        let local_committee = epoch_state.local_committee();
        let _timer = TraceTimer::info(LOG_TARGET, "OnPropose");

        // The EndEpoch command commits the committee to the *next* epoch's boundary hash. Only propose
        // it once our (lagged, reorg-stable) oracle has actually observed that epoch — otherwise we have
        // no hash to commit to and would be locking a value the committee hasn't ratified. If the oracle
        // hasn't crossed the boundary yet, defer the EOE proposal; a later round proposes it once the
        // boundary block is scanned.
        let next_epoch_hash = if propose_epoch_end {
            let next_epoch = epoch + Epoch(1);
            match self.epoch_manager.get_epoch_hash(next_epoch).await.optional()? {
                Some(hash) => Some(hash),
                None => {
                    info!(
                        target: LOG_TARGET,
                        "⏳ Not proposing EndEpoch for {epoch}: oracle has not yet observed the boundary block for {next_epoch}"
                    );
                    None
                },
            }
        } else {
            None
        };

        let on_propose = self.clone();

        let (next_block, foreign_proposals) = task::spawn_blocking(move || {
            on_propose.store.with_write_tx(|tx| {
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
                    next_epoch_hash,
                )?;

                let NextBlock {
                    block: next_block,
                    foreign_proposals,
                    executed_transactions,
                    lock_conflicts,
                } = next_block;

                lock_conflicts.save_for_block(tx, next_block.id())?;

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
    fn transaction_pool_record_to_command<TTx: StateStoreReadTransaction>(
        &self,
        start_of_chain_id: &LeafBlock,
        locked_epoch: &LockedEpoch,
        pool_tx: TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        change_set: &ProposedBlockChangeSet,
        substate_store: &mut PendingSubstateStore<TTx>,
        executed_transactions: &mut HashMap<TransactionId, TransactionExecution>,
        lock_conflicts: &mut TransactionLockConflicts,
    ) -> Result<Option<Command>, HotStuffError> {
        match pool_tx.current_stage() {
            TransactionPoolStage::New => self.prepare_transaction(
                start_of_chain_id,
                locked_epoch,
                pool_tx,
                local_committee_info,
                change_set,
                substate_store,
                executed_transactions,
                lock_conflicts,
            ),
            // Leader thinks all foreign PREPARE pledges have been received (condition for LocalPrepared stage to be
            // ready)
            TransactionPoolStage::LocalPrepared => self.local_accept_transaction(
                start_of_chain_id,
                local_committee_info,
                change_set,
                pool_tx,
                substate_store,
                executed_transactions,
            ),

            // Leader thinks that all foreign ACCEPT pledges have been received and, we are ready to accept the result
            // (COMMIT/ABORT)
            TransactionPoolStage::LocalAccepted => {
                self.accept_transaction(start_of_chain_id, &pool_tx, local_committee_info, substate_store)
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
        // `Some(hash)` if we are proposing an end-of-epoch block; carries the next epoch's boundary hash.
        end_epoch_hash: Option<FixedHash>,
    ) -> Result<NextBlock, HotStuffError> {
        let high_qc_id = high_qc_certificate.calculate_id();
        let justify_block = Block::get_justified_block(tx, &high_qc_certificate, epoch)?;
        let start_of_chain_block = highest_seen_block;
        let parent_block = dummy_block.unwrap_or_else(|| highest_seen_block.as_leaf());
        let highest_seen_block = Block::get(tx, highest_seen_block.block_id())?;
        let is_end_of_epoch_in_chain = highest_seen_block.is_epoch_end_proposed_in_chain(tx)?;

        let should_not_propose_commands = is_end_of_epoch_in_chain || end_epoch_hash.is_some() || {
            // TODO: prevent proposers from proposing transactions after an epoch end command is in the justified
            // pending chain, regardless of whether we see the end of epoch or not (race condition).
            // If the last justified/parent block is an epoch end block, we dont propose commands since the block will
            // be rejected
            highest_seen_block.is_epoch_end()
        };

        let mut total_leader_fee = 0u64;
        // When filling a timeout gap with a dummy chain, the candidate effectively extends from justify_block (the
        // dummies are empty blocks that carry justify_block's accumulated_data and state forward — see
        // `calculate_last_dummy_block`). Anchor accumulated_data, the substate store, and the pending state tree
        // diff lookup at justify_block to match what validators recompute from the reconstructed dummy chain.
        // Otherwise speculative state and leader-fee burn that accumulated on a locally-stored fork above the high QC
        // would be incorrectly carried into the new candidate and validators would reject with either an
        // exhaust-burn mismatch or a state Merkle-root mismatch.
        let state_anchor_leaf = if dummy_block.is_some() {
            justify_block.as_leaf()
        } else {
            start_of_chain_block.as_leaf()
        };
        let mut accumulated_data = if dummy_block.is_some() {
            *justify_block.header().accumulated_data()
        } else {
            *highest_seen_block.header().accumulated_data()
        };

        let mut substate_store =
            PendingSubstateStore::new(tx, state_anchor_leaf, self.config.consensus_constants.num_preshards);

        let mut executed_transactions = HashMap::new();

        let batch = if should_not_propose_commands {
            ProposalBatch::default()
        } else {
            self.fetch_next_proposal_batch(tx, local_committee_info, start_of_chain_block)?
        };
        debug!(target: LOG_TARGET, "🌿 PROPOSE: {} (justify: {}) {batch}", highest_seen_block.height(), justify_block.height());

        let mut commands = if is_end_of_epoch_in_chain {
            BTreeSet::from_iter([])
        } else if let Some(next_epoch_hash) = end_epoch_hash {
            BTreeSet::from_iter([Command::EndEpoch(EndEpochAtom::new(next_epoch_hash))])
        } else {
            BTreeSet::from_iter(
                batch
                    .foreign_proposals
                    .iter()
                    .map(|fp| Command::ForeignProposal(fp.to_atom()))
                    .chain(
                        batch
                            .evict_nodes
                            .into_iter()
                            .map(|public_key| Command::EvictNode(EvictNodeAtom { public_key })),
                    ),
            )
        };

        // NOTE: the block for the change set is not used.
        let mut change_set = ProposedBlockChangeSet::new(start_of_chain_block.as_leaf());

        // No need to include evidence from justified block if no transactions are included in the next block
        if !batch.transactions.is_empty() {
            // A replica evaluates this block as: the newly justified block, then the commands in block order
            // (foreign proposals first, see `Command`'s ordering), all against a single change set. The
            // commands generated below must be derived from that same sequence, or the proposer commits to
            // an atom no replica can reproduce and the block is unvotable.
            // TODO: we dont need to process transactions here that are not in the batch
            process_newly_justified_block(tx, &justify_block, high_qc_id, local_committee_info, &mut change_set)?;

            for fp in &batch.foreign_proposals {
                if let Err(err) = process_foreign_block(
                    tx,
                    &high_qc_certificate.as_leaf_block(),
                    fp,
                    local_committee_info,
                    &mut substate_store,
                    &mut change_set,
                ) {
                    warn!(
                        target: LOG_TARGET,
                        "Failed to process foreign proposal: {}. Not proposing...",
                        err
                    );
                    // TODO: should mark as invalid?
                    continue;
                }
            }

            // Add all (ABORT) executions that may have resulted from foreign proposals
            executed_transactions.extend(change_set.take_transaction_executions());
        }

        let locked_epoch = LockedEpoch::new(
            highest_seen_block.epoch(),
            highest_seen_block.epoch_hash().into_array().into(),
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
            // Capture before the record is moved. The processing work below (incl. execution) is incurred
            // whether or not a command is produced, so accumulate for every processed transaction.
            executed_weight += transaction.proposal_weight();
            num_executed += 1;
            let tx_id = *transaction.id();
            // Entries already present (e.g. foreign-proposal aborts from the change set) were not executed
            // for this block, so only count executions newly produced by the command conversion below.
            let had_execution = executed_transactions.contains_key(&tx_id);
            let maybe_command = self.transaction_pool_record_to_command(
                &start_of_chain_block.as_leaf(),
                // This locked epoch is used to set the transaction LockedEpoch if necessary
                &locked_epoch,
                transaction,
                local_committee_info,
                &change_set,
                &mut substate_store,
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
                            .and_then(|tx| tx.leader_fee.as_ref())
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
                // TODO: a BTreeSet changes the order from the original batch. Uncertain if this is a problem since the
                // proposer also processes transactions in the completed block order, however on_propose does perform
                // some operations (e.g. prepare, execute) in batch order. To ensure correctness, we should process
                // on_propose in canonical order.
                commands.insert(command);
            }
        }
        timer.done();

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

        let mut header = BlockHeader::create_unsigned(
            self.config.network,
            ProtocolVersion::at(self.config.network, epoch),
            *parent_block.block_id(),
            high_qc_id,
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
            ExtraData::new(),
        )?;

        let signature = self.signing_service.sign(&header);
        header.set_signature(signature.to_byte_type());

        let next_block = Block::new(header, high_qc_certificate, commands, propose_high_tc);

        Ok(NextBlock {
            block: next_block,
            foreign_proposals: batch.foreign_proposals,
            executed_transactions,
            lock_conflicts,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn fetch_next_proposal_batch<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        local_committee_info: &CommitteeInfo,
        start_of_chain_block: HighestSeenBlock,
    ) -> Result<ProposalBatch, HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "fetch_next_proposal_batch");
        // A block is budgeted by total command weight (`max_block_weight`), not a flat command count.
        // Foreign proposals and evict nodes consume part of that budget before local transactions fill the
        // rest. A foreign proposal is weighted by the substate pledges it carries (the dominant processing
        // cost when applying it at propose time), on the same scale as transaction input weight, rather
        // than a flat 10x multiplier.
        const MAX_FOREIGN_PROPOSALS_PER_BLOCK: usize = 10;
        const FP_BASE_WEIGHT: u64 = 50;
        const FP_PLEDGE_WEIGHT: u64 = 15;
        const EVICT_NODE_WEIGHT: u64 = 50;

        let max_block_weight = self.config.consensus_constants.max_block_weight;
        let max_commands = self.config.consensus_constants.max_commands_in_block;

        let foreign_proposals =
            ForeignProposalRecord::get_all_new(tx, start_of_chain_block.block_id(), MAX_FOREIGN_PROPOSALS_PER_BLOCK)?;

        if !foreign_proposals.is_empty() {
            debug!(
                target: LOG_TARGET,
                "🌿 Found {} foreign proposals for next block",
                foreign_proposals.len()
            );
        }

        let foreign_proposal_weight: u64 = foreign_proposals
            .iter()
            .map(|fp| FP_BASE_WEIGHT + fp.block_pledge().len() as u64 * FP_PLEDGE_WEIGHT)
            .sum();

        let mut remaining_weight = subtract_weight_checked(Some(max_block_weight), foreign_proposal_weight);

        let evict_nodes = remaining_weight
            // Disable eviction proposals if not enabled in config
            .filter(|_| self.config.enable_eviction_proposal)
            .map(|remaining| {
                let num_evicted =
                    ValidatorConsensusStats::count_number_evicted_nodes(tx, start_of_chain_block.epoch())?;
                // TODO: technically, we should not evict more than 1/3 of the voting power, not the number of nodes
                // (but this is currently the same thing)
                let max_allowed_to_evict = u64::from(local_committee_info.max_failure_shard_group_members())
                    .saturating_sub(num_evicted)
                    .min(remaining / EVICT_NODE_WEIGHT);
                ValidatorConsensusStats::get_nodes_to_evict(
                    tx,
                    start_of_chain_block.block_id(),
                    self.config.consensus_constants.missed_proposal_evict_threshold,
                    max_allowed_to_evict,
                )
            })
            .transpose()?
            .unwrap_or_default();

        if !evict_nodes.is_empty() {
            debug!(
                target: LOG_TARGET,
                "🌿 Found {} EVICT nodes for next block",
                evict_nodes.len()
            )
        }

        remaining_weight = subtract_weight_checked(remaining_weight, evict_nodes.len() as u64 * EVICT_NODE_WEIGHT);

        // Bound the transaction count so the total command count (foreign proposals + evict + transactions)
        // stays under the hard command cap regardless of how light the transactions are.
        let max_tx_count = max_commands.saturating_sub(foreign_proposals.len() + evict_nodes.len());

        let transactions = remaining_weight
            .filter(|_| max_tx_count > 0)
            .map(|weight_budget| {
                self.transaction_pool.get_batch_for_next_block(
                    tx,
                    weight_budget,
                    max_tx_count,
                    start_of_chain_block.block_id(),
                )
            })
            .transpose()?
            .unwrap_or_default();

        Ok(ProposalBatch {
            foreign_proposals: foreign_proposals.into_iter().map(|fp| fp.into_proposal()).collect(),
            transactions,
            evict_nodes,
            commands: vec![],
        })
    }

    #[allow(clippy::too_many_lines)]
    fn prepare_transaction<TTx: StateStoreReadTransaction>(
        &self,
        parent_block: &LeafBlock,
        locked_epoch: &LockedEpoch,
        mut pool_tx: TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        change_set: &ProposedBlockChangeSet,
        substate_store: &mut PendingSubstateStore<TTx>,
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
            .prepare(
                substate_store,
                local_committee_info,
                &pool_tx,
                *parent_block,
                change_set,
            )
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        if prepared.lock_status().is_any_failed() && !prepared.lock_status().is_hard_conflict() {
            warn!(
                target: LOG_TARGET,
                "⚠️ Transaction {} has lock conflicts, but no hard conflicts. Skipping proposing this transaction...",
                pool_tx.id(),
            );

            lock_conflicts.add(*pool_tx.id(), prepared.into_lock_status().into_lock_conflicts());
            return Ok(None);
        }

        match prepared {
            PreparedTransaction::LocalOnly(local) => match *local {
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
                        "🏠️ Transaction {} is local only, proposing LocalOnly",
                        pool_tx.id(),
                    );

                    if pool_tx.current_decision().is_commit() {
                        // STRICT INVARIANT: a LocalOnly transaction involves exactly the local shard group, so its
                        // leader fee is calculated with one involved shard group and it accounts for the entire
                        // exhaust burn.
                        if pool_tx.evidence().num_shard_groups() != 1 {
                            return Err(HotStuffError::InvariantError(format!(
                                "PROPOSE: LocalOnly transaction {} has evidence for {} shard groups",
                                pool_tx.id(),
                                pool_tx.evidence().num_shard_groups(),
                            )));
                        }
                        let involved = NonZeroU64::new(1).expect("1 > 0");
                        let leader_fee = pool_tx.calculate_leader_fee(involved);
                        pool_tx.set_leader_fee(leader_fee);
                        let diff = execution.result().finalize.any_accept().ok_or_else(|| {
                            HotStuffError::InvariantError(format!(
                                "prepare_transaction: Transaction {} has COMMIT decision but execution failed when \
                                 proposing",
                                pool_tx.id(),
                            ))
                        })?;

                        if let Err(err) = substate_store.put_diff(diff) {
                            // A lock failure or DOWN input is an expected conflict under contention - skip the
                            // transaction this round. Any other store error is fatal and must be propagated.
                            let lock_err = err.ok_lock_failed()?;
                            warn!(
                                target: LOG_TARGET,
                                "🔒 Skipping LocalOnly transaction {} while proposing due to input conflict: {}",
                                pool_tx.id(),
                                lock_err,
                            );
                            return Ok(None);
                        }
                    }

                    executed_transactions.insert(*pool_tx.id(), execution);

                    let atom = pool_tx.get_current_transaction_atom();
                    Ok(Some(Command::LocalOnly(atom)))
                },
                LocalPreparedTransaction::EarlyAbort { execution } => {
                    info!(
                        target: LOG_TARGET,
                        "⚠️ Transaction is LOCAL-ONLY EARLY ABORT, proposing LocalOnly({}, ABORT)",
                        pool_tx.id()
                    );
                    pool_tx
                        .set_local_decision(execution.decision())
                        .set_transaction_fee(execution.transaction_fee())
                        .set_exhaust_burn(execution.exhaust_burn())
                        .no_leader_fee()
                        .merge_evidence(execution.to_evidence(
                            local_committee_info.num_preshards(),
                            local_committee_info.num_committees(),
                        ));

                    executed_transactions.insert(*pool_tx.id(), execution);
                    let atom = pool_tx.get_current_transaction_atom();
                    Ok(Some(Command::LocalOnly(atom)))
                },
            },

            PreparedTransaction::MultiShard(multishard) => {
                match multishard.into_evidence_or_execution() {
                    EvidenceOrExecution::Execution { execution } => {
                        // CASE: All inputs are local and outputs are foreign (i.e. the transaction is
                        // executed), or all inputs are foreign and this shard
                        // group is output only and, we've already received all pledges.
                        pool_tx.update_from_execution(
                            local_committee_info.num_preshards(),
                            local_committee_info.num_committees(),
                            &execution,
                        );

                        // TODO: this is kinda hacky - we may not be involved in the transaction after ABORT execution,
                        // but this would be invalid so we ensure that we are added to evidence. Ideally, we wouldn't
                        // sequence this transaction at all - investigate.
                        pool_tx
                            .evidence_mut()
                            .add_shard_group(local_committee_info.shard_group());

                        if execution.decision().is_commit() {
                            let involves_inputs = pool_tx.evidence().has_inputs(local_committee_info.shard_group());
                            if !involves_inputs {
                                let num_involved_shard_groups = pool_tx.evidence().num_shard_groups();
                                let involved = NonZeroU64::new(num_involved_shard_groups as u64).ok_or_else(|| {
                                    HotStuffError::InvariantError("Number of involved shard groups is 0".to_string())
                                })?;
                                let leader_fee = pool_tx.calculate_leader_fee(involved);
                                pool_tx.set_leader_fee(leader_fee);
                            }
                        }

                        executed_transactions.insert(*pool_tx.id(), *execution);
                    },
                    EvidenceOrExecution::Evidence { evidence, .. } => {
                        // CASE: All local inputs were resolved. We need to continue with consensus to get the
                        // foreign inputs/outputs.
                        pool_tx.set_local_decision(Decision::Commit)
                            // Set partial evidence using local inputs and known outputs.
                            // NOTE: we could have evidence for initial sequence from foreign proposals, so we must not overwrite it
                            .merge_evidence(evidence);
                    },
                }

                info!(
                    target: LOG_TARGET,
                    "🌍 Transaction involves foreign shard groups, proposing Prepare({}, {})",
                    pool_tx.id(),
                    pool_tx.current_decision(),
                );

                if pool_tx.current_decision().is_abort() {
                    let atom = pool_tx.get_current_transaction_atom();
                    return Ok(Some(Command::LocalAccept(atom)));
                }
                if pool_tx
                    .evidence()
                    .is_committee_output_only(local_committee_info.shard_group())
                {
                    // No prepare phase needed for output-only transactions. All foreign shards have prepared inputs
                    // and the output shard groups need to execute and accept outputs.
                    debug!(
                        target: LOG_TARGET,
                        "ℹ️ Transaction {} is output-only for {}, proposing LocalAccept",
                        pool_tx.id(),
                        local_committee_info.shard_group()
                    );
                    let atom = pool_tx.get_current_transaction_atom();
                    Ok(Some(Command::LocalAccept(atom)))
                } else {
                    let atom = pool_tx.get_current_transaction_atom();
                    Ok(Some(Command::LocalPrepare(atom)))
                }
            },
        }
    }

    fn local_accept_transaction<TTx: StateStoreReadTransaction>(
        &self,
        parent_block: &LeafBlock,
        local_committee_info: &CommitteeInfo,
        change_set: &ProposedBlockChangeSet,
        mut tx_rec: TransactionPoolRecord,
        substate_store: &mut PendingSubstateStore<TTx>,
        executed_transactions: &mut HashMap<TransactionId, TransactionExecution>,
    ) -> Result<Option<Command>, HotStuffError> {
        // Only set to abort if either the local or one or more foreign shards decided to ABORT
        if tx_rec.current_decision().is_abort() {
            return Ok(Some(Command::LocalAccept(tx_rec.get_current_transaction_atom())));
        }

        let locked_epoch = tx_rec.locked_epoch().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "PROPOSE: local_accept_transaction: Transaction {} is in LocalPrepared stage but has no locked epoch",
                tx_rec.id(),
            ))
        })?;

        let tx = substate_store.read_transaction();
        let transaction = tx_rec.get_transaction(tx)?;
        let execution = self.execute_transaction(tx, parent_block, transaction, change_set, locked_epoch.clone())?;

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
            return Ok(Some(Command::LocalAccept(tx_rec.get_current_transaction_atom())));
        }

        tx_rec.update_from_execution(
            local_committee_info.num_preshards(),
            local_committee_info.num_committees(),
            &execution,
        );
        executed_transactions.insert(*tx_rec.id(), execution);
        // If we locally decided to ABORT, we are still saying that we think all prepared and, after execution decide to
        // ABORT. When we enter the acceptance phase, we will propose SomeAccept for this case.
        let atom = self.get_transaction_atom_with_leader_fee(&tx_rec)?;
        Ok(Some(Command::LocalAccept(atom)))
    }

    fn accept_transaction<TTx: StateStoreReadTransaction>(
        &self,
        parent_block: &LeafBlock,
        tx_rec: &TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
    ) -> Result<Option<Command>, HotStuffError> {
        if tx_rec.current_decision().is_abort() {
            return Ok(Some(Command::SomeAccept(tx_rec.get_current_transaction_atom())));
        }

        let tx = substate_store.read_transaction();
        let execution = tx_rec
            .get_pending_execution_for_block(tx, parent_block)
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
        let atom = self.get_transaction_atom_with_leader_fee(tx_rec)?;
        Ok(Some(Command::AllAccept(atom)))
    }

    fn get_transaction_atom_with_leader_fee(
        &self,
        tx_rec: &TransactionPoolRecord,
    ) -> Result<TransactionAtom, HotStuffError> {
        let mut atom = tx_rec.get_current_transaction_atom();
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
        tx: &TTx,
        parent_block: &LeafBlock,
        transaction: TransactionRecord,
        change_set: &ProposedBlockChangeSet,
        locked_epoch: LockedEpoch,
    ) -> Result<TransactionExecution, HotStuffError> {
        // Should have been executed already if all inputs are local
        if let Some(execution) =
            BlockTransactionExecution::get_pending_for_block(tx, transaction.id(), parent_block).optional()?
        {
            info!(
                target: LOG_TARGET,
                "👨‍🔧 PROPOSE: Using existing transaction execution {} ({})",
                transaction.id(), execution.decision(),
            );
            return Ok(execution.into_transaction_execution());
        }

        let mut pledged = PledgedTransaction::load_pledges(tx, transaction)?;
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
            .execute(locked_epoch, pledged)
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))
    }
}

#[derive(Default)]
struct ProposalBatch {
    pub foreign_proposals: Vec<ForeignProposal>,
    pub transactions: Vec<TransactionPoolRecord>,
    pub evict_nodes: Vec<RistrettoPublicKeyBytes>,
    pub commands: Vec<Command>,
}

impl Display for ProposalBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} transaction(s), {} foreign proposal(s), {} evict, {} command(s)",
            self.transactions.len(),
            self.foreign_proposals.len(),
            self.evict_nodes.len(),
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
