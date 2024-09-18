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
    ToSubstateAddress,
    VersionedSubstateId,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        BlockTransactionExecution,
        BurntUtxo,
        Command,
        Decision,
        ForeignProposal,
        ForeignSendCounters,
        HighQc,
        LastProposed,
        LeafBlock,
        LockedBlock,
        PendingShardStateTreeDiff,
        QuorumCertificate,
        SubstateChange,
        SubstateRequirementLockIntent,
        TransactionAtom,
        TransactionExecution,
        TransactionPool,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
    },
    StateStore,
};
use tari_engine_types::{commit_result::RejectReason, substate::Substate};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::TransactionId;
use tokio::task;

use crate::{
    hotstuff::{
        block_change_set::ProposedBlockChangeSet,
        calculate_state_merkle_root,
        error::HotStuffError,
        filter_diff_for_committee,
        substate_store::PendingSubstateStore,
        transaction_manager::{
            ConsensusTransactionManager,
            LocalPreparedTransaction,
            PledgedTransaction,
            PreparedTransaction,
            TransactionLockConflicts,
        },
        HotstuffConfig,
        EXHAUST_DIVISOR,
    },
    messages::{HotstuffMessage, ProposalMessage},
    tracing::TraceTimer,
    traits::{ConsensusSpec, OutboundMessaging, ValidatorSignatureService, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_local_propose";

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
        transaction_manager: ConsensusTransactionManager<
            TConsensusSpec::TransactionExecutor,
            TConsensusSpec::StateStore,
        >,
        signing_service: TConsensusSpec::SignatureService,
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
        epoch: Epoch,
        local_committee: &Committee<TConsensusSpec::Addr>,
        local_committee_info: CommitteeInfo,
        leaf_block: LeafBlock,
        is_newview_propose: bool,
        propose_epoch_end: bool,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::info(LOG_TARGET, "OnPropose");
        if let Some(last_proposed) = self.store.with_read_tx(|tx| LastProposed::get(tx)).optional()? {
            if last_proposed.epoch == leaf_block.epoch && last_proposed.height > leaf_block.height {
                // is_newview_propose means that a NEWVIEW has reached quorum and nodes are expecting us to propose.
                // Re-broadcast the previous proposal
                if is_newview_propose {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Newview propose {leaf_block} but we already proposed block {last_proposed}.",
                    );
                    // if let Some(next_block) = self.store.with_read_tx(|tx| last_proposed.get_block(tx)).optional()? {
                    //     info!(
                    //         target: LOG_TARGET,
                    //         "🌿 RE-BROADCASTING local block {}({}) to {} validators. {} command(s), justify: {} ({}),
                    // parent: {}",         next_block.id(),
                    //         next_block.height(),
                    //         local_committee.len(),
                    //         next_block.commands().len(),
                    //         next_block.justify().block_id(),
                    //         next_block.justify().block_height(),
                    //         next_block.parent(),
                    //     );
                    //     self.broadcast_local_proposal(next_block, local_committee).await?;
                    //     return Ok(());
                    // }
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
        let (current_base_layer_block_height, current_base_layer_block_hash) =
            self.epoch_manager.current_base_layer_block_info().await?;

        let base_layer_block_hash = current_base_layer_block_hash;
        let base_layer_block_height = current_base_layer_block_height;

        let on_propose = self.clone();
        let (next_block, foreign_proposals) = task::spawn_blocking(move || {
            on_propose.store.with_write_tx(|tx| {
                let high_qc = HighQc::get(&**tx, epoch)?;
                let high_qc_cert = high_qc.get_quorum_certificate(&**tx)?;

                info!(
                    target: LOG_TARGET,
                    "🌿 PROPOSE local block with parent {}. HighQC: {}",
                    leaf_block,
                    high_qc_cert,
                );

                let next_block = on_propose.build_next_block(
                    tx,
                    epoch,
                    &leaf_block,
                    high_qc_cert,
                    validator.public_key,
                    &local_committee_info,
                    // TODO: This just avoids issues with proposed transactions causing leader failures. Not sure if
                    // this is a good idea.
                    is_newview_propose,
                    base_layer_block_height,
                    base_layer_block_hash,
                    propose_epoch_end,
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
                for executed in executed_transactions.into_values() {
                    executed.for_block(*next_block.id()).insert_if_required(tx)?;
                }

                next_block.as_last_proposed().set(tx)?;

                Ok::<_, HotStuffError>((next_block, foreign_proposals))
            })
        })
        .await??;

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

        self.broadcast_local_proposal(next_block, foreign_proposals, local_committee)
            .await?;

        Ok(())
    }

    pub async fn broadcast_local_proposal(
        &mut self,
        next_block: Block,
        foreign_proposals: Vec<ForeignProposal>,
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
                    block: next_block,
                    foreign_proposals,
                }),
            )
            .await?;

        Ok(())
    }

    /// Returns Ok(None) if the command cannot be sequenced yet due to lock conflicts.
    fn transaction_pool_record_to_command(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        parent_block: &LeafBlock,
        mut tx_rec: TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        executed_transactions: &mut HashMap<TransactionId, TransactionExecution>,
        lock_conflicts: &mut TransactionLockConflicts,
    ) -> Result<Option<Command>, HotStuffError> {
        match tx_rec.current_stage() {
            TransactionPoolStage::New => self.prepare_transaction(
                parent_block,
                &mut tx_rec,
                local_committee_info,
                substate_store,
                executed_transactions,
                lock_conflicts,
            ),
            // Leader thinks all local nodes have prepared
            TransactionPoolStage::Prepared => Ok(Some(Command::LocalPrepare(tx_rec.get_local_transaction_atom()))),
            // Leader thinks all foreign PREPARE pledges have been received (condition for LocalPrepared stage to be
            // ready)
            TransactionPoolStage::LocalPrepared => self.all_or_some_prepare_transaction(
                tx,
                parent_block,
                local_committee_info,
                &mut tx_rec,
                substate_store,
                executed_transactions,
            ),

            // Leader thinks that all local nodes agree that all shard groups have prepared, we are ready to accept
            // locally
            TransactionPoolStage::AllPrepared => Ok(Some(Command::LocalAccept(
                self.get_transaction_atom_with_leader_fee(local_committee_info, &mut tx_rec)?,
            ))),
            // Leader thinks local nodes are ready to accept an ABORT
            TransactionPoolStage::SomePrepared => Ok(Some(Command::LocalAccept(tx_rec.get_current_transaction_atom()))),
            // Leader thinks that all foreign ACCEPT pledges have been received and, we are ready to accept the result
            // (COMMIT/ABORT)
            TransactionPoolStage::LocalAccepted => {
                self.accept_transaction(tx, parent_block, &mut tx_rec, local_committee_info, substate_store)
            },
            // Not reachable as there is nothing to propose for these stages. To confirm that all local nodes
            // agreed with the Accept, more (possibly empty) blocks with QCs will be
            // proposed and accepted, otherwise the Accept block will not be committed.
            TransactionPoolStage::AllAccepted |
            TransactionPoolStage::SomeAccepted |
            TransactionPoolStage::LocalOnly => {
                unreachable!(
                    "It is invalid for TransactionPoolStage::{} to be ready to propose",
                    tx_rec.current_stage()
                )
            },
        }
    }

    fn process_newly_justified_block(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        new_leaf_block: &Block,
        high_qc: HighQc,
        local_committee_info: &CommitteeInfo,
        change_set: &mut ProposedBlockChangeSet,
    ) -> Result<(), HotStuffError> {
        let locked_block = LockedBlock::get(tx, new_leaf_block.epoch())?;
        info!(
            target: LOG_TARGET,
            "✅ New leaf block {} is justified. Updating evidence for transactions",
            new_leaf_block,
        );

        let leaf = new_leaf_block.as_leaf_block();
        for cmd in new_leaf_block.commands() {
            if !cmd.is_local_prepare() && !cmd.is_local_accept() {
                continue;
            }

            let atom = cmd.transaction().expect("Command must be a transaction");

            let Some(mut pool_tx) = change_set.get_transaction(tx, &locked_block, &leaf, atom.id())? else {
                return Err(HotStuffError::InvariantError(format!(
                    "Transaction {} in newly justified block {} not found in the pool",
                    atom.id(),
                    leaf,
                )));
            };

            if cmd.is_local_prepare() {
                pool_tx.add_prepare_qc_evidence(local_committee_info, high_qc.qc_id);
            } else if cmd.is_local_accept() {
                pool_tx.add_accept_qc_evidence(local_committee_info, high_qc.qc_id);
            } else {
                // Nothing
            }

            debug!(
                target: LOG_TARGET,
                "ON PROPOSE: process_newly_justified_block {} {} {}, QC[{}]",
                pool_tx.transaction_id(),
                pool_tx.current_stage(),
                local_committee_info.shard_group(),
                high_qc.qc_id
            );

            change_set.set_next_transaction_update(pool_tx)?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn build_next_block(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        epoch: Epoch,
        parent_block: &LeafBlock,
        high_qc_certificate: QuorumCertificate,
        proposed_by: PublicKey,
        local_committee_info: &CommitteeInfo,
        dont_propose_transactions: bool,
        base_layer_block_height: u64,
        base_layer_block_hash: FixedHash,
        propose_epoch_end: bool,
    ) -> Result<NextBlock, HotStuffError> {
        // TODO: Configure
        const TARGET_BLOCK_SIZE: usize = 500;

        let next_height = parent_block.height() + NodeHeight(1);

        let mut total_leader_fee = 0;

        let foreign_proposals = if propose_epoch_end {
            vec![]
        } else {
            ForeignProposal::get_all_new(
                tx,
                base_layer_block_height,
                parent_block.block_id(),
                TARGET_BLOCK_SIZE / 4,
            )?
        };

        if !foreign_proposals.is_empty() {
            debug!(
                target: LOG_TARGET,
                "🌿 Found {} foreign proposals for next block",
                foreign_proposals.len()
            );
        }

        let burnt_utxos = if dont_propose_transactions || propose_epoch_end {
            vec![]
        } else {
            TARGET_BLOCK_SIZE
                .checked_sub(foreign_proposals.len() * 4)
                .filter(|n| *n > 0)
                .map(|size| BurntUtxo::get_all_unproposed(tx, parent_block.block_id(), size))
                .transpose()?
                .unwrap_or_default()
        };

        debug!(
            target: LOG_TARGET,
           "🌿 Found {} burnt utxos for next block",
            burnt_utxos.len()
        );

        let batch = if dont_propose_transactions || propose_epoch_end {
            vec![]
        } else {
            TARGET_BLOCK_SIZE
                // Each foreign proposal is "heavier" than a transaction command
                .checked_sub(foreign_proposals.len() * 4 + burnt_utxos.len())
                .filter(|n| *n > 0)
                .map(|size| self.transaction_pool.get_batch_for_next_block(tx, size, parent_block.block_id()))
                .transpose()?
                .unwrap_or_default()
        };

        let mut commands = if propose_epoch_end {
            BTreeSet::from_iter([Command::EndEpoch])
        } else {
            BTreeSet::from_iter(
                foreign_proposals
                    .iter()
                    .map(|fp| Command::ForeignProposal(fp.to_atom()))
                    .chain(
                        burnt_utxos
                            .iter()
                            .map(|bu| Command::MintConfidentialOutput(bu.to_atom())),
                    ),
            )
        };

        let mut change_set = ProposedBlockChangeSet::new(high_qc_certificate.as_leaf_block());

        // No need to include evidence from justified block if no transactions are included in the next block
        if !batch.is_empty() {
            // TODO(protocol-efficiency): We should process any foreign proposals included in this block to include
            // evidence. And that should determine if they are ready. However this is difficult because we
            // get the batch from the database which isnt aware of which foreign proposals we're going to
            // propose. This is why the system currently never proposes foreign proposals affecting a
            // transaction in the same block as LocalPrepare/LocalAccept
            //
            // for fp in foreign_proposals {
            //     process_foreign_block(
            //         tx,
            //         &high_qc_certificate.as_leaf_block(),
            //         locked_block,
            //         fp,
            //         foreign_committee_info,
            //         local_committee_info,
            //         &mut change_set,
            //     )?;
            // }

            let justified_block = high_qc_certificate.get_block(tx)?;
            if !justified_block.is_justified() {
                // TODO: we dont need to process transactions here that are not in the batch
                self.process_newly_justified_block(
                    tx,
                    &justified_block,
                    high_qc_certificate.as_high_qc(),
                    local_committee_info,
                    &mut change_set,
                )?;
            }
        }

        // batch is empty for is_empty, is_epoch_end and is_epoch_start blocks
        let mut substate_store = PendingSubstateStore::new(tx, *parent_block.block_id(), self.config.num_preshards);
        let mut executed_transactions = HashMap::new();
        let timer = TraceTimer::info(LOG_TARGET, "Generating commands").with_iterations(batch.len());
        let mut lock_conflicts = TransactionLockConflicts::new();
        for mut transaction in batch {
            // Apply the transaction updates (if any) that occurred as a result of the justified block.
            // This allows us to propose evidence in the next block that relates to transactions in the justified block.
            change_set.apply_evidence(&mut transaction);
            if let Some(command) = self.transaction_pool_record_to_command(
                tx,
                parent_block,
                transaction,
                local_committee_info,
                &mut substate_store,
                &mut executed_transactions,
                &mut lock_conflicts,
            )? {
                total_leader_fee += command
                    .committing()
                    .and_then(|tx| tx.leader_fee.as_ref())
                    .map(|f| f.fee)
                    .unwrap_or(0);
                commands.insert(command);
            }
        }
        timer.done();

        // This relies on the UTXO commands being ordered after transaction commands
        for utxo in burnt_utxos {
            let id = VersionedSubstateId::new(utxo.substate_id.clone(), 0);
            let shard = id.to_substate_address().to_shard(local_committee_info.num_preshards());
            let change = SubstateChange::Up {
                id,
                shard,
                // N/A
                transaction_id: Default::default(),
                substate: Substate::new(0, utxo.substate_value),
            };

            substate_store.put(change)?;
        }

        debug!(
            target: LOG_TARGET,
            "command(s) for next block: [{}]",
            commands.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",")
        );

        let timer = TraceTimer::info(LOG_TARGET, "Propose calculate state root");
        let pending_tree_diffs = PendingShardStateTreeDiff::get_all_up_to_commit_block(tx, parent_block.block_id())?;

        let (state_root, _) = calculate_state_merkle_root(
            tx,
            local_committee_info.shard_group(),
            pending_tree_diffs,
            substate_store.diff(),
        )?;
        timer.done();

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
            high_qc_certificate,
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
            None,
        );

        let signature = self.signing_service.sign(next_block.id());
        next_block.set_signature(signature);

        Ok(NextBlock {
            block: next_block,
            foreign_proposals,
            executed_transactions,
            lock_conflicts,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn prepare_transaction(
        &self,
        parent_block: &LeafBlock,
        tx_rec: &mut TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        executed_transactions: &mut HashMap<TransactionId, TransactionExecution>,
        lock_conflicts: &mut TransactionLockConflicts,
    ) -> Result<Option<Command>, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "👨‍🔧 PROPOSE: PREPARE transaction {}",
            tx_rec.transaction_id(),
        );

        let prepared = self
            .transaction_manager
            .prepare(
                substate_store,
                local_committee_info,
                parent_block.epoch(),
                *tx_rec.transaction_id(),
                parent_block.block_id(),
            )
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        if prepared.lock_status().is_any_failed() && !prepared.lock_status().is_hard_conflict() {
            warn!(
                target: LOG_TARGET,
                "⚠️ Transaction {} has lock conflicts, but no hard conflicts. Skipping proposing this transaction...",
                tx_rec.transaction_id(),
            );

            lock_conflicts.add(
                *tx_rec.transaction_id(),
                prepared.into_lock_status().into_lock_conflicts(),
            );
            return Ok(None);
        }

        let command = match prepared {
            PreparedTransaction::LocalOnly(LocalPreparedTransaction::Accept { execution, .. }) => {
                // Update the decision so that we can propose it
                tx_rec.update_from_execution(
                    local_committee_info.num_preshards(),
                    local_committee_info.num_committees(),
                    &execution,
                );

                info!(
                    target: LOG_TARGET,
                    "🏠️ Transaction {} is local only, proposing LocalOnly",
                    tx_rec.transaction_id(),
                );

                if tx_rec.current_decision().is_commit() {
                    let involved = NonZeroU64::new(1).expect("1 > 0");
                    let leader_fee = tx_rec.calculate_leader_fee(involved, EXHAUST_DIVISOR);
                    tx_rec.set_leader_fee(leader_fee);
                    let diff = execution.result().finalize.result.accept().ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "prepare_transaction: Transaction {} has COMMIT decision but execution failed when \
                             proposing",
                            tx_rec.transaction_id(),
                        ))
                    })?;

                    if let Err(err) = substate_store.put_diff(*tx_rec.transaction_id(), diff) {
                        error!(
                            target: LOG_TARGET,
                            "🔒 Failed to write to temporary state store for transaction {} for LocalOnly: {}. Skipping proposing this transaction...",
                            tx_rec.transaction_id(),
                            err,
                        );
                        // Only error if it is not related to lock errors
                        let _err = err.ok_lock_failed()?;
                        return Ok(None);
                    }
                }

                executed_transactions.insert(*tx_rec.transaction_id(), execution);

                let atom = tx_rec.get_current_transaction_atom();
                Command::LocalOnly(atom)
            },
            PreparedTransaction::LocalOnly(LocalPreparedTransaction::EarlyAbort { execution }) => {
                info!(
                    target: LOG_TARGET,
                    "⚠️ Transaction is LOCAL-ONLY EARLY ABORT, proposing LocalOnly({}, ABORT)",
                    tx_rec.transaction_id(),
                );
                tx_rec.set_local_decision(Decision::Abort);

                info!(
                    target: LOG_TARGET,
                    "⚠️ Transaction is LOCAL-ONLY EARLY ABORT, proposing LocalOnly({}, ABORT)",
                    tx_rec.transaction_id(),
                );

                tx_rec.update_from_execution(
                    local_committee_info.num_preshards(),
                    local_committee_info.num_committees(),
                    &execution,
                );
                executed_transactions.insert(*tx_rec.transaction_id(), execution);
                let atom = tx_rec.get_current_transaction_atom();
                Command::LocalOnly(atom)
            },

            PreparedTransaction::MultiShard(multishard) => {
                match multishard.current_decision() {
                    Decision::Commit => {
                        if multishard.is_executed() {
                            // CASE: All inputs are local and outputs are foreign (i.e. the transaction is executed), or
                            let execution = multishard.into_execution().expect("Abort should have execution");
                            tx_rec.update_from_execution(
                                local_committee_info.num_preshards(),
                                local_committee_info.num_committees(),
                                &execution,
                            );
                            executed_transactions.insert(*tx_rec.transaction_id(), execution);
                        } else {
                            // CASE: All local inputs were resolved. We need to continue with consensus to get the
                            // foreign inputs/outputs.
                            tx_rec.set_local_decision(Decision::Commit);
                            // Set partial evidence using local inputs and known outputs.
                            tx_rec.evidence_mut().update(&multishard.to_initial_evidence(
                                local_committee_info.num_preshards(),
                                local_committee_info.num_committees(),
                            ));
                        }
                    },
                    Decision::Abort => {
                        // CASE: The transaction was ABORTed due to a lock conflict
                        let execution = multishard.into_execution().expect("Abort must have execution");
                        tx_rec.update_from_execution(
                            local_committee_info.num_preshards(),
                            local_committee_info.num_committees(),
                            &execution,
                        );
                        executed_transactions.insert(*tx_rec.transaction_id(), execution);
                    },
                }

                info!(
                    target: LOG_TARGET,
                    "🌍 Transaction involves foreign shard groups, proposing Prepare({}, {})",
                    tx_rec.transaction_id(),
                    tx_rec.current_decision(),
                );

                let atom = tx_rec.get_local_transaction_atom();
                Command::Prepare(atom)
            },
        };

        Ok(Some(command))
    }

    fn all_or_some_prepare_transaction(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        parent_block: &LeafBlock,
        local_committee_info: &CommitteeInfo,
        tx_rec: &mut TransactionPoolRecord,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
        executed_transactions: &mut HashMap<TransactionId, TransactionExecution>,
    ) -> Result<Option<Command>, HotStuffError> {
        // Only set to abort if either the local or one or more foreign shards decided to ABORT
        if tx_rec.current_decision().is_abort() {
            return Ok(Some(Command::SomePrepare(tx_rec.get_current_transaction_atom())));
        }

        let mut execution =
            self.execute_transaction(tx, &parent_block.block_id, parent_block.epoch, tx_rec.transaction_id())?;

        // Try to lock all local outputs
        let local_outputs = execution
            .resulting_outputs()
            .iter()
            .filter(|o| {
                o.substate_id().is_transaction_receipt() ||
                    local_committee_info.includes_substate_address(&o.to_substate_address())
            })
            .map(|output| SubstateRequirementLockIntent::from(output.clone()));
        let lock_status = substate_store.try_lock_all(*tx_rec.transaction_id(), local_outputs, false)?;
        if let Some(err) = lock_status.failures().first() {
            warn!(
                target: LOG_TARGET,
                "⚠️ Failed to lock outputs for transaction {}: {}",
                tx_rec.transaction_id(),
                err,
            );
            // If the transaction does not lock, we propose to abort it
            execution.set_abort_reason(RejectReason::FailedToLockOutputs(err.to_string()));
            tx_rec.update_from_execution(
                local_committee_info.num_preshards(),
                local_committee_info.num_committees(),
                &execution,
            );

            executed_transactions.insert(*tx_rec.transaction_id(), execution);
            return Ok(Some(Command::AllPrepare(tx_rec.get_current_transaction_atom())));
        }

        tx_rec.update_from_execution(
            local_committee_info.num_preshards(),
            local_committee_info.num_committees(),
            &execution,
        );

        executed_transactions.insert(*tx_rec.transaction_id(), execution);
        // If we locally decided to ABORT, we are still saying that we think all prepared. When we enter the acceptance
        // phase, we will propose SomeAccept for this case.
        Ok(Some(Command::AllPrepare(tx_rec.get_current_transaction_atom())))
    }

    fn accept_transaction(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        parent_block: &LeafBlock,
        tx_rec: &mut TransactionPoolRecord,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TConsensusSpec::StateStore>,
    ) -> Result<Option<Command>, HotStuffError> {
        if tx_rec.current_decision().is_abort() {
            return Ok(Some(Command::SomeAccept(tx_rec.get_current_transaction_atom())));
        }

        let execution =
            BlockTransactionExecution::get_pending_for_block(tx, tx_rec.transaction_id(), &parent_block.block_id)
                .optional()?
                .ok_or_else(|| {
                    HotStuffError::InvariantError(format!(
                        "accept_transaction: Transaction {} has COMMIT decision but execution is missing",
                        tx_rec.transaction_id(),
                    ))
                })?;
        let diff = execution.result().finalize.accept().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "local_accept_transaction: Transaction {} has COMMIT decision but execution failed when proposing",
                tx_rec.transaction_id(),
            ))
        })?;
        substate_store.put_diff(
            *tx_rec.transaction_id(),
            &filter_diff_for_committee(local_committee_info, diff),
        )?;
        let atom = self.get_transaction_atom_with_leader_fee(local_committee_info, tx_rec)?;
        Ok(Some(Command::AllAccept(atom)))
    }

    fn get_transaction_atom_with_leader_fee(
        &self,
        local_committee_info: &CommitteeInfo,
        tx_rec: &mut TransactionPoolRecord,
    ) -> Result<TransactionAtom, HotStuffError> {
        if tx_rec.current_decision().is_commit() {
            let num_involved_shard_groups =
                local_committee_info.count_distinct_shard_groups(tx_rec.evidence().substate_addresses_iter());
            let involved = NonZeroU64::new(num_involved_shard_groups as u64).ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "PROPOSE: Transaction {} involves zero shard groups",
                    tx_rec.transaction_id(),
                ))
            })?;
            let leader_fee = tx_rec.calculate_leader_fee(involved, EXHAUST_DIVISOR);
            tx_rec.set_leader_fee(leader_fee);
        }
        let atom = tx_rec.get_current_transaction_atom();
        Ok(atom)
    }

    fn execute_transaction(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        parent_block_id: &BlockId,
        current_epoch: Epoch,
        transaction_id: &TransactionId,
    ) -> Result<TransactionExecution, HotStuffError> {
        let transaction = TransactionRecord::get(tx, transaction_id)?;
        // Might have been executed already if all inputs are local
        if let Some(execution) =
            BlockTransactionExecution::get_pending_for_block(tx, transaction_id, parent_block_id).optional()?
        {
            info!(
                target: LOG_TARGET,
                "👨‍🔧 PROPOSE: Using existing transaction execution {} ({})",
                transaction_id, execution.execution.decision(),
            );
            return Ok(execution.into_transaction_execution());
        }

        let pledged = PledgedTransaction::load_pledges(tx, transaction)?;

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PROPOSE: Executing transaction {} (pledges: {} local, {} foreign)",
            transaction_id, pledged.local_pledges.len(), pledged.foreign_pledges.len(),
        );

        let executed = self
            .transaction_manager
            .execute(current_epoch, pledged)
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        Ok(executed.into_execution())
    }
}

pub fn get_non_local_shards(diff: &[SubstateChange], local_committee_info: &CommitteeInfo) -> HashSet<Shard> {
    diff.iter()
        .map(|ch| {
            ch.versioned_substate_id()
                .to_substate_address()
                .to_shard(local_committee_info.num_preshards())
        })
        .filter(|shard| local_committee_info.shard_group().contains(shard))
        .collect()
}
