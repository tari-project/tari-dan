//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Debug, Formatter},
    iter,
    time::{Duration, Instant},
};

use log::*;
use tari_common_types::types::FixedHash;
use tari_consensus_types::{
    HighPc,
    HighTc,
    HighestSeenBlock,
    LastProposed,
    LastSentVote,
    LeafBlock,
    PcId,
    ProposalCertificate,
    TimeoutCertificate,
};
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
        BookkeepingModel,
        EpochCheckpoint,
        ForeignProposalRecord,
        NoVoteReason,
        SubstateRecord,
        TransactionPool,
        TransactionRecord,
    },
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_shutdown::ShutdownSignal;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::{
    sync::{broadcast, mpsc},
    time,
};

use super::{
    ProposalValidationError,
    calculate_last_dummy_block,
    config::HotstuffConfig,
    get_highest_seen_justified_view,
    on_receive_new_transaction::OnReceiveNewTransaction,
};
use crate::{
    hotstuff::{
        epoch_gc::EpochGc,
        epoch_state::EpochState,
        error::HotStuffError,
        event::HotstuffEvent,
        on_catch_up_sync::OnCatchUpSync,
        on_catch_up_sync_request::OnSyncRequest,
        on_inbound_message::OnInboundMessage,
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
        transaction_manager::ConsensusTransactionManager,
        vote_collector::{ProposalVoteCollector, TimeoutVoteCollector},
    },
    messages::{HotstuffMessage, ProposalMessage},
    tracing::TraceTimer,
    traits::{CertificateStore, ConsensusSpec, LeaderStrategy, PeriodicTask, hooks::ConsensusHooks},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::worker";

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
    leader_strategy: TConsensusSpec::LeaderStrategy,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,

    /// Epoch to switch to (Epoch, activated_at)
    next_epoch: Option<(Epoch, Instant)>,
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
        let proposal_vote_collector =
            ProposalVoteCollector::new(state_store.clone(), epoch_manager.clone(), signing_service.clone());
        let timeout_vote_collector =
            TimeoutVoteCollector::new(state_store.clone(), epoch_manager.clone(), signing_service.clone());
        let transaction_manager = ConsensusTransactionManager::new(
            transaction_executor.clone(),
            config.consensus_constants.max_transaction_validity_epochs,
        );

        Self {
            local_validator_addr: local_validator_addr.clone(),

            config: config.clone(),
            rx_new_transactions,
            rx_missing_transactions,

            on_inbound_message: OnInboundMessage::new(
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
                leader_strategy.clone(),
                signing_service.clone(),
                outbound_messaging.clone(),
                tx_events.downgrade(),
            ),

            on_next_sync_view: OnNextSyncViewHandler::new(
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
                pacemaker.clone_handle(),
                proposal_vote_collector.clone(),
                local_validator_addr.clone(),
                leader_strategy.clone(),
            ),
            on_receive_new_view: OnReceiveNewViewHandler::new(
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
            leader_strategy,
            epoch_manager,
            transaction_pool,

            next_epoch: None,
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

    async fn get_starting_epoch(&self) -> Result<(Epoch, FixedHash), HotStuffError> {
        // NOTE: we assume the latest checkpoint has been synced already
        let checkpoint = self
            .state_store
            .with_read_tx(|tx| EpochCheckpoint::get_last_checkpoint(tx))
            .optional()?;

        let last_finalised_epoch = checkpoint.map(|ch| ch.epoch());

        let current_epoch = {
            // Typically, the current epoch = last finalised epoch + 1. Unless, the epoch was not finalised yet at
            // startup.
            match last_finalised_epoch {
                Some(epoch) => epoch + Epoch(1),
                None => self.epoch_manager.current_epoch().await?,
            }
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
        // checkpoint was written. Detect that here and seed the deferred-EOE state so the next
        // EpochChanged event (or this startup if the oracle is already current) finishes the
        // transition.
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
                epoch_state
                    .update_from_epoch_manager(&self.epoch_manager, current_epoch)
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

                Some(result) = self.on_inbound_message.next_message(epoch_state.epoch(), current_height, self.worker_state.has_processed_first_block) => {
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
        result: Result<(TConsensusSpec::Addr, HotstuffMessage), HotStuffError>,
    ) -> Result<(), HotStuffError> {
        let (from, msg) = result?;

        match self
            .on_message_validate
            .handle(
                current_height,
                epoch_state,
                from.clone(),
                msg,
                &self.on_receive_new_transaction,
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
        let (local_proposals, foreign_proposals) = self
            .on_message_validate
            .update_parked_blocks(current_height, transaction_ids)?;

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

        for msg in local_proposals {
            if let Err(e) = self.on_proposal_message(epoch_state, current_height, msg).await {
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
    /// behind, the resume will re-defer and `on_epoch_manager_event` will retry later.
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
                    let state_root = *block.state_merkle_root();
                    return Ok(Some((block, commit_qc, state_root)));
                }
                if block.height().is_zero() || block.parent().is_zero() {
                    return Ok(None);
                }
                block = block.get_parent(tx)?;
            }
        })?;

        if let Some((eoe_block, commit_qc, state_root)) = pending {
            info!(
                target: LOG_TARGET,
                "🔄 Detected unprocessed EOE block {} on startup. Seeding deferred end-of-epoch state.",
                eoe_block.id()
            );
            self.on_receive_local_proposal
                .set_pending_end_of_epoch(eoe_block, commit_qc, state_root);
            // Attempt to complete immediately. If the oracle is still behind, this will re-defer
            // and a later EpochChanged event will retry.
            let _ = self.on_receive_local_proposal.try_resume_pending_end_of_epoch().await?;
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
                if self.on_receive_local_proposal.has_pending_end_of_epoch() &&
                    let Err(err) = self.on_receive_local_proposal.try_resume_pending_end_of_epoch().await
                {
                    error!(
                        target: LOG_TARGET,
                        "Failed to resume deferred end-of-epoch processing for {epoch}: {err}"
                    );
                    return Err(err);
                }
            },
        }

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
        self.hooks.on_error(err);
        self.publish_event(HotstuffEvent::Failure {
            message: err.to_string(),
        });
        error!(target: LOG_TARGET, "Error ({}): {}", context, err);
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

    /// Called when it may be time to propose if this node is the leader for the next view
    async fn on_beat(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        local_claim_public_key: &RistrettoPublicKeyBytes,
    ) -> Result<(), HotStuffError> {
        let (highest_justified, last_proposed) = self.state_store.with_read_tx(|tx| {
            let highest_height = get_highest_seen_justified_view(tx, epoch_state.epoch())?;
            let last_proposed = LastProposed::get(tx, epoch_state.epoch()).optional()?;
            Ok::<_, HotStuffError>((highest_height, last_proposed))
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
        if !self.leader_strategy.is_leader(
            &self.local_validator_addr,
            epoch_state.local_committee(),
            highest_justified,
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

        let propose_now = self.state_store.with_read_tx(|tx| {
            let highest_block = HighestSeenBlock::get(tx, epoch_state.epoch())?;
            // Propose quickly if there are UTXOs to mint or transactions to propose
            let propose_now = ForeignProposalRecord::has_unconfirmed(tx, epoch_state.epoch())? ||
                self.transaction_pool
                    .has_ready_or_pending_transaction_updates(tx, highest_block.block_id())?;

            Ok::<_, HotStuffError>(propose_now)
        })?;

        let propose_epoch_end = self.can_propose_epoch_end(epoch_state);
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

        // `height` is the highest justified view - check if this node is the leader (i.e. should propose to advance the
        // view to h + 1)
        let is_leader =
            self.leader_strategy
                .is_leader(&self.local_validator_addr, epoch_state.local_committee(), height);

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

    /// Remove all records from the transaction pool, returning the number removed.
    ///
    /// Called after a state sync. State sync only rewrites the state store, not the (persisted)
    /// consensus transaction pool, so the pool can be left holding transactions the freshly-synced
    /// state has already finalised. Re-proposing such a transaction breaks liveness: the rest of the
    /// committee finalised and removed it, so it can never gather a QC again and every block carrying
    /// it is rejected (`TransactionNotInPool`). Clearing lets the node rebuild its pool from fresh
    /// mempool/consensus activity against the synced state.
    // TODO: the transaction pool only holds pending (derived) state and should not be persisted at
    //       all — that would remove this whole class of pool-vs-state-store divergence.
    pub fn clear_transaction_pool(&self) -> Result<usize, HotStuffError> {
        self.state_store.with_write_tx(|tx| {
            let recs = self.transaction_pool.get_all(&**tx, usize::MAX)?;
            let ids = recs.iter().map(|rec| rec.id());
            let removed = self.transaction_pool.remove_all(tx, ids)?;
            Ok::<_, HotStuffError>(removed.len())
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

            propose_high_tc = high_tc;

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
                    justify_block.timestamp(),
                    *justify_block.header().accumulated_data(),
                    *justify_block.epoch_hash(),
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

        self.on_propose
            .handle(
                epoch_state,
                next_height,
                local_claim_public_key,
                highest_block,
                dummy_block,
                propose_high_tc,
                propose_epoch_end,
            )
            .await?;

        Ok(())
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
                self.on_proposal_message(epoch_state, current_height, *msg).await,
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
                    .handle_notification_received(from, epoch_state.epoch(), msg, epoch_state.local_committee_info())
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
            HotstuffMessage::MissingTransactionsRequest(msg) => log_err(
                "on_receive_request_missing_transactions",
                self.on_receive_request_missing_txs.handle(from, msg).await,
            ),
            HotstuffMessage::MissingTransactionsResponse(msg) => log_err(
                "on_receive_new_transaction",
                self.on_receive_new_transaction
                    .process_requested(epoch_state.epoch(), from, msg, epoch_state.local_committee_info())
                    .await,
            ),
            HotstuffMessage::CatchUpSyncRequest(msg) => {
                self.on_sync_request.handle(from, epoch_state.epoch(), msg);
                Ok(())
            },
            // Catch-up blocks reach here already past the view filter (see `msg_relative_view`) and
            // are processed by the same block-import path as live proposals; `on_proposal_message`
            // also drives the catch-up re-request loop.
            HotstuffMessage::CatchUpSyncResponse(msg) => log_err(
                "on_receive_catch_up_sync_response",
                self.on_proposal_message(epoch_state, current_height, *msg).await,
            ),
        }
    }

    async fn on_proposal_message(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        msg: ProposalMessage,
    ) -> Result<(), HotStuffError> {
        let proposed_by = *msg.block.proposed_by();
        match log_err(
            "on_receive_local_proposal",
            self.on_receive_local_proposal.handle(epoch_state, msg).await,
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

    fn create_genesis_block_if_required(
        &self,
        epoch: Epoch,
        epoch_hash: FixedHash,
        shard_group: ShardGroup,
    ) -> Result<bool, HotStuffError> {
        self.state_store.with_write_tx(|tx| {
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

            let state_merkle_root = ShardedStateTree::new(&**tx).calculate_state_root(shard_group)?;

            let mut genesis = Block::genesis(
                self.config.network,
                ProtocolVersion::at(self.config.network, epoch),
                epoch,
                epoch_hash,
                shard_group,
                FixedHash::new(state_merkle_root.into_array()),
                self.config.sidechain_id,
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
