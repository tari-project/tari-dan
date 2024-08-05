//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    num::NonZeroU64,
};

use indexmap::IndexMap;
use log::*;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_crypto::tari_utilities::epoch_time::EpochTime;
use tari_dan_common_types::{
    committee::{Committee, CommitteeInfo},
    optional::Optional,
    shard::Shard,
    Epoch,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        Command,
        ExecutedTransaction,
        ForeignProposal,
        ForeignSendCounters,
        HighQc,
        LastProposed,
        LeafBlock,
        LockedBlock,
        PendingShardStateTreeDiff,
        QuorumCertificate,
        SubstateChange,
        SubstateLockFlag,
        TransactionPool,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
        VersionedSubstateIdLockIntent,
    },
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::TransactionId;

use crate::{
    hotstuff::{
        calculate_state_merkle_root,
        error::HotStuffError,
        substate_store::PendingSubstateStore,
        HotstuffConfig,
        EXHAUST_DIVISOR,
    },
    messages::{HotstuffMessage, ProposalMessage},
    traits::{
        BlockTransactionExecutor,
        ConsensusSpec,
        OutboundMessaging,
        ValidatorSignatureService,
        WriteableSubstateStore,
    },
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_local_propose";

pub struct OnPropose<TConsensusSpec: ConsensusSpec> {
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    transaction_executor: TConsensusSpec::TransactionExecutor,
    signing_service: TConsensusSpec::SignatureService,
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
        transaction_executor: TConsensusSpec::TransactionExecutor,
        signing_service: TConsensusSpec::SignatureService,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
    ) -> Self {
        Self {
            config,
            store,
            epoch_manager,
            transaction_pool,
            transaction_executor,
            signing_service,
            outbound_messaging,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle(
        &mut self,
        epoch: Epoch,
        local_committee: &Committee<TConsensusSpec::Addr>,
        leaf_block: LeafBlock,
        is_newview_propose: bool,
        propose_epoch_end: bool,
    ) -> Result<(), HotStuffError> {
        if let Some(last_proposed) = self.store.with_read_tx(|tx| LastProposed::get(tx)).optional()? {
            if last_proposed.epoch == leaf_block.epoch && last_proposed.height > leaf_block.height {
                // is_newview_propose means that a NEWVIEW has reached quorum and nodes are expecting us to propose.
                // Re-broadcast the previous proposal
                if is_newview_propose {
                    if let Some(next_block) = self.store.with_read_tx(|tx| last_proposed.get_block(tx)).optional()? {
                        info!(
                            target: LOG_TARGET,
                            "🌿 RE-BROADCASTING local block {}({}) to {} validators. {} command(s), justify: {} ({}), parent: {}",
                            next_block.id(),
                            next_block.height(),
                            local_committee.len(),
                            next_block.commands().len(),
                            next_block.justify().block_id(),
                            next_block.justify().block_height(),
                            next_block.parent(),
                        );
                        self.broadcast_local_proposal(next_block, local_committee).await?;
                        return Ok(());
                    }
                }

                info!(
                    target: LOG_TARGET,
                    "⤵️ SKIPPING propose for {} because we already proposed block {}",
                    leaf_block,
                    last_proposed,
                );

                return Ok(());
            }
        }

        let validator = self.epoch_manager.get_our_validator_node(epoch).await?;
        let local_committee_info = self.epoch_manager.get_local_committee_info(epoch).await?;
        let (current_base_layer_block_height, current_base_layer_block_hash) =
            self.epoch_manager.current_base_layer_block_info().await?;

        let base_layer_block_hash = current_base_layer_block_hash;
        let base_layer_block_height = current_base_layer_block_height;

        let next_block = self.store.with_write_tx(|tx| {
            let high_qc = HighQc::get(&**tx)?;
            let high_qc_cert = high_qc.get_quorum_certificate(&**tx)?;
            let (next_block, executed_transactions) = self.build_next_block(
                tx,
                epoch,
                &leaf_block,
                high_qc_cert,
                validator.public_key,
                &local_committee_info,
                // TODO: This just avoids issues with proposed transactions causing leader failures. Not sure if this
                //       is a good idea.
                is_newview_propose,
                base_layer_block_height,
                base_layer_block_hash,
                propose_epoch_end,
            )?;

            // Add executions for this block
            debug!(
                target: LOG_TARGET,
                "Adding {} executed transaction(s) to block {}",
                executed_transactions.len(),
                next_block.id()
            );
            for mut executed in executed_transactions.into_values() {
                // TODO: This is a hacky workaround, if the executed transaction has no shards after execution, we
                // remove it from the pool so that it does not get proposed again. Ideally we should be
                // able to catch this in transaction validation and propose ABORT.
                if local_committee_info.count_distinct_shard_groups(executed.involved_addresses_iter()) == 0 {
                    self.transaction_pool.remove(tx, *executed.id())?;
                    executed
                        .set_abort("Transaction has no involved shards after execution")
                        .update(tx)?;
                } else {
                    executed
                        .into_execution_for_block(*next_block.id())
                        .insert_if_required(tx)?;
                }
            }

            next_block.as_last_proposed().set(tx)?;
            Ok::<_, HotStuffError>(next_block)
        })?;

        info!(
            target: LOG_TARGET,
            "🌿 [{}] PROPOSING new local block {} to {} validators. justify: {} ({}), parent: {}",
            validator.address,
            next_block,
            local_committee.len(),
            next_block.justify().block_id(),
            next_block.justify().block_height(),
            next_block.parent()
        );

        self.broadcast_local_proposal(next_block, local_committee).await?;

        Ok(())
    }

    pub async fn broadcast_local_proposal(
        &mut self,
        next_block: Block,
        local_committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🌿 Broadcasting local proposal {} to {} local committees",
            next_block,
            local_committee.len(),
        );

        // Broadcast to local and foreign committees
        self.outbound_messaging
            .multicast(
                local_committee.iter().map(|(addr, _)| addr),
                HotstuffMessage::Proposal(ProposalMessage {
                    block: next_block.clone(),
                }),
            )
            .await?;

        Ok(())
    }

    /// Executes the given transaction.
    /// If the transaction has already been executed it will be re-executed.
    fn execute_transaction(
        &self,
        store: &PendingSubstateStore<TConsensusSpec::StateStore>,
        current_epoch: Epoch,
        transaction_id: &TransactionId,
    ) -> Result<ExecutedTransaction, HotStuffError> {
        let transaction = TransactionRecord::get(store.read_transaction(), transaction_id)?;

        // TODO: check the failure cases for this. Some failures should not cause consensus to fail
        let executed = self
            .transaction_executor
            .execute(transaction.into_transaction(), store, current_epoch)
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        Ok(executed)
    }

    /// Returns Ok(None) if the command cannot be sequenced yet due to lock conflicts.
    #[allow(clippy::too_many_lines)]
    fn transaction_pool_record_to_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        parent_block: &LeafBlock,
        mut tx_rec: TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        executed_transactions: &mut HashMap<TransactionId, ExecutedTransaction>,
    ) -> Result<Option<Command>, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "👨‍🔧 PROPOSE: Executing transaction {}",
            tx_rec.transaction_id(),
        );

        if tx_rec.current_stage().is_new() {
            let executed = self.execute_transaction(substate_store, parent_block.epoch(), tx_rec.transaction_id())?;
            // Update the decision so that we can propose it
            tx_rec.set_local_decision(executed.decision());
            tx_rec.set_evidence(executed.to_initial_evidence());
            tx_rec.set_transaction_fee(executed.transaction_fee());
            executed_transactions.insert(*executed.id(), executed);
        }

        let num_involved_shard_groups =
            local_committee_info.count_distinct_shard_groups(tx_rec.evidence().substate_addresses_iter());

        if num_involved_shard_groups == 0 {
            warn!(
                target: LOG_TARGET,
                "Transaction {} has no involved shards, skipping...",
                tx_rec.transaction_id(),
            );

            return Ok(None);
        }

        // If the transaction is local only, propose LocalOnly. If the transaction is not new, it must have been
        // previously prepared in a multi-shard command (TBD if that a valid thing to do).
        if num_involved_shard_groups == 1 && !tx_rec.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "Transaction {} is local only but was not previously proposed as such. It is in stage {}",
                tx_rec.transaction_id(),
                tx_rec.current_stage(),
            )
        }

        // LOCAL-ONLY
        if num_involved_shard_groups == 1 && tx_rec.current_stage().is_new() {
            info!(
                target: LOG_TARGET,
                "🏠️ Transaction {} is local only, proposing LocalOnly",
                tx_rec.transaction_id(),
            );
            let involved = NonZeroU64::new(num_involved_shard_groups as u64).expect("involved is 1");
            let leader_fee = tx_rec.calculate_leader_fee(involved, EXHAUST_DIVISOR);
            let tx_atom = tx_rec.get_final_transaction_atom(leader_fee);
            if tx_atom.decision.is_commit() {
                let transaction = executed_transactions.get(tx_rec.transaction_id()).ok_or_else(|| {
                    HotStuffError::InvariantError(format!(
                        "Transaction {} has not been executed when proposing",
                        tx_rec.transaction_id(),
                    ))
                })?;
                let objects = transaction.resolved_inputs().iter().cloned().chain(
                    transaction
                        .resulting_outputs()
                        .iter()
                        .map(|id| VersionedSubstateIdLockIntent::new(id.clone(), SubstateLockFlag::Output)),
                );
                if let Err(err) = substate_store.try_lock_all(*transaction.id(), objects, true) {
                    warn!(
                        target: LOG_TARGET,
                        "🔒 Transaction {} cannot be locked for LocalOnly: {}. Proposing to ABORT...",
                        tx_rec.transaction_id(),
                        err,
                    );
                    // Only error if it is not related to lock errors
                    let _err = err.ok_or_fatal_error()?;
                    // If the transaction does not lock, we propose to abort it
                    return Ok(Some(Command::LocalOnly(tx_atom.abort())));
                }

                let diff = transaction.result().finalize.result.accept().ok_or_else(|| {
                    HotStuffError::InvariantError(format!(
                        "Transaction {} has COMMIT decision but execution failed when proposing",
                        tx_rec.transaction_id(),
                    ))
                })?;
                if let Err(err) = substate_store.put_diff(*tx_rec.transaction_id(), diff) {
                    warn!(
                        target: LOG_TARGET,
                        "🔒 Transaction {} cannot be locked for LocalOnly: {}. Proposing to ABORT...",
                        tx_rec.transaction_id(),
                        err,
                    );
                    // Only error if it is not related to lock errors
                    let _err = err.ok_or_fatal_error()?;
                    // If the transaction does not lock, we propose to abort it
                    return Ok(Some(Command::LocalOnly(tx_atom.abort())));
                }
            }
            return Ok(Some(Command::LocalOnly(tx_atom)));
        }

        match tx_rec.current_stage() {
            // If the transaction is New, propose to Prepare it
            TransactionPoolStage::New => {
                if tx_rec.current_local_decision().is_commit() {
                    let transaction = executed_transactions.get(tx_rec.transaction_id()).ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Transaction {} has not been executed when proposing",
                            tx_rec.transaction_id(),
                        ))
                    })?;

                    let objects = transaction.resolved_inputs().iter().cloned().chain(
                        transaction
                            .resulting_outputs()
                            .iter()
                            .map(|id| VersionedSubstateIdLockIntent::new(id.clone(), SubstateLockFlag::Output)),
                    );
                    if let Err(err) = substate_store.try_lock_all(*transaction.id(), objects, false) {
                        warn!(
                            target: LOG_TARGET,
                            "🔒 Transaction {} cannot be locked for Prepare: {}. Proposing to ABORT...",
                            tx_rec.transaction_id(),
                            err,
                        );
                        // Only error if it is not related to lock errors
                        let _err = err.ok_or_fatal_error()?;
                        // If the transaction does not lock, we should propose to abort it
                        return Ok(Some(Command::Prepare(tx_rec.get_local_transaction_atom().abort())));
                    }
                }
                Ok(Some(Command::Prepare(tx_rec.get_local_transaction_atom())))
            },
            // The transaction is Prepared, this stage is only _ready_ once we know that all local nodes
            // accepted Prepared so we propose LocalPrepared
            TransactionPoolStage::Prepared => Ok(Some(Command::LocalPrepared(tx_rec.get_local_transaction_atom()))),
            // The transaction is LocalPrepared, meaning that we know that all foreign and local nodes have
            // prepared. We can now propose to Accept it. We also propose the decision change which everyone
            // should agree with if they received the same foreign LocalPrepare.
            TransactionPoolStage::LocalPrepared => {
                let involved = NonZeroU64::new(num_involved_shard_groups as u64).ok_or_else(|| {
                    HotStuffError::InvariantError(format!(
                        "Number of involved shards is zero for transaction {}",
                        tx_rec.transaction_id(),
                    ))
                })?;
                let leader_fee = tx_rec.calculate_leader_fee(involved, EXHAUST_DIVISOR);
                let tx_atom = tx_rec.get_final_transaction_atom(leader_fee);
                if tx_atom.decision.is_commit() {
                    let execution = tx_rec
                        .get_execution_for_block(tx, parent_block.block_id())
                        .optional()?
                        .ok_or_else(|| {
                            HotStuffError::InvariantError(format!(
                                "Transaction {} is committed but has no result when proposing",
                                tx_rec.transaction_id(),
                            ))
                        })?;

                    let diff = execution.result().finalize.accept().ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Transaction {} has COMMIT decision but execution failed when proposing",
                            tx_rec.transaction_id(),
                        ))
                    })?;
                    substate_store.put_diff(*tx_rec.transaction_id(), diff)?;
                }
                Ok(Some(Command::Accept(tx_atom)))
            },
            // Not reachable as there is nothing to propose for these stages. To confirm that all local nodes
            // agreed with the Accept, more (possibly empty) blocks with QCs will be
            // proposed and accepted, otherwise the Accept block will not be committed.
            TransactionPoolStage::AllPrepared |
            TransactionPoolStage::SomePrepared |
            TransactionPoolStage::LocalOnly => {
                unreachable!(
                    "It is invalid for TransactionPoolStage::{} to be ready to propose",
                    tx_rec.current_stage()
                )
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    fn build_next_block(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        epoch: Epoch,
        parent_block: &LeafBlock,
        high_qc: QuorumCertificate,
        proposed_by: PublicKey,
        local_committee_info: &CommitteeInfo,
        dont_propose_transactions: bool,
        base_layer_block_height: u64,
        base_layer_block_hash: FixedHash,
        propose_epoch_end: bool,
    ) -> Result<(Block, HashMap<TransactionId, ExecutedTransaction>), HotStuffError> {
        // TODO: Configure
        const TARGET_BLOCK_SIZE: usize = 500;
        let batch = if dont_propose_transactions || propose_epoch_end {
            vec![]
        } else {
            self.transaction_pool.get_batch_for_next_block(tx, TARGET_BLOCK_SIZE)?
        };
        let next_height = parent_block.height() + NodeHeight(1);

        let mut total_leader_fee = 0;
        let locked_block = LockedBlock::get(tx)?;
        let pending_proposals = ForeignProposal::get_all_pending(tx, locked_block.block_id(), parent_block.block_id())?;
        let mut commands = if propose_epoch_end {
            BTreeSet::from_iter([Command::EndEpoch])
        } else {
            ForeignProposal::get_all_new(tx)?
                .into_iter()
                .filter(|foreign_proposal| {
                    // If the proposal base layer height is too high, ignore for now.
                    foreign_proposal.base_layer_block_height <= base_layer_block_height &&
                        // If the foreign proposal is already pending, don't propose it again
                        !pending_proposals.iter().any(|pending_proposal| {
                            pending_proposal.shard_group == foreign_proposal.shard_group &&
                                pending_proposal.block_id == foreign_proposal.block_id
                        })
                })
                .map(|mut foreign_proposal| {
                    foreign_proposal.set_proposed_height(parent_block.height().saturating_add(NodeHeight(1)));
                    Command::ForeignProposal(foreign_proposal)
                })
                .collect()
        };

        // batch is empty for is_empty, is_epoch_end and is_epoch_start blocks
        let mut substate_store = PendingSubstateStore::new(tx, *parent_block.block_id(), self.config.num_preshards);
        let mut executed_transactions = HashMap::new();
        for transaction in batch {
            if let Some(command) = self.transaction_pool_record_to_command(
                tx,
                parent_block,
                transaction,
                local_committee_info,
                &mut substate_store,
                &mut executed_transactions,
            )? {
                total_leader_fee += command
                    .committing()
                    .and_then(|tx| tx.leader_fee.as_ref())
                    .map(|f| f.fee)
                    .unwrap_or(0);
                commands.insert(command);
            }
        }

        debug!(
            target: LOG_TARGET,
            "command(s) for next block: [{}]",
            commands.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",")
        );

        let pending_tree_diffs = PendingShardStateTreeDiff::get_all_up_to_commit_block(tx, high_qc.block_id())?;

        let (state_root, _) = calculate_state_merkle_root(
            tx,
            local_committee_info.shard_group(),
            pending_tree_diffs,
            substate_store
                .diff()
                .iter()
                .filter(|ch| local_committee_info.shard_group().contains(&ch.shard())),
        )?;

        let non_local_shards = get_non_local_shards(substate_store.diff(), local_committee_info);

        let foreign_counters = ForeignSendCounters::get_or_default(tx, parent_block.block_id())?;
        let mut foreign_indexes = non_local_shards
            .iter()
            .map(|shard| (*shard, foreign_counters.get_count(*shard) + 1))
            .collect::<IndexMap<_, _>>();

        // Ensure that foreign indexes are canonically ordered
        foreign_indexes.sort_keys();

        let mut next_block = Block::new(
            self.config.network,
            *parent_block.block_id(),
            high_qc,
            next_height,
            epoch,
            local_committee_info.shard_group(),
            proposed_by,
            commands,
            state_root,
            total_leader_fee,
            foreign_indexes,
            None,
            EpochTime::now().as_u64(),
            base_layer_block_height,
            base_layer_block_hash,
        );

        let signature = self.signing_service.sign(next_block.id());
        next_block.set_signature(signature);

        Ok((next_block, executed_transactions))
    }
}

pub fn get_non_local_shards(diff: &[SubstateChange], local_committee_info: &CommitteeInfo) -> HashSet<Shard> {
    diff.iter()
        .map(|ch| {
            ch.versioned_substate_id()
                .to_substate_address()
                .to_shard(local_committee_info.num_shards())
        })
        .filter(|shard| local_committee_info.shard_group().contains(shard))
        .collect()
}
