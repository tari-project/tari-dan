//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashSet,
    fmt::{Debug, Formatter},
    iter,
    time::{Duration, Instant},
};

use log::*;
use tari_consensus_types::{
    BlockId,
    HighPc,
    HighTc,
    HighestSeenBlock,
    LastExecuted,
    LastProposed,
    LastSentVote,
    LeafBlock,
    PcId,
    ProposalCertificate,
    TimeoutCertificate,
};
use tari_engine_types::fees::ExhaustBurnRate;
use tari_epoch_manager::{EpochManagerEvent, EpochManagerReader};
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    ProtocolVersion,
    ShardGroup,
    VersionedSubstateId,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{
        Block,
        BookkeepingEpochAgnosticRead,
        BookkeepingModel,
        EpochCheckpoint,
        ForeignProposalRecord,
        NoVoteReason,
        ProposalOrigin,
        SubstateRecord,
        TransactionPool,
        TransactionRecord,
    },
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_shutdown::ShutdownSignal;
use tari_sidechain::FixedHash;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::{
    sync::{broadcast, mpsc},
    time,
};

use super::{
    LeaderSkipSet,
    ProposalValidationError,
    calculate_last_dummy_block,
    config::HotstuffConfig,
    get_highest_seen_justified_view,
    on_receive_new_transaction::OnReceiveNewTransaction,
};
use crate::{
    hotstuff::{
        epoch_end_drain::EpochEndDrain,
        epoch_gc::EpochGc,
        epoch_state::EpochState,
        error::HotStuffError,
        event::HotstuffEvent,
        exhaust_burn_rate::resolve_epoch_exhaust_burn_rate,
        on_catch_up_sync::OnCatchUpSync,
        on_catch_up_sync_request::OnSyncRequest,
        on_inbound_message::{InboundMessage, OnInboundMessage},
        on_leader_timeout::LeaderTimeout,
        on_message_validate::{MessageValidationResult, OnMessageValidate},
        on_next_sync_view::OnNextSyncViewHandler,
        on_propose::OnPropose,
        on_receive_foreign_proposal::OnReceiveForeignProposalHandler,
        on_receive_local_proposal::OnReceiveLocalProposalHandler,
        on_receive_new_view::OnReceiveNewViewHandler,
        on_receive_request_missing_transactions::OnReceiveRequestMissingTransactions,
        on_receive_vote::OnReceiveVoteHandler,
        pacemaker::PaceMaker,
        pacemaker_handle::PaceMakerHandle,
        state_tree_gc::StateTreeGc,
        substate_store::ShardedStateTree,
        transaction_manager::{ConsensusTransactionManager, UncommittedTemplateStore},
        vote_collector::{ProposalVoteCollector, TimeoutVoteCollector},
    },
    messages::{HotstuffMessage, ProposalMessage},
    tracing::TraceTimer,
    traits::{CertificateStore, ConsensusSpec, PeriodicTask, hooks::ConsensusHooks},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::worker";

/// See [`HotstuffWorker::new_transactions_at_epoch_end`].
#[derive(Debug, Clone)]
pub struct NewTransactionsAtEpochEnd {
    epoch: Epoch,
    transaction_ids: Vec<TransactionId>,
}

#[derive(Debug, Clone, Copy)]
pub struct ClearedTransactionPool {
    pub num_removed: usize,
    pub num_resequenced: usize,
}

/// Reason the hotstuff worker loop exited cleanly (i.e. returned Ok).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerExitReason {
    Shutdown,
}

pub struct HotstuffWorker<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,

    config: HotstuffConfig,
    hooks: TConsensusSpec::Hooks,

    tx_events: broadcast::Sender<HotstuffEvent>,
    rx_new_transactions: mpsc::Receiver<(Transaction, usize)>,
    rx_missing_transactions: mpsc::UnboundedReceiver<Vec<TransactionId>>,

    on_inbound_message: OnInboundMessage<TConsensusSpec>,
    on_next_sync_view: OnNextSyncViewHandler<TConsensusSpec>,
    on_receive_local_proposal: OnReceiveLocalProposalHandler<TConsensusSpec>,
    on_receive_foreign_proposal: OnReceiveForeignProposalHandler<TConsensusSpec>,
    on_receive_vote: OnReceiveVoteHandler<TConsensusSpec>,
    on_receive_new_view: OnReceiveNewViewHandler<TConsensusSpec>,
    on_receive_request_missing_txs: OnReceiveRequestMissingTransactions<TConsensusSpec>,
    on_receive_new_transaction: OnReceiveNewTransaction<TConsensusSpec>,
    on_message_validate: OnMessageValidate<TConsensusSpec>,
    on_propose: OnPropose<TConsensusSpec>,
    on_sync_request: OnSyncRequest<TConsensusSpec>,
    on_catch_up_sync: OnCatchUpSync<TConsensusSpec>,
    worker_state: WorkerState<TConsensusSpec::Addr>,

    state_store: TConsensusSpec::StateStore,
    uncommitted_templates: UncommittedTemplateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,

    /// Epoch to switch to (Epoch, activated_at)
    next_epoch: Option<(Epoch, Instant)>,
    /// Whether the end of the given epoch changes the number of committees. Cached for the epoch being ended.
    next_epoch_changes_layout: Option<(Epoch, bool)>,
    epoch_manager: TConsensusSpec::EpochManager,
    pacemaker_worker: Option<PaceMaker>,
    pacemaker: PaceMakerHandle,
    shutdown: ShutdownSignal,
}
impl<TConsensusSpec: ConsensusSpec> HotstuffWorker<TConsensusSpec> {
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub fn new(
        config: HotstuffConfig,
        local_validator_addr: TConsensusSpec::Addr,
        inbound_messaging: TConsensusSpec::InboundMessaging,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        rx_new_transactions: mpsc::Receiver<(Transaction, usize)>,
        state_store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        signing_service: TConsensusSpec::SignerService,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        transaction_executor: TConsensusSpec::TransactionExecutor,
        transaction_validator: TConsensusSpec::TransactionValidator,
        tx_events: broadcast::Sender<HotstuffEvent>,
        hooks: TConsensusSpec::Hooks,
        shutdown: ShutdownSignal,
    ) -> Self {
        let (tx_missing_transactions, rx_missing_transactions) = mpsc::unbounded_channel();
        let pacemaker = PaceMaker::new(config.consensus_constants.pacemaker_block_time);
        let proposal_vote_collector = ProposalVoteCollector::new(
            config.network,
            state_store.clone(),
            epoch_manager.clone(),
            signing_service.clone(),
            hooks.clone(),
        );
        let timeout_vote_collector = TimeoutVoteCollector::new(
            config.network,
            state_store.clone(),
            epoch_manager.clone(),
            signing_service.clone(),
        );
        let transaction_manager = ConsensusTransactionManager::new(
            transaction_executor.clone(),
            config.consensus_constants.max_transaction_validity_epochs,
            config.consensus_constants.max_block_pledge_bytes,
        )
        .with_speculation(config.execution_threads, state_store.clone());
        let uncommitted_templates = transaction_manager.uncommitted_templates().clone();

        Self {
            local_validator_addr: local_validator_addr.clone(),

            config: config.clone(),
            rx_new_transactions,
            rx_missing_transactions,

            on_inbound_message: OnInboundMessage::new(
                config.network,
                inbound_messaging,
                epoch_manager.clone(),
                signing_service.clone(),
                hooks.clone(),
            ),
            on_message_validate: OnMessageValidate::new(
                config.clone(),
                state_store.clone(),
                epoch_manager.clone(),
                pacemaker.clone_handle().current_view().clone(),
                signing_service.clone(),
                outbound_messaging.clone(),
                tx_events.downgrade(),
            ),

            on_next_sync_view: OnNextSyncViewHandler::new(
                config.clone(),
                state_store.clone(),
                outbound_messaging.clone(),
                leader_strategy.clone(),
                signing_service.clone(),
            ),
            on_receive_local_proposal: OnReceiveLocalProposalHandler::new(
                state_store.clone(),
                epoch_manager.clone(),
                leader_strategy.clone(),
                pacemaker.clone_handle(),
                outbound_messaging.clone(),
                signing_service.clone(),
                transaction_pool.clone(),
                tx_events.downgrade(),
                transaction_manager.clone(),
                config.clone(),
                hooks.clone(),
            ),
            on_receive_foreign_proposal: OnReceiveForeignProposalHandler::new(
                state_store.clone(),
                epoch_manager.clone(),
                pacemaker.clone_handle(),
                outbound_messaging.clone(),
            ),
            on_receive_vote: OnReceiveVoteHandler::new(
                config.clone(),
                state_store.clone(),
                pacemaker.clone_handle(),
                proposal_vote_collector.clone(),
                local_validator_addr.clone(),
                leader_strategy.clone(),
            ),
            on_receive_new_view: OnReceiveNewViewHandler::new(
                config.clone(),
                local_validator_addr,
                state_store.clone(),
                leader_strategy.clone(),
                pacemaker.clone_handle(),
                proposal_vote_collector,
                timeout_vote_collector,
            ),
            on_receive_request_missing_txs: OnReceiveRequestMissingTransactions::new(
                state_store.clone(),
                outbound_messaging.clone(),
            ),
            on_receive_new_transaction: OnReceiveNewTransaction::new(
                state_store.clone(),
                transaction_pool.clone(),
                transaction_validator,
                tx_missing_transactions,
            ),
            on_propose: OnPropose::new(
                config,
                state_store.clone(),
                epoch_manager.clone(),
                transaction_pool.clone(),
                transaction_manager,
                signing_service,
                outbound_messaging.clone(),
            ),

            on_sync_request: OnSyncRequest::new(state_store.clone(), outbound_messaging.clone()),
            on_catch_up_sync: OnCatchUpSync::new(state_store.clone(), pacemaker.clone_handle(), outbound_messaging),
            worker_state: WorkerState::default(),

            state_store,
            uncommitted_templates,
            leader_strategy,
            epoch_manager,
            transaction_pool,

            next_epoch: None,
            next_epoch_changes_layout: None,
            pacemaker: pacemaker.clone_handle(),
            pacemaker_worker: Some(pacemaker),
            hooks,
            tx_events,
            shutdown,
        }
    }

    pub fn pacemaker(&self) -> &PaceMakerHandle {
        &self.pacemaker
    }

    pub fn hooks_mut(&mut self) -> &mut TConsensusSpec::Hooks {
        &mut self.hooks
    }

    async fn get_starting_epoch(&self) -> Result<(Epoch, FixedHash), HotStuffError> {
        // NOTE: we assume the latest checkpoint has been synced already
        let (last_finalised_epoch, leaf_epoch) = self.state_store.with_read_tx(|tx| {
            let checkpoint = EpochCheckpoint::get_last_checkpoint(tx).optional()?;
            let leaf = LeafBlock::get_any(tx).optional()?;
            Ok::<_, HotStuffError>((checkpoint.map(|ch| ch.epoch()), leaf.map(|leaf| leaf.epoch())))
        })?;

        let current_epoch = match select_starting_epoch(last_finalised_epoch, leaf_epoch) {
            Some(epoch) => epoch,
            None => self.epoch_manager.current_epoch().await?,
        };

        let epoch_hash = self.epoch_manager.get_epoch_hash(current_epoch).await?;

        Ok((current_epoch, epoch_hash))
    }

    /// If the oracle crossed an epoch boundary while this node was offline, the corresponding `EpochChanged` event was
    /// published when the epoch manager completed its initial scan — before this worker subscribed to epoch events.
    /// Broadcast events are not replayed, so without re-deriving `next_epoch` from the oracle here, no validator
    /// resuming a stalled epoch would propose `EndEpoch` until the next base-layer epoch boundary publishes a fresh
    /// event, leaving the committee stuck one epoch behind for an entire base-layer epoch.
    ///
    /// Seeds the epoch immediately following `starting_epoch` and lets consensus progress naturally from there. If
    /// the oracle is more than one epoch ahead, the committee transitions one epoch and the next `EpochChanged` event
    /// (or peer QCs at a higher epoch) drives the rest.
    async fn seed_next_epoch_from_oracle(&mut self, starting_epoch: Epoch) -> Result<(), HotStuffError> {
        let oracle_epoch = self.epoch_manager.current_epoch().await?;
        if oracle_epoch <= starting_epoch {
            return Ok(());
        }

        let next_epoch = starting_epoch + Epoch(1);
        if !self
            .epoch_manager
            .is_this_validator_registered_for_epoch(next_epoch)
            .await?
        {
            info!(
                target: LOG_TARGET,
                "💤 This validator is not registered for next epoch {next_epoch}. Will stop consensus once the \
                 current epoch {starting_epoch} has transitioned."
            );
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "🌟 Oracle is at epoch {oracle_epoch} but consensus is resuming at {starting_epoch}. Seeding next epoch \
             {next_epoch} so that EndEpoch can be proposed.",
        );
        self.next_epoch = Some((next_epoch, Instant::now()));
        Ok(())
    }

    pub async fn start(&mut self) -> Result<WorkerExitReason, HotStuffError> {
        let (current_epoch, current_epoch_hash) = self.get_starting_epoch().await?;
        let local_committee_info = self.epoch_manager.get_local_committee_info(current_epoch).await?;

        self.create_genesis_block_if_required(current_epoch, current_epoch_hash, local_committee_info.shard_group())?;
        // Blocks saved before a restart are still uncommitted, and their templates are in no durable state.
        self.state_store.with_read_tx(|tx| {
            if let Some(committed) = LastExecuted::get(tx, current_epoch).optional()? {
                self.uncommitted_templates.prune_to(committed.epoch, committed.height);
                self.uncommitted_templates.rebuild(tx, &committed.block_id)?;
            }
            Ok::<_, HotStuffError>(())
        })?;
        // Resume pacemaker from the last epoch/height
        let (current_height, leaf_height) = self.state_store.with_read_tx(|tx| {
            let height = get_highest_seen_justified_view(tx, current_epoch)?;
            let leaf_block = LeafBlock::get(tx, current_epoch)?;
            Ok::<_, HotStuffError>((height, leaf_block.height()))
        })?;

        self.worker_state.has_processed_first_block = !leaf_height.is_zero();

        self.pacemaker
            .start(current_epoch, current_height, current_height)
            .await?;

        info!(target: LOG_TARGET, "🚀 Pacemaker started at epoch {}, height: {}", current_epoch, current_height);

        // Recovery: if the previous run crashed inside `process_end_of_epoch` (e.g. NoEpochFound
        // because the local oracle was lagging), the EOE block is committed on disk but no
        // checkpoint was written. Detect that here and seed the deferred-EOE state so a later
        // retry (or this startup if the oracle is already current) finishes the transition.
        self.recover_pending_end_of_epoch(current_epoch).await?;

        // Catch up on any epoch change the oracle observed before this worker subscribed to epoch events.
        self.seed_next_epoch_from_oracle(current_epoch).await?;

        self.publish_event(HotstuffEvent::EpochChanged {
            epoch: current_epoch,
            registered_shard_group: Some(local_committee_info.shard_group()),
        });

        let local_committee = self.epoch_manager.get_local_committee(current_epoch).await?;
        let epoch_state = EpochState {
            epoch: current_epoch,
            epoch_hash: current_epoch_hash,
            exhaust_burn_rate: self.epoch_exhaust_burn_rate(current_epoch)?,
            local_committee_info,
            local_committee,
        };

        let _cancel_state_tree_gc_task_on_drop = StateTreeGc::new(
            self.state_store.clone(),
            epoch_state.local_committee_info.num_preshards(),
        )
        .do_work_periodically(self.config.state_tree_cleanup_interval);

        let _cancel_epoch_gc_task_on_drop =
            EpochGc::new(self.state_store.clone()).do_work_periodically(self.config.epoch_gc_interval);

        let exit_reason = self.run(epoch_state).await?;
        Ok(exit_reason)
    }

    #[allow(clippy::too_many_lines)]
    async fn run(
        &mut self,
        mut epoch_state: EpochState<TConsensusSpec::Addr>,
    ) -> Result<WorkerExitReason, HotStuffError> {
        // Spawn pacemaker if not spawned already
        if let Some(pm) = self.pacemaker_worker.take() {
            pm.spawn();
        }

        let mut on_force_beat = self.pacemaker.get_on_force_beat();
        let mut on_leader_timeout = self.pacemaker.get_on_leader_timeout();

        let mut epoch_manager_events = self.epoch_manager.subscribe();

        let mut prev_height = self.pacemaker.current_view().get_height();
        let current_epoch = self.pacemaker.current_view().get_epoch();
        let mut local_claim_public_key = self
            .epoch_manager
            .get_our_validator_node(current_epoch)
            .await?
            .fee_claim_public_key;

        // Assume we need to catch up at startup. If we're already at the latest height, validators will not return
        // blocks.
        self.request_catch_up_sync(&epoch_state).await?;

        let mut periodic_tasks = time::interval(Duration::from_secs(10));
        periodic_tasks.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

        let exit_reason;
        loop {
            let current_height = self.pacemaker.current_view().get_height();
            let current_epoch = self.pacemaker.current_view().get_epoch();

            // Need to update epoch state if the epoch has changed
            if epoch_state.epoch != current_epoch {
                let exhaust_burn_rate = self.epoch_exhaust_burn_rate(current_epoch)?;
                epoch_state
                    .update_from_epoch_manager(&self.epoch_manager, current_epoch, exhaust_burn_rate)
                    .await?;
                local_claim_public_key = self
                    .epoch_manager
                    .get_our_validator_node(current_epoch)
                    .await?
                    .fee_claim_public_key;
                // We only clear next_epoch if we've reached it, If more than one eopch has progressed (edge case
                // expected only on testnets with low difficulties) then we need to keep the next epoch
                // in place so that we can quickly progress to the actual current epoch.
                if let Some((next_epoch, _)) = self.next_epoch.as_ref() {
                    if *next_epoch == current_epoch {
                        self.next_epoch = None;
                    } else {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ this node/shard group is more than one epoch behind! Current epoch: {}, worker epoch: {}",
                            next_epoch,
                            current_epoch,
                        );
                    }
                }
            }

            if current_height != prev_height {
                self.hooks.on_pacemaker_height_changed(current_height);
                prev_height = current_height;
            }

            debug!(
                target: LOG_TARGET,
                "🔥 {} Current height #{}",
                self.local_validator_addr,
                current_height.as_u64()
            );

            tokio::select! {
                Ok(event) = epoch_manager_events.recv() => {
                    self.on_epoch_manager_event(event).await?;
                },

                forced_height = on_force_beat.wait() => {
                    if let Err(e) = self.on_force_beat(&epoch_state, current_height, forced_height, &local_claim_public_key).await {
                        self.on_failure("propose_if_leader", &e).await;
                        return Err(e);
                    }
                },

                _ = self.pacemaker.on_beat() => {
                    if let Err(e) = self.on_beat(&epoch_state,  &local_claim_public_key).await {
                        self.on_failure("on_beat", &e).await;
                        return Err(e);
                    }
                },

                Some((tx_id, pending)) = self.rx_new_transactions.recv() => {
                    if let Err(err) = self.on_new_transaction(tx_id, pending, &epoch_state,current_height ).await {
                        self.hooks.on_error(&err);
                        error!(target: LOG_TARGET, "🚨Error handling new transaction: {}", err);
                    }
                },

                Some(result) = self.on_inbound_message.next_message(&epoch_state, current_height, self.worker_state.has_processed_first_block) => {
                    if let Err(e) = self.on_unvalidated_message(&epoch_state, current_height, result).await {
                        self.on_failure("on_unvalidated_message", &e).await;
                        return Err(e);
                    }
                },

               // TODO: This channel is used to work around some design-flaws in missing transactions handling.
                //       We cannot simply call check_if_block_can_be_unparked in dispatch_hotstuff_message as that creates a cycle.
                //       One suggestion is to refactor consensus to emit events (kinda like libp2p does) and handle those events.
                //       This should be easy to reason about and avoid a large depth of async calls and "callback channels".
                Some(batch) = self.rx_missing_transactions.recv() => {
                    if let Err(err) = self.check_if_block_can_be_unparked(&epoch_state, current_height, batch.iter()).await {
                        self.hooks.on_error(&err);
                        error!(target: LOG_TARGET, "🚨Error handling missing transaction: {}", err);
                    }
                },

                timeout = on_leader_timeout.wait() => {
                    if let Err(e) = self.on_leader_timeout(&epoch_state, current_height, Some(timeout)).await {
                        self.on_failure("on_leader_timeout", &e).await;
                        return Err(e);
                    }
                },

                _ = periodic_tasks.tick() => {
                    if let Err(e) = self.on_task_tick(&epoch_state).await {
                        self.hooks.on_error(&e);
                        error!(target: LOG_TARGET, "🚨Error during periodic task: {}", e);
                    }

                    // A deferred EOE waits on the epoch row, which the oracle writes as soon as it
                    // applies the queued EpochChanged for that epoch - between scan batches during a
                    // catch-up. EpochManagerEvent::EpochChanged only fires once the whole scan
                    // reaches the lagged tip, so poll the data here rather than waiting for it.
                    // Ordered after `on_task_tick` because a successful resume advances the pacemaker
                    // past `epoch_state`, which is only refreshed at the top of the next iteration.
                    self.try_resume_pending_end_of_epoch().await?;
                },

                _ = self.shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Consensus shutting down");
                    exit_reason = WorkerExitReason::Shutdown;
                    break;
                },
            }
        }

        self.on_inbound_message.clear_buffer();
        if let Err(err) = self.pacemaker.stop().await {
            debug!(target: LOG_TARGET, "Pacemaker channel dropped: {}", err);
        }

        Ok(exit_reason)
    }

    async fn on_unvalidated_message(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        result: Result<InboundMessage<TConsensusSpec::Addr>, HotStuffError>,
    ) -> Result<(), HotStuffError> {
        let (from, msg) = match result? {
            InboundMessage::Ready { from, message } => (from, message),
            InboundMessage::ProposalAhead { from, justify } => {
                return self
                    .on_proposal_ahead(epoch_state, current_height, from, &justify)
                    .await;
            },
        };

        match self
            .on_message_validate
            .handle(
                current_height,
                epoch_state,
                from.clone(),
                msg,
                &self.on_receive_new_transaction,
                &mut self.on_receive_foreign_proposal,
            )
            .await?
        {
            MessageValidationResult::Ready { from, message: msg } => {
                if let Err(e) = self
                    .dispatch_hotstuff_message(epoch_state, current_height, from, msg)
                    .await
                {
                    return self.handle_hotstuff_error(current_height, epoch_state, None, e).await;
                }
                Ok(())
            },
            MessageValidationResult::RequestedTransactions {
                from,
                block_id,
                epoch,
                transactions,
                remaining,
            } => {
                let result = self
                    .on_requested_transactions(epoch_state, from, block_id, epoch, transactions, remaining)
                    .await;
                if let Err(e) = log_err("on_receive_new_transaction", result) {
                    return self.handle_hotstuff_error(current_height, epoch_state, None, e).await;
                }
                Ok(())
            },
            MessageValidationResult::ParkedProposal {
                epoch,
                missing_txs,
                block_id,
                ..
            } => {
                let mut request_from_address = from;
                if request_from_address == self.local_validator_addr {
                    // Edge case: If we're catching up, we could be the proposer but we no longer have
                    // the transaction (we deleted our database likely during development testing).
                    // In this case, request from another random VN.
                    // (TODO: not 100% reliable since we're just asking a single random committee member)
                    match epoch_state
                        .local_committee
                        .shuffled()
                        .find(|m| m.address != self.local_validator_addr)
                    {
                        Some(m) => {
                            warn!(
                                target: LOG_TARGET,
                                "⚠️Requesting missing transactions from another validator {} because we are (presumably) catching up (local_peer_id = {})",
                                m,
                                self.local_validator_addr,
                            );
                            request_from_address = m.address.clone();
                        },
                        None => {
                            warn!(
                                target: LOG_TARGET,
                                "❌NEVERHAPPEN: We're the only validator in the committee but we need to request missing transactions."
                            );
                            return Ok(());
                        },
                    }
                }

                self.on_message_validate
                    .request_missing_transactions(request_from_address, block_id, epoch, missing_txs)
                    .await?;
                Ok(())
            },
            MessageValidationResult::Discard => Ok(()),
            // In these cases, we want to propagate the error back to the state machine, to allow sync
            MessageValidationResult::Invalid { err, message, from } => {
                if let HotStuffError::ProposalValidationError(_) = err {
                    error!(target: LOG_TARGET, "🚨 Invalid message from {from}: {err} - {message}");
                }

                self.handle_hotstuff_error(current_height, epoch_state, None, err).await
            },
        }
    }

    /// Processes the transactions a peer returned for a missing-transactions request and asks the same peer for the
    /// rest. A response is bounded in bytes, so the peer may hold more of the request than it sent. Each follow-up
    /// asks for strictly fewer transactions, and a response that returns nothing ends the exchange.
    async fn on_requested_transactions(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        from: TConsensusSpec::Addr,
        block_id: BlockId,
        epoch: Epoch,
        transactions: Vec<Transaction>,
        remaining: HashSet<TransactionId>,
    ) -> Result<(), HotStuffError> {
        let made_progress = !transactions.is_empty();
        self.on_receive_new_transaction
            .process_requested(
                epoch_state.epoch(),
                from.clone(),
                block_id,
                transactions,
                epoch_state.local_committee_info(),
            )
            .await?;
        if made_progress && !remaining.is_empty() {
            self.on_message_validate
                .request_missing_transactions(from, block_id, epoch, remaining)
                .await?;
        }
        Ok(())
    }

    async fn on_new_transaction(
        &mut self,
        transaction: Transaction,
        num_pending_txs: usize,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::info(LOG_TARGET, "on_new_transaction");
        let maybe_transaction = self.on_receive_new_transaction.try_sequence_transaction(
            epoch_state.epoch(),
            TransactionRecord::new(transaction),
            epoch_state.local_committee_info(),
        )?;

        let Some(transaction) = maybe_transaction else {
            return Ok(());
        };

        info!(
            target: LOG_TARGET,
            "🔥 new transaction ready for consensus: {} ({} pending)",
            transaction.id(),
            num_pending_txs,
        );

        self.hooks.on_transaction_ready(transaction.id());

        if self
            .check_if_block_can_be_unparked(epoch_state, current_height, iter::once(transaction.id()))
            .await?
        {
            // No need to call on_beat, a block was unparked so on_beat will be called as needed
            return Ok(());
        }

        // There are num_pending_txs transactions in the queue. If we have no pending transactions, we'll propose now if
        // able.
        if num_pending_txs == 0 {
            self.pacemaker.beat();
        }

        Ok(())
    }

    /// Returns true if a block was unparked, otherwise false
    async fn check_if_block_can_be_unparked<
        'a,
        I: IntoIterator<Item = &'a TransactionId> + ExactSizeIterator + Clone,
    >(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        transaction_ids: I,
    ) -> Result<bool, HotStuffError> {
        let (local_proposals, foreign_proposals) = self.on_message_validate.update_parked_blocks(transaction_ids)?;

        let is_any_block_unparked = !local_proposals.is_empty() || !foreign_proposals.is_empty();

        for msg in foreign_proposals {
            if let Err(e) = self
                .on_receive_foreign_proposal
                .handle_received(msg, epoch_state.local_committee_info())
                .await
            {
                self.on_failure("check_if_block_can_be_unparked -> on_receive_foreign_proposal", &e)
                    .await;
                return Err(e);
            }
        }

        for (msg, origin) in local_proposals {
            if let Err(e) = self.on_proposal_message(epoch_state, current_height, msg, origin).await {
                self.on_failure("check_if_block_can_be_unparked -> on_proposal_message", &e)
                    .await;
                return Err(e);
            }
        }

        Ok(is_any_block_unparked)
    }

    /// Detects an EOE block that was committed but whose next-epoch processing did not finish
    /// (most likely the previous run crashed inside `process_end_of_epoch` while looking up an
    /// epoch the local oracle had not yet observed). When found, seeds the deferred-EOE state on
    /// the local-proposal handler and immediately attempts to resume; if the oracle is still
    /// behind, the resume will re-defer and the worker's periodic retry will pick it up.
    async fn recover_pending_end_of_epoch(&mut self, current_epoch: Epoch) -> Result<(), HotStuffError> {
        let pending = self.state_store.with_read_tx(|tx| {
            // A successful end-of-epoch transition writes a checkpoint for `current_epoch`.
            // If the latest checkpoint covers this epoch (or later), the previous run finished
            // cleanly and there is nothing to recover.
            if let Some(last) = EpochCheckpoint::get_last_checkpoint(tx).optional()? &&
                last.epoch() >= current_epoch
            {
                return Ok::<_, HotStuffError>(None);
            }

            // Walk back from the leaf looking for a committed EOE block in `current_epoch`.
            let leaf = LeafBlock::get(tx, current_epoch)?;
            if leaf.is_genesis() {
                return Ok(None);
            }
            let mut block = Block::get(tx, leaf.block_id())?;
            loop {
                if block.is_epoch_end() && block.is_committed() {
                    let commit_qc = block.get_commit_qc(tx)?;
                    return Ok(Some((block, commit_qc)));
                }
                if block.height().is_zero() || block.parent().is_zero() {
                    return Ok(None);
                }
                block = block.get_parent(tx)?;
            }
        })?;

        if let Some((eoe_block, commit_qc)) = pending {
            info!(
                target: LOG_TARGET,
                "🔄 Detected unprocessed EOE block {} on startup. Seeding deferred end-of-epoch state.",
                eoe_block.id()
            );
            self.on_receive_local_proposal
                .set_pending_end_of_epoch(eoe_block, commit_qc);
            // Attempt to complete immediately. If the oracle is still behind, this will re-defer
            // and a later EpochChanged event will retry.
            let _ = self.on_receive_local_proposal.try_resume_pending_end_of_epoch().await?;
        }
        Ok(())
    }

    /// Retries deferred end-of-epoch processing if the local oracle has since observed the next
    /// epoch. A no-op when nothing is pending, and re-defers if the oracle is still behind.
    async fn try_resume_pending_end_of_epoch(&mut self) -> Result<(), HotStuffError> {
        if !self.on_receive_local_proposal.has_pending_end_of_epoch() {
            return Ok(());
        }
        if let Err(err) = self.on_receive_local_proposal.try_resume_pending_end_of_epoch().await {
            error!(
                target: LOG_TARGET,
                "Failed to resume deferred end-of-epoch processing: {err}"
            );
            return Err(err);
        }
        Ok(())
    }

    async fn on_epoch_manager_event(&mut self, event: EpochManagerEvent) -> Result<(), HotStuffError> {
        match event {
            EpochManagerEvent::EpochChanged {
                epoch,
                registered_shard_group,
                activated_at,
            } => {
                if registered_shard_group.is_none() {
                    let current_epoch = self.pacemaker.current_view().get_epoch();
                    if current_epoch < epoch {
                        info!(
                            target: LOG_TARGET,
                            "💤 This validator is not registered for next epoch {epoch}. Will stop consensus once the current epoch {current_epoch} has transitioned."
                        );
                        return Ok(());
                    }
                    info!(
                        target: LOG_TARGET,
                        "💤 This validator is not registered for epoch {}. Going to sleep.", epoch
                    );

                    return Err(HotStuffError::NotRegisteredForCurrentEpoch { epoch });
                }
                info!(
                    target: LOG_TARGET,
                    "🌟 This validator is registered for epoch {}.", epoch
                );
                self.next_epoch = Some((epoch, activated_at));

                // Oracle just observed `epoch`. If we previously deferred end-of-epoch
                // processing because the oracle was lagging, retry now that the data is
                // available.
                self.try_resume_pending_end_of_epoch().await?;
            },
        }

        Ok(())
    }

    /// Catches up from `from`, which proposed on the certified block `justify` while this node lags more than one
    /// view behind it. The proposal itself waits in the view buffer until catch-up imports its ancestors.
    async fn on_proposal_ahead(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        justify: &ProposalCertificate,
    ) -> Result<(), HotStuffError> {
        if self.worker_state.is_active_catch_up() || from == self.local_validator_addr {
            return Ok(());
        }
        // A certified block we already hold, e.g. one parked on missing transactions, is not fetched by catching
        // up.
        let block_id = justify.calculate_block_id();
        if self
            .state_store
            .with_read_tx(|tx| Block::record_exists(tx, &block_id))?
        {
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "⏩ {from} proposed on certified block {}/{} while this node is at {current_height}. Catching up from {from}.",
            justify.epoch(),
            justify.height(),
        );
        self.on_catch_up_sync
            .request_sync(epoch_state.epoch(), from.clone())
            .await?;
        self.worker_state.catch_up = Some(CatchUp::new(
            justify.height(),
            current_height,
            self.config.catch_up_request_timeout,
            from,
        ));
        Ok(())
    }

    async fn request_catch_up_sync(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
    ) -> Result<Option<TConsensusSpec::Addr>, HotStuffError> {
        let current_addr = self.worker_state.catch_up.as_ref().map(|c| c.from.clone());
        let committee_size = epoch_state.local_committee().len();
        for member in epoch_state.local_committee().shuffled() {
            // Exclude self and, if there are more than 2 committee members, exclude the current sync peer
            if member.address != self.local_validator_addr &&
                (committee_size <= 2 || Some(&member.address) != current_addr.as_ref())
            {
                self.on_catch_up_sync
                    .request_sync(epoch_state.epoch(), member.address.clone())
                    .await?;
                return Ok(Some(member.address.clone()));
            }
        }
        // We're the only committee member
        Ok(None)
    }

    async fn on_failure(&mut self, context: &str, err: &HotStuffError) {
        if err.is_sync_required() {
            debug!(target: LOG_TARGET, "⚠️ Behind peers ({}): {}", context, err);
            self.publish_event(HotstuffEvent::SyncRequired {
                message: err.to_string(),
            });
        } else {
            self.hooks.on_error(err);
            self.publish_event(HotstuffEvent::Failure {
                message: err.to_string(),
            });
            error!(target: LOG_TARGET, "Error ({}): {}", context, err);
        }
        if let Err(e) = self.pacemaker.stop().await {
            error!(target: LOG_TARGET, "Error while stopping pacemaker: {}", e);
        }
        self.on_inbound_message.clear_buffer();
    }

    pub fn shutdown_signal(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    /// Read and discard messages. This should be used only when consensus is inactive.
    pub async fn discard_messages(&mut self) {
        loop {
            tokio::select! {
                biased;
                _ = self.shutdown.wait() => {
                    break;
                },
                _ = self.on_inbound_message.discard() => {},
                _ = self.rx_new_transactions.recv() => {}
            }
        }
    }

    async fn on_leader_timeout(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        timeout: Option<LeaderTimeout>,
    ) -> Result<(), HotStuffError> {
        self.hooks.on_leader_timeout(current_height);
        info!(
            target: LOG_TARGET,
            "⚠️ {} Leader failure: NEXTSYNCVIEW for epoch {} and current height {} (timeout: {})",
            self.local_validator_addr,
            epoch_state.epoch(),
            current_height,
            timeout.as_ref().map(|t| t.num_timeouts).display(),
        );
        self.on_next_sync_view
            .handle(epoch_state.epoch(), current_height, epoch_state.local_committee())
            .await?;

        self.publish_event(HotstuffEvent::LeaderTimeout { height: current_height });

        // If we've gone into 3 leader failures in a row, request a catch up sync
        if !self.worker_state.is_active_catch_up() && timeout.is_some_and(|t| t.num_timeouts.is_multiple_of(3)) {
            warn!(
                target: LOG_TARGET,
                "⚠️ {} Leader timeout count is {}. Requesting catch up sync.",
                self.local_validator_addr,
                timeout.as_ref().map(|t| t.num_timeouts).display()
            );
            let Some(sync_req_from) = self.request_catch_up_sync(epoch_state).await? else {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ {} Cannot request catch up sync as there are no other committee members.",
                    self.local_validator_addr,
                );
                return Ok(());
            };
            if let Some(ref mut catch_up) = self.worker_state.catch_up {
                catch_up.reset_timeout(self.config.catch_up_request_timeout);
                catch_up.from = sync_req_from;
            }
        }

        Ok(())
    }

    async fn on_task_tick(&mut self, epoch_state: &EpochState<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        // Re-request foreign proposals that have had no response after a timeout
        self.on_receive_foreign_proposal
            .handle_timed_out_requests(epoch_state.local_committee_info())
            .await?;

        Ok(())
    }

    /// The validators whose slot leader selection skips, for the views a proposal certified at
    /// `justify_height` decides.
    fn load_leader_skip_set(
        &self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        justify_height: NodeHeight,
    ) -> Result<LeaderSkipSet, HotStuffError> {
        let skip_set = self.state_store.with_read_tx(|tx| {
            LeaderSkipSet::load_for_justify(
                tx,
                epoch_state.epoch(),
                justify_height,
                epoch_state.local_committee(),
                &self.config.consensus_constants.liveness_thresholds(),
            )
        })?;
        Ok(skip_set)
    }

    /// Called when it may be time to propose if this node is the leader for the next view
    async fn on_beat(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        local_claim_public_key: &RistrettoPublicKeyBytes,
    ) -> Result<(), HotStuffError> {
        let (highest_justified, high_pc_height, last_proposed) = self.state_store.with_read_tx(|tx| {
            let highest_height = get_highest_seen_justified_view(tx, epoch_state.epoch())?;
            let high_pc_height = HighPc::get(tx, epoch_state.epoch())?.height();
            let last_proposed = LastProposed::get(tx, epoch_state.epoch()).optional()?;
            Ok::<_, HotStuffError>((highest_height, high_pc_height, last_proposed))
        })?;

        // h + 1 because we have not entered the next view yet after creating the new PC.
        // This will happen when we process the block we are about to propose.
        let next_height = highest_justified + NodeHeight(1);
        if last_proposed.is_some_and(|lp| lp.height >= next_height) {
            // We have already proposed at this height, so we don't need to propose again
            debug!(
                target: LOG_TARGET,
                "⤵️ [on_beat] {} Already proposed at height ({})",
                self.local_validator_addr,
                next_height
            );
            return Ok(());
        }

        // Once the highest view justifies this node as leader, we continue i.e we wait for votes to progress the view.
        // Force beat will ensure that if we don't get votes, we will propose with the current QC.
        // The view to propose for is the highest justified one, which a timeout certificate can carry
        // above the high PC. The proposal takes its certificate from the high PC either way, so that is
        // the liveness anchor every validator will check it against.
        let skip_set = self.load_leader_skip_set(epoch_state, high_pc_height)?;
        if !skip_set.is_effective_leader(
            &self.leader_strategy,
            epoch_state.local_committee(),
            highest_justified,
            &self.local_validator_addr,
        ) {
            debug!(
                target: LOG_TARGET,
                "🔥 [on_beat] {} Not the leader for height ({})",
                self.local_validator_addr,
                highest_justified
            );
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "🔥 [on_beat] {} Local node is the leader for {}, num local members: {}, {}",
            self.local_validator_addr,
            highest_justified,
            epoch_state.local_committee().len(),
            epoch_state.local_committee_info().shard_group()
        );

        let epoch_end_drain = self.epoch_end_drain(epoch_state).await?;
        let (propose_now, is_epoch_end_held) = self.state_store.with_read_tx(|tx| {
            let highest_block = HighestSeenBlock::get(tx, epoch_state.epoch())?;
            // Propose quickly if there are UTXOs to mint or transactions to propose
            let propose_now = ForeignProposalRecord::has_unconfirmed(tx, epoch_state.epoch())? ||
                self.transaction_pool.has_ready_or_pending_transaction_updates(
                    tx,
                    highest_block.block_id(),
                    &epoch_end_drain
                        .proposal_filter(epoch_state.epoch(), epoch_state.local_committee_info().shard_group()),
                )?;
            let is_epoch_end_held = epoch_end_drain.is_active() &&
                self.transaction_pool
                    .any_in_chain(tx, highest_block.block_id(), |rec| {
                        epoch_end_drain.holds_epoch_end_for(rec.current_stage())
                    })?;

            Ok::<_, HotStuffError>((propose_now, is_epoch_end_held))
        })?;

        let propose_epoch_end = self.can_propose_epoch_end(epoch_state) && !is_epoch_end_held;
        // Propose quickly if we should end the epoch (i.e base layer epoch > pacemaker epoch)
        if !propose_now && !propose_epoch_end {
            info!(target: LOG_TARGET, "[on_beat] No transactions to propose. Waiting for a timeout.");
            return Ok(());
        }

        if propose_epoch_end {
            info!(
                target: LOG_TARGET,
                "[on_beat] Preparing to propose epoch end {}->{}.",
                epoch_state.epoch,
                self.next_epoch.as_ref().map(|(e, _)| e).display(),
            );
        }

        self.propose_now(
            epoch_state,
            next_height,
            false,
            *local_claim_public_key,
            propose_epoch_end,
        )
        .await?;

        Ok(())
    }

    fn can_propose_epoch_end(&self, epoch_state: &EpochState<TConsensusSpec::Addr>) -> bool {
        // Allow some time for the network to recognise the new epoch. Currently, we get leader failures
        // due to quick end epoch proposals and some nodes have not quite detected it yet.
        self.next_epoch.is_some_and(|(e, at)| {
            e > epoch_state.epoch() &&
                Instant::now().saturating_duration_since(at) >= self.config.epoch_end_grace_period
        })
    }

    /// Called when the block time expires (forced_height == None) or when NEWVIEW quorum (TimeoutCertificate) is
    /// reached (forced_height == Some(_))
    async fn on_force_beat(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        forced_height: Option<NodeHeight>,
        local_claim_public_key: &RistrettoPublicKeyBytes,
    ) -> Result<(), HotStuffError> {
        let height = match forced_height {
            Some(height) => {
                debug!(target: LOG_TARGET, "🔥 [force_beat {}] leader timeout at {height}", self.local_validator_addr);
                height
            },
            None => self
                .state_store
                .with_read_tx(|tx| get_highest_seen_justified_view(tx, epoch_state.epoch()))?,
        };

        let next_height_to_propose = height + NodeHeight(1);
        let last_proposed = self
            .state_store
            .with_read_tx(|tx| LastProposed::get(tx, epoch_state.epoch()))
            .optional()?;
        if last_proposed.is_some_and(|lp| lp.height >= next_height_to_propose) {
            // We have already proposed at this height, so we don't need to propose again
            debug!(
                target: LOG_TARGET,
                "⤵️ [on_force_beat] {} Already proposed at height ({})",
                self.local_validator_addr,
                next_height_to_propose
            );
            return Ok(());
        }

        // `height` is the view this node may propose for (i.e. it should propose to advance the view to
        // h + 1). On a leader timeout that view sits above the certificate the proposal will carry, so
        // the liveness anchor comes from the HighQC rather than from `height`.
        let justify_height = self
            .state_store
            .with_read_tx(|tx| HighPc::get(tx, epoch_state.epoch()))?
            .height();
        let skip_set = self.load_leader_skip_set(epoch_state, justify_height)?;
        let is_leader = skip_set.is_effective_leader(
            &self.leader_strategy,
            epoch_state.local_committee(),
            height,
            &self.local_validator_addr,
        );

        if !is_leader {
            debug!(
                target: LOG_TARGET,
                "🔥 [force_beat] {} Not the leader for {}, local_committee: {} (current height: {})",
                self.local_validator_addr,
                height,
                epoch_state.local_committee().len(),
                current_height
            );
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "🔥 [force_beat] {} Local node is leader for {}, local_committee: {}",
            self.local_validator_addr,
            height,
            epoch_state
                .local_committee()
                .len(),
        );

        let propose_epoch_end = self.can_propose_epoch_end(epoch_state);
        self.propose_now(
            epoch_state,
            next_height_to_propose,
            forced_height.is_some(),
            *local_claim_public_key,
            propose_epoch_end,
        )
        .await?;

        Ok(())
    }

    /// The transactions in the `New` stage of the pool that no shard group has prepared, if this node has committed the
    /// end of its leaf's epoch and has not opened the next epoch.
    ///
    /// A validator whose shard group moves at an epoch boundary stops there and state-syncs its new shard group, and
    /// [`Self::clear_transaction_pool`] then drops its pool. A transaction still `New` at that point has taken no locks
    /// and has not been prepared by this node's shard group. This includes the cross-shard transactions that the
    /// [`EpochEndDrain`] held back. Nothing else holds them for the new shard group, so they are sequenced again once
    /// the sync completes.
    ///
    /// A transaction whose evidence shows a foreign shard group has prepared it is left out. That group still has it
    /// in `LocalPrepared`, and its own drain aborts it, because this shard group never sends the LocalPrepare it is
    /// waiting for. Sequenced again, the transaction would start over with no record of that, and the shard group
    /// taking over the foreign shards, which holds it as finalized, would ignore its LocalPrepare.
    pub fn new_transactions_at_epoch_end(&self) -> Result<Option<NewTransactionsAtEpochEnd>, HotStuffError> {
        self.state_store.with_read_tx(|tx| {
            let checkpoint_epoch = EpochCheckpoint::get_last_checkpoint(tx)
                .optional()?
                .map(|checkpoint| checkpoint.epoch());
            let leaf_epoch = LeafBlock::get_any(tx).optional()?.map(|leaf| leaf.epoch());
            let Some(epoch) = checkpoint_epoch.filter(|epoch| Some(*epoch) == leaf_epoch) else {
                return Ok(None);
            };
            let transaction_ids = self
                .transaction_pool
                .get_all(tx, usize::MAX)?
                .into_iter()
                .filter(|rec| rec.committed_stage().is_new() && !rec.evidence().some_shard_groups_prepared())
                .map(|rec| *rec.id())
                .collect();
            Ok::<_, HotStuffError>(Some(NewTransactionsAtEpochEnd { epoch, transaction_ids }))
        })
    }

    /// Remove all records from the transaction pool, then sequence `at_epoch_end` again for the epoch after it.
    ///
    /// Called after a state sync. State sync only rewrites the state store, not the (persisted)
    /// consensus transaction pool, so the pool can be left holding transactions the freshly-synced
    /// state has already finalised. Re-proposing such a transaction breaks liveness: the rest of the
    /// committee finalised and removed it, so it can never gather a QC again and every block carrying
    /// it is rejected (`TransactionNotInPool`). Clearing lets the node rebuild its pool from fresh
    /// mempool/consensus activity against the synced state.
    ///
    /// `at_epoch_end` (see [`Self::new_transactions_at_epoch_end`]) is sequenced again only if the synced state is the
    /// end of the same epoch. A sync that skipped past it may have skipped their finalization too. Each is validated
    /// as if it had just arrived, against this node's shard group in the next epoch.
    // TODO: the transaction pool only holds pending (derived) state and should not be persisted at
    //       all — that would remove this whole class of pool-vs-state-store divergence.
    pub async fn clear_transaction_pool(
        &self,
        at_epoch_end: Option<NewTransactionsAtEpochEnd>,
    ) -> Result<ClearedTransactionPool, HotStuffError> {
        let mut resequence = None;
        if let Some(at_epoch_end) = at_epoch_end {
            let synced_epoch = self
                .state_store
                .with_read_tx(|tx| EpochCheckpoint::get_last_checkpoint(tx).optional())?
                .map(|checkpoint| checkpoint.epoch());
            if synced_epoch == Some(at_epoch_end.epoch) {
                let next_epoch = at_epoch_end.epoch + Epoch(1);
                if let Some(committee_info) = self
                    .epoch_manager
                    .get_local_committee_info(next_epoch)
                    .await
                    .optional()?
                {
                    resequence = Some((next_epoch, committee_info, at_epoch_end.transaction_ids));
                }
            }
        }

        self.state_store.with_write_tx(|tx| {
            let recs = self.transaction_pool.get_all(&**tx, usize::MAX)?;
            let ids = recs.iter().map(|rec| rec.id());
            let removed = self.transaction_pool.remove_all(tx, ids)?;
            let num_resequenced = match resequence {
                Some((epoch, committee_info, ids)) => {
                    self.on_receive_new_transaction
                        .resequence_known_transactions(tx, epoch, ids, &committee_info)?
                },
                None => 0,
            };
            Ok::<_, HotStuffError>(ClearedTransactionPool {
                num_removed: removed.len(),
                num_resequenced,
            })
        })
    }

    /// Drop any pooled transaction that is already committed in the substate store, returning the
    /// number removed.
    ///
    /// Unlike [`Self::clear_transaction_pool`] (the correct, wholesale response after a state sync
    /// rewrites the state store), this reconciles the pool against committed state and only removes
    /// records that are demonstrably stale — so it is safe to run on the up-to-date re-entry path,
    /// where the pool still holds genuinely-pending transactions we must keep.
    ///
    /// The persisted transaction pool is keyed by [`TransactionId`] only and is not epoch-scoped. In
    /// steady state a transaction's outputs go `UP`, its pool record is removed and it is marked
    /// finalised atomically in the same commit; but committed state applied *outside* local block-commit
    /// (e.g. a state sync, the `#2195` case) can leave the persisted pool holding already-committed
    /// transactions. Such a record can survive across a restart that re-enters consensus already
    /// up-to-date (i.e. without going through `Syncing`, which clears the pool). Re-proposing it in the
    /// new epoch breaks liveness: the leader's prepare aborts with `LockInputsFailed` because the
    /// output substates are already `UP`, replicas that removed it no-vote with `TransactionNotInPool`,
    /// no QC forms and the pacemaker loops on `NEWVIEW` forever.
    ///
    /// Every committed transaction creates a `TransactionReceipt` output substate (version 0) whose
    /// address is derived from the transaction ID, so the receipt existing in the store is a precise
    /// per-transaction commitment marker: no other transaction can create it (checking other outputs
    /// would false-positive on address collisions with a *different* committed transaction and drop a
    /// genuinely-pending record from only our pool). Note this detects commits whose receipt shard
    /// belongs to the local shard group, and aborted transactions write no receipt — neither of which
    /// produces the local output conflict that wedges the leader.
    // TODO: the transaction pool only holds pending (derived) state and should not be persisted at
    //       all — that would remove this whole class of pool-vs-state-store divergence.
    pub fn reconcile_transaction_pool(&self) -> Result<usize, HotStuffError> {
        // Scan under a read tx so we don't hold the exclusive write lock for the per-record receipt
        // lookups. `remove_all` skips missing keys, so opening a separate write tx for the removal is
        // safe even if the pool changes in between.
        let stale_ids = self.state_store.with_read_tx(|tx| {
            let recs = self.transaction_pool.get_all(tx, usize::MAX)?;
            let mut stale_ids = Vec::new();
            for rec in &recs {
                let receipt = VersionedSubstateId::for_tx_receipt(rec.to_receipt_id());
                if SubstateRecord::exists(tx, receipt.as_versioned_ref())? {
                    stale_ids.push(*rec.id());
                }
            }
            Ok::<_, HotStuffError>(stale_ids)
        })?;

        if stale_ids.is_empty() {
            return Ok(0);
        }

        self.state_store.with_write_tx(|tx| {
            let removed = self.transaction_pool.remove_all(tx, &stale_ids)?;
            Ok::<_, HotStuffError>(removed.len())
        })
    }

    async fn propose_now(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        next_height: NodeHeight,
        is_timeout: bool,
        local_claim_public_key: RistrettoPublicKeyBytes,
        propose_epoch_end: bool,
    ) -> Result<(), HotStuffError> {
        // If we're catching up, we don't propose
        if self.worker_state.is_active_catch_up() {
            info!(
                target: LOG_TARGET,
                "⤵️ [propose_now] {} Currently catching up, will not propose at height ({})",
                self.local_validator_addr,
                next_height
            );
            return Ok(());
        }
        let last_sent_vote = self
            .state_store
            .with_read_tx(|tx| LastSentVote::get(tx, epoch_state.epoch()))
            .optional()?;

        if last_sent_vote.is_some_and(|vote| vote.block_height() >= next_height) {
            info!(
                target: LOG_TARGET,
                "⤵️ [propose_now] {} Already sent vote at height ({}). Not proposing",
                self.local_validator_addr,
                next_height
            );
            return Ok(());
        }

        // The parent of the next block must be a block every honest node provably shares: either the
        // HighQC's justified block, or a deterministic dummy chain extending it. Building directly on
        // `highest_block` is correct ONLY when it *is* the HighQC (the steady-state case): a block can
        // be locally accepted without a certificate forming for it, and we still build on it so a
        // subsequent QC justifies it. But when the highest seen block sits *above* the HighQC
        // (`leaf > HighPC` — accepted locally yet never certified), we must NOT extend it on a recovery
        // proposal. Validators reconstruct the post-timeout chain from the HighQC carried in our
        // proposal and would reject a real parent at the gap height as "does not extend justify",
        // forking the committee. So on a timeout (or a height gap) we anchor on the HighQC and fill the
        // gap with dummy blocks instead — exactly what validators recompute.
        let highest_block = self
            .state_store
            .with_read_tx(|tx| HighestSeenBlock::get(tx, epoch_state.epoch()))?;

        let mut dummy_block = None;
        let mut propose_high_tc = None;
        if is_timeout || next_height > highest_block.height + NodeHeight(1) {
            let (high_qc, high_tc, justify_block) = self.state_store.with_read_tx(|tx| {
                let high_qc = HighPc::get(tx, epoch_state.epoch())?;
                let high_qc = ProposalCertificate::get(tx, high_qc.epoch(), high_qc.id())?;
                let high_tc = HighTc::get(tx, epoch_state.epoch()).optional()?;
                let high_tc = high_tc
                    .map(|tc| TimeoutCertificate::get(tx, tc.epoch(), tc.id()))
                    .transpose()?;
                let justify_block = Block::get_justified_block(tx, &high_qc, epoch_state.epoch())?;
                Ok::<_, HotStuffError>((high_qc, high_tc, justify_block))
            })?;

            // A block may only carry the timeout certificate for the view directly below it
            // (`check_timeout_certificate_precedes_block`).
            propose_high_tc = high_tc.filter(|tc| tc.height().checked_add(NodeHeight(1)) == Some(next_height));

            // Fill [justify_block.height + 1, next_height - 1] with dummy blocks anchored on the HighQC,
            // so our parent matches what every validator deterministically recomputes from the QC. When
            // next_height == justify_block.height + 1 there is no gap: the parent is the HighQC block
            // itself, which in that case is also the highest seen block, so on_propose's fallback to
            // highest_block is correct.
            if next_height > justify_block.height() + NodeHeight(1) {
                info!(
                    target: LOG_TARGET,
                    "⚠️ Leader Failure: proposing at {next_height} anchored on HighQC (height {}); highest seen is {highest_block}. Filling the gap with dummy blocks.",
                    justify_block.height(),
                );

                let skip_set = self.load_leader_skip_set(epoch_state, justify_block.height())?;
                if let Some(dummy) = calculate_last_dummy_block(
                    justify_block.height(),
                    next_height,
                    self.config.network,
                    epoch_state.epoch(),
                    justify_block.shard_group(),
                    *justify_block.id(),
                    &high_qc,
                    *justify_block.state_merkle_root(),
                    &self.leader_strategy,
                    epoch_state.local_committee(),
                    &skip_set,
                    justify_block.timestamp(),
                    *justify_block.header().accumulated_data(),
                    *justify_block.epoch_hash(),
                    justify_block.exhaust_burn_rate(),
                ) {
                    dummy_block = Some(dummy);
                }
            }
        }

        if propose_epoch_end {
            info!(
                target: LOG_TARGET,
                "🌟 Can propose end of epoch {}->{}",
                epoch_state.epoch(),
                self.next_epoch.as_ref().map(|(e, _)| e).display()
            );
        }

        let epoch_end_drain = self.epoch_end_drain(epoch_state).await?;

        self.on_propose
            .handle(
                epoch_state,
                next_height,
                local_claim_public_key,
                highest_block,
                dummy_block,
                propose_high_tc,
                propose_epoch_end,
                epoch_end_drain,
            )
            .await?;

        Ok(())
    }

    /// The drain of cross-shard transactions this node applies when proposing in `epoch_state`'s epoch. See
    /// [`EpochEndDrain`].
    async fn epoch_end_drain(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
    ) -> Result<EpochEndDrain, HotStuffError> {
        let epoch = epoch_state.epoch();
        let Some((_, known_at)) = self.next_epoch.filter(|(e, _)| *e > epoch) else {
            return Ok(EpochEndDrain::Inactive);
        };

        let changes_layout = match self.next_epoch_changes_layout {
            Some((ending, changes_layout)) if ending == epoch => changes_layout,
            _ => {
                let next_num_committees = self.epoch_manager.get_num_committees(epoch + Epoch(1)).await?;
                let changes_layout = next_num_committees != epoch_state.local_committee_info().num_committees();
                if changes_layout {
                    info!(
                        target: LOG_TARGET,
                        "🌊 The number of committees changes from {} to {next_num_committees} at the end of {epoch}. \
                         Draining cross-shard transactions before proposing EndEpoch.",
                        epoch_state.local_committee_info().num_committees(),
                    );
                }
                self.next_epoch_changes_layout = Some((epoch, changes_layout));
                changes_layout
            },
        };

        if !changes_layout {
            return Ok(EpochEndDrain::Inactive);
        }
        Ok(EpochEndDrain::after(
            Instant::now().saturating_duration_since(known_at),
            self.config.epoch_end_drain_period,
        ))
    }

    async fn dispatch_hotstuff_message(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage,
    ) -> Result<(), HotStuffError> {
        match msg {
            HotstuffMessage::NewView(message) => log_err(
                "on_receive_new_view",
                self.on_receive_new_view
                    .handle(epoch_state, current_height, from, *message)
                    .await,
            ),
            HotstuffMessage::Proposal(msg) => log_err(
                "on_receive_local_proposal",
                self.on_proposal_message(epoch_state, current_height, *msg, ProposalOrigin::Live)
                    .await,
            ),
            HotstuffMessage::ForeignProposal(msg) => log_err(
                "on_receive_foreign_proposal (received)",
                self.on_receive_foreign_proposal
                    .handle_received(msg, epoch_state.local_committee_info())
                    .await,
            ),
            HotstuffMessage::ForeignProposalNotification(msg) => log_err(
                "on_receive_foreign_proposal (notification)",
                self.on_receive_foreign_proposal
                    .handle_notification_received(from, msg, epoch_state.local_committee_info())
                    .await,
            ),
            HotstuffMessage::ForeignProposalRequest(msg) => log_err(
                "on_receive_foreign_proposal (request)",
                self.on_receive_foreign_proposal.handle_requested(from, msg).await,
            ),
            HotstuffMessage::Vote(msg) => log_err(
                "on_receive_vote",
                self.on_receive_vote
                    .handle(from, current_height, epoch_state, msg)
                    .await,
            ),
            HotstuffMessage::MissingTransactionsRequest(msg) => {
                self.on_receive_request_missing_txs.handle(from, msg);
                Ok(())
            },
            // Validation decodes a response and hands its transactions over as `RequestedTransactions`, so a
            // response is never dispatched as a message.
            HotstuffMessage::MissingTransactionsResponse(msg) => {
                warn!(
                    target: LOG_TARGET,
                    "Ignoring undecoded missing transactions response (req_id = {}) from {from}",
                    msg.request_id
                );
                Ok(())
            },
            HotstuffMessage::CatchUpSyncRequest(msg) => {
                self.on_sync_request.handle(from, epoch_state.epoch(), msg);
                Ok(())
            },
            // Catch-up blocks reach here already past the view filter (see `msg_relative_view`) and
            // are processed by the same block-import path as live proposals, without a vote;
            // `on_proposal_message` also drives the catch-up re-request loop.
            HotstuffMessage::CatchUpSyncResponse(msg) => log_err(
                "on_receive_catch_up_sync_response",
                self.on_proposal_message(epoch_state, current_height, *msg, ProposalOrigin::CatchUp)
                    .await,
            ),
        }
    }

    async fn on_proposal_message(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        msg: ProposalMessage,
        origin: ProposalOrigin,
    ) -> Result<(), HotStuffError> {
        let proposed_by = *msg.block.proposed_by();
        match log_err(
            "on_receive_local_proposal",
            self.on_receive_local_proposal.handle(epoch_state, msg, origin).await,
        ) {
            Ok(no_vote) => {
                if let Some(mut catch_up) = self.worker_state.catch_up.take() {
                    if current_height >= catch_up.expected_batch_height {
                        if catch_up.set_next_batch(current_height, self.config.catch_up_request_timeout) {
                            info!(
                                target: LOG_TARGET,
                                "⏳ Still catching up... current height: {}, expected up to: {}",
                                current_height,
                                catch_up.expected_batch_height,
                            );
                            self.on_catch_up_sync
                                .request_sync(epoch_state.epoch(), catch_up.from.clone())
                                .await?;
                            self.worker_state.catch_up = Some(catch_up);
                        } else {
                            info!(
                                target: LOG_TARGET,
                                "✅ Finished catch up at height {}. Resuming normal operation.",
                                current_height,
                            );
                        }
                    } else {
                        debug!(
                            target: LOG_TARGET,
                            "⏳ Still catching up... current height: {}, expected up to: {}",
                            current_height,
                            catch_up.expected_batch_height,
                        );
                        self.worker_state.catch_up = Some(catch_up);
                    }
                }

                match no_vote {
                    None | Some(NoVoteReason::AlreadyVotedAtHeight) => {
                        self.worker_state.has_processed_first_block = true;
                        Ok(())
                    },
                    Some(_) => {
                        // We decided NOVOTE, so we immediately send a NEWVIEW
                        self.on_leader_timeout(epoch_state, current_height, None).await
                    },
                }
            },
            Err(err) => {
                self.handle_hotstuff_error(current_height, epoch_state, Some(proposed_by), err)
                    .await
            },
        }
    }

    /// A protocol version activates on a schedule compiled into the binary, so a proposal under a version this node
    /// does not expect means this binary's schedule disagrees with the committee's. Every subsequent proposal is
    /// rejected for the same reason, so raise one loud alarm per distinct disagreement.
    fn alarm_protocol_version_disagreement(
        &mut self,
        epoch: Epoch,
        expected_version: ProtocolVersion,
        block_version: ProtocolVersion,
    ) {
        let divergence = (epoch, expected_version, block_version);
        if self.worker_state.last_protocol_version_alarm == Some(divergence) {
            return;
        }
        self.worker_state.last_protocol_version_alarm = Some(divergence);
        error!(
            target: LOG_TARGET,
            "🚨 Protocol version disagreement at {epoch}: this node's schedule expects {expected_version} but the \
             committee is proposing under {block_version}. Consensus is stalled on this node and stays stalled \
             until it runs a binary whose activation schedule matches the committee's. Upgrade this node."
        );
    }

    async fn handle_hotstuff_error(
        &mut self,
        current_height: NodeHeight,
        local_epoch_state: &EpochState<TConsensusSpec::Addr>,
        catch_up_from: Option<RistrettoPublicKeyBytes>,
        err: HotStuffError,
    ) -> Result<(), HotStuffError> {
        self.hooks.on_error(&err);
        let (remote_epoch, remote_height) = match &err {
            HotStuffError::FallenBehind {
                qc_epoch, qc_height, ..
            } => (*qc_epoch, *qc_height),
            HotStuffError::ProposalValidationError(ProposalValidationError::JustifyBlockNotFound {
                justify_block,
                ..
            }) => (justify_block.epoch(), justify_block.height()),
            HotStuffError::ProposalValidationError(ProposalValidationError::InvalidEpochHash {
                epoch,
                local_epoch_hash,
                invalid_epoch_hash,
                ..
            }) => {
                // A locked epoch hash can no longer be corrected by the oracle (see
                // EpochManager::lock_epoch), so a persistent mismatch against peers means this node
                // committed a different base-layer boundary block than the rest of the committee —
                // almost always a base-layer reorg deeper than `base_layer_confirmations` straddling
                // the epoch boundary. Consensus cannot recover from this automatically yet (epoch
                // state rollback + resync is a follow-up), so raise a single loud alarm per distinct
                // divergence rather than swallowing one warning per rejected proposal.
                let divergence = (*epoch, *local_epoch_hash, *invalid_epoch_hash);
                if self.worker_state.last_epoch_hash_alarm != Some(divergence) {
                    self.worker_state.last_epoch_hash_alarm = Some(divergence);
                    error!(
                        target: LOG_TARGET,
                        "🚨 Epoch hash divergence at {epoch}: this node committed {local_epoch_hash} but peers are \
                         proposing against {invalid_epoch_hash}. Consensus is stalled on this node. It can only \
                         recover once the rest of the committee reaches quorum on the other hash and advances past \
                         this epoch (a higher-epoch QC then triggers state sync); if the committee is split with no \
                         quorum, manual recovery is required. Likely cause: a base-layer reorg deeper than the \
                         confirmation depth at the epoch boundary."
                    );
                }
                return Ok(());
            },
            HotStuffError::ProposalValidationError(ProposalValidationError::InvalidProtocolVersion {
                expected_version,
                block_version,
                epoch,
                ..
            }) => {
                self.alarm_protocol_version_disagreement(*epoch, *expected_version, *block_version);
                return Ok(());
            },
            HotStuffError::ProposalValidationError(err) => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Proposal validation error: {err}."
                );
                // Failed validations should  not crash consensus
                return Ok(());
            },
            _ => {
                // Other errors can pass though
                return Err(err);
            },
        };

        if remote_epoch > local_epoch_state.epoch() {
            // Valid remote certificate is in a future epoch, so we are behind
            warn!(
                target: LOG_TARGET,
                "❌ Justify block {remote_epoch}/{remote_height} is in a future epoch > current epoch {}. State sync required.",
                local_epoch_state.epoch()
            );
            // Sync
            return Err(err);
        }

        if self.worker_state.is_active_catch_up() {
            warn!(
                target: LOG_TARGET,
                "⏳ Already catching up. Ignoring additional catch up."
            );
            return Ok(());
        }

        // Otherwise, catch up
        let vn = match catch_up_from {
            Some(pk) => {
                self.epoch_manager
                    .get_validator_node_by_public_key(local_epoch_state.epoch(), pk)
                    .await?
            },
            None => {
                self.epoch_manager
                    .get_random_committee_member(
                        local_epoch_state.epoch(),
                        Some(local_epoch_state.local_committee_info.shard_group()),
                        [self.local_validator_addr.clone()].into_iter().collect(),
                    )
                    .await?
            },
        };

        warn!(
            target: LOG_TARGET,
            "⚠️This node has fallen behind due to a missing justified block: {err}. Catching up"
        );

        self.on_catch_up_sync
            .request_sync(local_epoch_state.epoch(), vn.address.clone())
            .await?;

        self.worker_state.catch_up = Some(CatchUp::new(
            remote_height,
            current_height,
            self.config.catch_up_request_timeout,
            vn.address,
        ));

        Ok(())
    }

    /// The exhaust burn rate `epoch` runs at, read off the genesis block that opened it.
    ///
    /// Every block of the epoch names the same rate, and the genesis block is the one this node
    /// wrote when it entered the epoch, so this is what a proposal from a peer is held to.
    fn epoch_exhaust_burn_rate(&self, epoch: Epoch) -> Result<ExhaustBurnRate, HotStuffError> {
        let genesis = self
            .state_store
            .with_read_tx(|tx| Block::get_genesis_for_epoch(tx, epoch))?;
        Ok(genesis.exhaust_burn_rate())
    }

    fn create_genesis_block_if_required(
        &self,
        epoch: Epoch,
        epoch_hash: FixedHash,
        shard_group: ShardGroup,
    ) -> Result<bool, HotStuffError> {
        self.state_store.with_write_tx(|tx| {
            // Resolved rather than carried forward from the previous epoch's end-of-epoch block: this
            // opens an epoch for a node that may never have seen that block, because it joined by state
            // sync. The resolution depends only on the epoch and on global-shard state, which such a
            // node does have, so it lands on the value the rest of the network ratified.
            let exhaust_burn_rate = resolve_epoch_exhaust_burn_rate(&**tx, self.config.network, epoch)?;

            // The parent for genesis blocks refer to this zero block
            let mut zero_block = Block::zero_block(self.config.network, self.config.consensus_constants.num_preshards);
            if !zero_block.exists(&**tx)? {
                debug!(target: LOG_TARGET, "Creating zero block");
                zero_block.justify().save(tx)?;
                zero_block.insert(tx)?;
                zero_block.add_justify_qc(tx, &PcId::zero())?;
                zero_block.commit_block_without_state_changes(tx, &zero_block.justify().calculate_id())?;
            }

            if !Block::get_ids_by_epoch_and_height(&**tx, epoch, NodeHeight::zero())?.is_empty() {
                return Ok(false);
            }

            let state_merkle_root = ShardedStateTree::new(&**tx)
                .calculate_state_root(ProtocolVersion::at(self.config.network, epoch), shard_group)?;

            let mut genesis = Block::genesis(
                self.config.network,
                ProtocolVersion::at(self.config.network, epoch),
                epoch,
                epoch_hash,
                shard_group,
                FixedHash::new(state_merkle_root.into_array()),
                self.config.sidechain_id,
                exhaust_burn_rate,
            );
            // Warn: you cannot use genesis.exists(tx) here, because we're calculating the current state merkle root,
            // not the merkle root at height = 0.
            info!(target: LOG_TARGET, "✨Creating genesis block {genesis}");
            genesis.justify().save(tx)?;
            genesis.insert(tx)?;
            genesis.add_justify_qc(tx, &PcId::zero())?;
            genesis.as_locked().set(tx)?;
            genesis.as_leaf().set(tx)?;
            genesis.as_highest_seen().set(tx)?;
            genesis.as_last_executed().set(tx)?;
            genesis.as_last_voted().set(tx)?;
            genesis.justify().as_high_pc().set(tx)?;
            genesis.commit_block_without_state_changes(tx, &genesis.justify().calculate_id())?;

            Ok(true)
        })
    }

    fn publish_event(&self, event: HotstuffEvent) {
        let _ignore = self.tx_events.send(event);
    }
}

impl<TConsensusSpec: ConsensusSpec> Debug for HotstuffWorker<TConsensusSpec> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotstuffWorker")
            .field("validator_addr", &self.local_validator_addr)
            .field("epoch_manager", &"EpochManager")
            .field("pacemaker_handle", &self.pacemaker)
            .field("pacemaker", &"Pacemaker")
            .field("shutdown", &self.shutdown)
            .finish()
    }
}

fn log_err<T>(context: &'static str, result: Result<T, HotStuffError>) -> Result<T, HotStuffError> {
    if let Err(ref e) = result {
        error!(target: LOG_TARGET, "Error while processing new hotstuff message ({context}): {e}");
    }
    result
}

#[derive(Debug)]
struct WorkerState<TAddr> {
    /// Keeps track of the active catch up. This can be None even if a catch-up request was sent. The main use of
    /// this struct is to track timeouts and expected batch heights i.e. we've received a batch of blocks, and we need
    /// to request more.
    pub catch_up: Option<CatchUp<TAddr>>,
    pub has_processed_first_block: bool,
    /// Last (epoch, local_hash, remote_hash) we raised an epoch-hash divergence alarm for, used to
    /// emit a single loud alarm per distinct divergence instead of once per rejected proposal.
    pub last_epoch_hash_alarm: Option<(Epoch, FixedHash, FixedHash)>,
    pub last_protocol_version_alarm: Option<(Epoch, ProtocolVersion, ProtocolVersion)>,
}

impl<TAddr> WorkerState<TAddr> {
    pub fn is_active_catch_up(&self) -> bool {
        self.catch_up.as_ref().is_some_and(|c| !c.has_timed_out())
    }
}

impl<TAddr> Default for WorkerState<TAddr> {
    fn default() -> Self {
        Self {
            catch_up: None,
            has_processed_first_block: false,
            last_epoch_hash_alarm: None,
            last_protocol_version_alarm: None,
        }
    }
}

#[derive(Debug)]
struct CatchUp<TAddr> {
    pub remote_height: NodeHeight,
    pub expected_batch_height: NodeHeight,
    pub timeout_at: Instant,
    pub from: TAddr,
}

impl<TAddr> CatchUp<TAddr> {
    pub fn new(remote_height: NodeHeight, current_height: NodeHeight, timeout: Duration, from: TAddr) -> Self {
        Self {
            remote_height,
            // we get batches of 100 blocks which can only justify up to view h + 99
            expected_batch_height: (current_height + NodeHeight(99)).min(remote_height),
            timeout_at: Instant::now() + timeout,
            from,
        }
    }

    pub fn set_next_batch(&mut self, current_height: NodeHeight, timeout: Duration) -> bool {
        self.expected_batch_height = (current_height + NodeHeight(99)).min(self.remote_height);
        self.reset_timeout(timeout);
        self.expected_batch_height < self.remote_height
    }

    pub fn reset_timeout(&mut self, timeout: Duration) {
        self.timeout_at = Instant::now() + timeout;
    }

    pub fn has_timed_out(&self) -> bool {
        Instant::now() >= self.timeout_at
    }
}

/// The epoch consensus resumes in, or `None` for a node with no consensus state, which joins at the oracle's epoch.
///
/// A finalised checkpoint means its epoch ended, so consensus resumes in the next one. Without one, the persisted
/// leaf's epoch has not ended: its committee either stalled there or moved on, and state sync has already decided
/// which (a committee that moved on is synced from, leaving a checkpoint). Resuming at the leaf keeps this node on its
/// committee's chain; `seed_next_epoch_from_oracle` then drives the transition to the next epoch. Starting at the
/// oracle's epoch instead would create a genesis block the rest of the committee does not share.
fn select_starting_epoch(last_finalised_epoch: Option<Epoch>, leaf_epoch: Option<Epoch>) -> Option<Epoch> {
    let after_checkpoint = last_finalised_epoch.map(|epoch| epoch + Epoch(1));
    after_checkpoint.max(leaf_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod select_starting_epoch {
        use super::*;

        #[test]
        fn a_node_without_consensus_state_joins_at_the_oracle_epoch() {
            assert_eq!(select_starting_epoch(None, None), None);
        }

        #[test]
        fn an_unfinalised_leaf_epoch_is_resumed() {
            assert_eq!(select_starting_epoch(None, Some(Epoch(1))), Some(Epoch(1)));
        }

        #[test]
        fn the_epoch_after_a_checkpoint_is_resumed() {
            assert_eq!(select_starting_epoch(Some(Epoch(1)), None), Some(Epoch(2)));
            assert_eq!(select_starting_epoch(Some(Epoch(1)), Some(Epoch(1))), Some(Epoch(2)));
            assert_eq!(select_starting_epoch(Some(Epoch(1)), Some(Epoch(2))), Some(Epoch(2)));
        }

        #[test]
        fn a_checkpoint_synced_past_the_leaf_wins() {
            assert_eq!(select_starting_epoch(Some(Epoch(3)), Some(Epoch(1))), Some(Epoch(4)));
        }
    }
}
