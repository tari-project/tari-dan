//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp,
    fmt::{Debug, Formatter},
};

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockDiff,
        ExecutedTransaction,
        HighQc,
        LastVoted,
        LeafBlock,
        TransactionAtom,
        TransactionPool,
        TransactionRecord,
    },
    StateStore,
};
use tari_epoch_manager::{EpochManagerEvent, EpochManagerReader};
use tari_shutdown::ShutdownSignal;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{broadcast, mpsc};

use super::{
    config::HotstuffConfig,
    on_receive_requested_transactions::OnReceiveRequestedTransactions,
    proposer::Proposer,
    ProposalValidationError,
};
use crate::{
    hotstuff::{
        error::HotStuffError,
        event::HotstuffEvent,
        on_catch_up_sync::OnCatchUpSync,
        on_inbound_message::{IncomingMessageResult, OnInboundMessage},
        on_next_sync_view::OnNextSyncViewHandler,
        on_propose::OnPropose,
        on_receive_foreign_proposal::OnReceiveForeignProposalHandler,
        on_receive_local_proposal::OnReceiveLocalProposalHandler,
        on_receive_new_view::OnReceiveNewViewHandler,
        on_receive_request_missing_transactions::OnReceiveRequestMissingTransactions,
        on_receive_vote::OnReceiveVoteHandler,
        on_sync_request::OnSyncRequest,
        pacemaker::PaceMaker,
        pacemaker_handle::PaceMakerHandle,
        vote_receiver::VoteReceiver,
    },
    messages::HotstuffMessage,
    traits::{hooks::ConsensusHooks, ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::worker";

pub struct HotstuffWorker<TConsensusSpec: ConsensusSpec> {
    validator_addr: TConsensusSpec::Addr,
    network: Network,
    hooks: TConsensusSpec::Hooks,

    tx_events: broadcast::Sender<HotstuffEvent>,
    rx_new_transactions: mpsc::Receiver<(TransactionId, usize)>,

    on_inbound_message: OnInboundMessage<TConsensusSpec>,
    on_next_sync_view: OnNextSyncViewHandler<TConsensusSpec>,
    on_receive_local_proposal: OnReceiveLocalProposalHandler<TConsensusSpec>,
    on_receive_foreign_proposal: OnReceiveForeignProposalHandler<TConsensusSpec>,
    on_receive_vote: OnReceiveVoteHandler<TConsensusSpec>,
    on_receive_new_view: OnReceiveNewViewHandler<TConsensusSpec>,
    on_receive_request_missing_txs: OnReceiveRequestMissingTransactions<TConsensusSpec>,
    on_receive_requested_txs: OnReceiveRequestedTransactions<TConsensusSpec>,
    on_propose: OnPropose<TConsensusSpec>,
    on_sync_request: OnSyncRequest<TConsensusSpec>,
    on_catch_up_sync: OnCatchUpSync<TConsensusSpec>,

    state_store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,

    epoch_manager: TConsensusSpec::EpochManager,
    pacemaker_worker: Option<PaceMaker>,
    pacemaker: PaceMakerHandle,
    shutdown: ShutdownSignal,
}
impl<TConsensusSpec: ConsensusSpec> HotstuffWorker<TConsensusSpec> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        validator_addr: TConsensusSpec::Addr,
        network: Network,
        inbound_messaging: TConsensusSpec::InboundMessaging,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        rx_new_transactions: mpsc::Receiver<(TransactionId, usize)>,
        state_store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        signing_service: TConsensusSpec::SignatureService,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        transaction_executor: TConsensusSpec::TransactionExecutor,
        tx_events: broadcast::Sender<HotstuffEvent>,
        tx_mempool: mpsc::UnboundedSender<Transaction>,
        hooks: TConsensusSpec::Hooks,
        shutdown: ShutdownSignal,
        config: HotstuffConfig,
    ) -> Self {
        let pacemaker = PaceMaker::new();
        let vote_receiver = VoteReceiver::new(
            network,
            state_store.clone(),
            leader_strategy.clone(),
            epoch_manager.clone(),
            signing_service.clone(),
            pacemaker.clone_handle(),
        );
        let proposer =
            Proposer::<TConsensusSpec>::new(state_store.clone(), epoch_manager.clone(), outbound_messaging.clone());
        Self {
            validator_addr: validator_addr.clone(),
            network,
            tx_events: tx_events.clone(),
            rx_new_transactions,

            on_inbound_message: OnInboundMessage::new(
                validator_addr.clone(),
                network,
                config,
                state_store.clone(),
                epoch_manager.clone(),
                leader_strategy.clone(),
                signing_service.clone(),
                inbound_messaging,
                outbound_messaging.clone(),
                transaction_pool.clone(),
                tx_events.clone(),
                hooks.clone(),
            ),

            on_next_sync_view: OnNextSyncViewHandler::new(
                state_store.clone(),
                outbound_messaging.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
            ),
            on_receive_local_proposal: OnReceiveLocalProposalHandler::new(
                validator_addr,
                state_store.clone(),
                epoch_manager.clone(),
                leader_strategy.clone(),
                pacemaker.clone_handle(),
                outbound_messaging.clone(),
                signing_service.clone(),
                transaction_pool.clone(),
                tx_events,
                proposer.clone(),
                transaction_executor.clone(),
                network,
                hooks.clone(),
            ),
            on_receive_foreign_proposal: OnReceiveForeignProposalHandler::new(
                state_store.clone(),
                epoch_manager.clone(),
                transaction_pool.clone(),
                pacemaker.clone_handle(),
            ),
            on_receive_vote: OnReceiveVoteHandler::new(vote_receiver.clone()),
            on_receive_new_view: OnReceiveNewViewHandler::new(
                network,
                state_store.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
                pacemaker.clone_handle(),
                vote_receiver,
            ),
            on_receive_request_missing_txs: OnReceiveRequestMissingTransactions::new(
                state_store.clone(),
                outbound_messaging.clone(),
            ),
            on_receive_requested_txs: OnReceiveRequestedTransactions::new(tx_mempool),
            on_propose: OnPropose::new(
                network,
                state_store.clone(),
                epoch_manager.clone(),
                transaction_pool.clone(),
                transaction_executor,
                signing_service,
                outbound_messaging.clone(),
            ),

            on_sync_request: OnSyncRequest::new(state_store.clone(), outbound_messaging.clone()),
            on_catch_up_sync: OnCatchUpSync::new(
                state_store.clone(),
                pacemaker.clone_handle(),
                outbound_messaging,
                epoch_manager.clone(),
            ),

            state_store,
            leader_strategy,
            epoch_manager,
            transaction_pool,

            pacemaker: pacemaker.clone_handle(),
            pacemaker_worker: Some(pacemaker),
            hooks,
            shutdown,
        }
    }

    pub async fn start(&mut self) -> Result<(), HotStuffError> {
        self.create_zero_block_if_required()?;
        let (current_height, high_qc) = self.state_store.with_read_tx(|tx| {
            let leaf = LeafBlock::get(tx)?;
            let last_voted = LastVoted::get(tx)?;
            Ok::<_, HotStuffError>((cmp::max(leaf.height(), last_voted.height()), HighQc::get(tx)?))
        })?;

        let current_epoch = self.epoch_manager.current_epoch().await?;
        info!(
            target: LOG_TARGET,
            "🚀 Pacemaker starting for epoch {}, height: {}, high_qc: {}",
            current_epoch,
            current_height,
            high_qc
        );

        self.pacemaker
            .start(current_epoch, current_height, high_qc.block_height())
            .await?;

        self.run().await?;
        Ok(())
    }

    async fn run(&mut self) -> Result<(), HotStuffError> {
        // Spawn pacemaker if not spawned already
        if let Some(pm) = self.pacemaker_worker.take() {
            pm.spawn();
        }

        let mut on_beat = self.pacemaker.get_on_beat();
        let mut on_force_beat = self.pacemaker.get_on_force_beat();
        let mut on_leader_timeout = self.pacemaker.get_on_leader_timeout();

        let mut epoch_manager_events = self.epoch_manager.subscribe().await?;

        self.request_initial_catch_up_sync().await?;

        let mut prev_height = self.pacemaker.current_view().get_height();
        loop {
            let current_height = self.pacemaker.current_view().get_height() + NodeHeight(1);
            let current_epoch = self.pacemaker.current_view().get_epoch();

            if current_height != prev_height {
                self.hooks.on_pacemaker_height_changed(current_height);
                prev_height = current_height;
            }

            debug!(
                target: LOG_TARGET,
                "🔥 Current height #{}",
                current_height.as_u64()
            );

            tokio::select! {
                msg_or_sync = self.on_inbound_message.next_message(current_epoch, current_height) => {
                    if let Err(e) = self.dispatch_hotstuff_message(msg_or_sync).await {
                        self.on_failure("on_new_hs_message", &e).await;
                        return Err(e);
                    }
                },

                Some((tx_id, pending)) = self.rx_new_transactions.recv() => {
                    if let Err(err) = self.on_new_transaction(tx_id, pending, current_height).await {
                        self.hooks.on_error(&err);
                        error!(target: LOG_TARGET, "Error handling new transaction: {}", err);
                    }
                },

                Ok(event) = epoch_manager_events.recv() => {
                    self.on_epoch_manager_event(event).await?;
                },

                _ = on_beat.wait() => {
                    if let Err(e) = self.on_beat(current_epoch).await {
                        self.on_failure("on_beat", &e).await;
                        return Err(e);
                    }
                },

                maybe_leaf_block = on_force_beat.wait() => {
                    self.hooks.on_beat();
                    if let Err(e) = self.propose_if_leader(current_epoch, maybe_leaf_block).await {
                        self.on_failure("propose_if_leader", &e).await;
                        return Err(e);
                    }
                },

                new_height = on_leader_timeout.wait() => {
                    if let Err(e) = self.on_leader_timeout(new_height).await {
                        self.on_failure("on_leader_timeout", &e).await;
                        return Err(e);
                    }
                },

                _ = self.shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }

        self.on_receive_new_view.clear_new_views();
        self.on_inbound_message.clear_buffer();
        // This only happens if we're shutting down.
        if let Err(err) = self.pacemaker.stop().await {
            debug!(target: LOG_TARGET, "Pacemaker channel dropped: {}", err);
        }

        Ok(())
    }

    async fn on_new_transaction(
        &mut self,
        tx_id: TransactionId,
        num_pending_txs: usize,
        current_height: NodeHeight,
    ) -> Result<(), HotStuffError> {
        let exists = self.state_store.with_write_tx(|tx| {
            if self.transaction_pool.exists(&**tx, &tx_id)? {
                return Ok(true);
            }
            let transaction = TransactionRecord::get(&**tx, &tx_id)?;
            // Did the mempool execute it?
            if transaction.is_executed() {
                // This should never fail
                let executed = ExecutedTransaction::try_from(transaction)?;
                self.transaction_pool.insert(tx, executed.to_atom())?;
            } else {
                debug!(
                    target: LOG_TARGET,
                    "🔥 New transaction {tx_id} is deferred (not executed yet)",
                );
                // Deferred execution
                self.transaction_pool
                    .insert(tx, TransactionAtom::deferred(*transaction.id()))?;
            }
            Ok::<_, HotStuffError>(false)
        })?;

        debug!(
            target: LOG_TARGET,
            "🔥 new transaction ready for consensus: {} ({} pending, already exists = {})",
            tx_id,
            num_pending_txs,
            exists
        );

        if !exists {
            self.hooks.on_transaction_ready(&tx_id);
        }

        if let Err(err) = self
            .on_inbound_message
            .update_parked_blocks(current_height, &tx_id)
            .await
        {
            self.hooks.on_error(&err);
            error!(target: LOG_TARGET, "Error checking parked blocks: {}", err);
        }
        // There are num_pending_txs transactions in the queue. If we have no pending transactions, we'll propose now if
        // able.
        if !exists && num_pending_txs == 0 {
            self.pacemaker.beat();
        }

        Ok(())
    }

    async fn on_epoch_manager_event(&mut self, event: EpochManagerEvent) -> Result<(), HotStuffError> {
        match event {
            EpochManagerEvent::EpochChanged(epoch) => {
                if !self.epoch_manager.is_this_validator_registered_for_epoch(epoch).await? {
                    info!(
                        target: LOG_TARGET,
                        "💤 This validator is not registered for epoch {}. Going to sleep.", epoch
                    );
                    return Err(HotStuffError::NotRegisteredForCurrentEpoch { epoch });
                }

                // Edge case: we have started a VN and have progressed a few epochs quickly and have no blocks in
                // previous epochs to update the current view. This only really applies when mining is
                // instant (localnet)
                let leaf_block = self.state_store.with_read_tx(|tx| LeafBlock::get(tx))?;
                if leaf_block.is_genesis() {
                    self.pacemaker.update_epoch(epoch).await?;
                }

                // TODO: This is breaking my testing right now (division by zero, from time to time)
                // Send the last vote to the leader at the next epoch so that they can justify the current tip.
                // if let Some(last_voted) = self.state_store.with_read_tx(|tx| LastSentVote::get(tx)).optional()? {
                //     info!(
                //         target: LOG_TARGET,
                //         "💌 Sending last vote to the leader at epoch {}: {}",
                //         epoch,
                //         last_voted
                //     );
                //     let local_committee = self.epoch_manager.get_local_committee(epoch).await?;
                //     let leader = self
                //         .leader_strategy
                //         .get_leader_for_next_block(&local_committee, last_voted.block_height);
                //     self.outbound_messaging
                //         .send(leader.clone(), HotstuffMessage::Vote(last_voted.into()))
                //         .await?;
                // }
            },
            EpochManagerEvent::ThisValidatorIsRegistered { .. } => {},
        }

        Ok(())
    }

    async fn request_initial_catch_up_sync(&mut self) -> Result<(), HotStuffError> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        let committee = self.epoch_manager.get_local_committee(current_epoch).await?;
        for member in committee.shuffled() {
            if *member != self.validator_addr {
                self.on_catch_up_sync.request_sync(current_epoch, member).await?;
                break;
            }
        }
        Ok(())
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
        self.on_receive_new_view.clear_new_views();
        self.on_inbound_message.clear_buffer();
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

    async fn on_leader_timeout(&mut self, new_height: NodeHeight) -> Result<(), HotStuffError> {
        self.hooks.on_leader_timeout(new_height);
        self.on_next_sync_view.handle(new_height).await?;
        self.publish_event(HotstuffEvent::LeaderTimeout { new_height });
        Ok(())
    }

    async fn on_beat(&mut self, epoch: Epoch) -> Result<(), HotStuffError> {
        self.hooks.on_beat();
        if !self
            .state_store
            .with_read_tx(|tx| self.transaction_pool.has_uncommitted_transactions(tx))?
        {
            let current_epoch = self.epoch_manager.current_epoch().await?;
            // Propose quickly if we should end the epoch
            if current_epoch == epoch {
                debug!(target: LOG_TARGET, "[on_beat] No transactions to propose. Waiting for a timeout.");
                return Ok(());
            }
        }

        self.propose_if_leader(epoch, None).await?;

        Ok(())
    }

    async fn propose_if_leader(&mut self, epoch: Epoch, leaf_block: Option<LeafBlock>) -> Result<(), HotStuffError> {
        let is_newview_propose = leaf_block.is_some();
        let leaf_block = match leaf_block {
            Some(leaf_block) => leaf_block,
            None => self.state_store.with_read_tx(|tx| LeafBlock::get(tx))?,
        };
        // let locked_block = self
        //     .state_store
        //     .with_read_tx(|tx| LockedBlock::get(tx)?.get_block(tx))?;
        // let current_epoch = self.epoch_manager.current_epoch().await?;
        // let epoch = if locked_block.is_epoch_end() || locked_block.is_genesis() {
        //     current_epoch
        // } else {
        //     locked_block.epoch()
        // };
        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;

        let is_leader =
            self.leader_strategy
                .is_leader_for_next_block(&self.validator_addr, &local_committee, leaf_block.height);
        info!(
            target: LOG_TARGET,
            "🔥 [on_beat{}] {} Is leader: {:?}, leaf_block: {}, local_committee: {}",
            if is_newview_propose { " (NEWVIEW)"} else { "" },
            self.validator_addr,
            is_leader,
            leaf_block,
            local_committee
                .len(),
        );
        if is_leader {
            let current_epoch = self.epoch_manager.current_epoch().await?;
            let propose_epoch_end = current_epoch > epoch;

            self.on_propose
                .handle(
                    epoch,
                    &local_committee,
                    leaf_block,
                    is_newview_propose,
                    propose_epoch_end,
                )
                .await?;
        } else {
            // We can make this a warm/error in future, but for now I want to be sure this never happens
            debug_assert!(
                !is_newview_propose,
                "propose_if_leader called with is_newview_propose=true but we're not the leader"
            );
        }
        Ok(())
    }

    async fn dispatch_hotstuff_message(
        &mut self,
        result: IncomingMessageResult<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let (from, msg) = match result {
            Ok(Some(msg)) => msg,
            Ok(None) => {
                warn!(target: LOG_TARGET, "🚨Incoming message stream has closed");
                return Ok(());
            },
            Err(err) => {
                return Err(err);
            },
        };
        // let (from, msg) = match result {
        //     Ok(Some(msg)) => msg,
        //     Ok(None) => return Ok(()),
        //     Err(NeedsSync {
        //         from,
        //         local_height,
        //         qc_height,
        //         remote_epoch,
        //         local_epoch,
        //     }) => {
        //         self.hooks.on_needs_sync(local_height, qc_height);
        //         if remote_epoch > local_epoch {
        //             warn!(
        //                 target: LOG_TARGET,
        //                 "⚠️ Node is behind by more than an epoch from peer {} (local epoch: {}, height: {}, qc height:
        // {})",                 from,
        //                 local_epoch,
        //                 local_height,
        //                 qc_height
        //             );
        //             return Err(HotStuffError::FallenBehind {
        //                 local_height,
        //                 qc_height,
        //             });
        //         }
        //         self.on_catch_up_sync.request_sync(&from).await?;
        //         return Ok(());
        //     },
        // };

        // if !self
        //     .epoch_manager
        //     .is_this_validator_registered_for_epoch(msg.epoch())
        //     .await?
        // {
        //     warn!(
        //         target: LOG_TARGET,
        //         "Received message for inactive epoch: {}", msg.epoch()
        //     );
        //     return Ok(());
        // }

        // TODO: check the message comes from a local committee member (except foreign proposals which must come from a
        //       registered node)
        match msg {
            HotstuffMessage::NewView(message) => log_err(
                "on_receive_new_view",
                self.on_receive_new_view.handle(from, message).await,
            ),
            HotstuffMessage::Proposal(msg) => {
                let current_view = self.pacemaker.current_view().clone();
                match log_err(
                    "on_receive_local_proposal",
                    self.on_receive_local_proposal.handle(current_view, msg).await,
                ) {
                    Ok(_) => Ok(()),
                    Err(
                        err @ HotStuffError::ProposalValidationError(ProposalValidationError::JustifyBlockNotFound {
                            ..
                        }),
                    ) => {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️This node has fallen behind due to a missing justified block: {err}"
                        );
                        let current_epoch = self.epoch_manager.current_epoch().await?;
                        self.on_catch_up_sync.request_sync(current_epoch, &from).await?;
                        Ok(())
                    },
                    Err(err) => return Err(err),
                }
            },
            HotstuffMessage::ForeignProposal(msg) => log_err(
                "on_receive_foreign_proposal",
                self.on_receive_foreign_proposal.handle(from, msg).await,
            ),
            HotstuffMessage::Vote(msg) => log_err("on_receive_vote", self.on_receive_vote.handle(from, msg).await),
            HotstuffMessage::RequestMissingTransactions(msg) => log_err(
                "on_receive_request_missing_transactions",
                self.on_receive_request_missing_txs.handle(from, msg).await,
            ),
            HotstuffMessage::RequestedTransaction(msg) => log_err(
                "on_receive_requested_txs",
                self.on_receive_requested_txs.handle(from, msg).await,
            ),
            HotstuffMessage::CatchUpSyncRequest(msg) => {
                self.on_sync_request.handle(from, msg);
                Ok(())
            },
            HotstuffMessage::SyncResponse(_) => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Ignoring unrequested SyncResponse from {}",from
                );
                Ok(())
            },
        }
    }

    fn create_zero_block_if_required(&self) -> Result<(), HotStuffError> {
        self.state_store.with_write_tx(|tx| {
            // The parent for genesis blocks refer to this zero block
            let zero_block = Block::zero_block(self.network);
            if !zero_block.exists(&**tx)? {
                debug!(target: LOG_TARGET, "Creating zero block");
                zero_block.justify().insert(tx)?;
                zero_block.insert(tx)?;
                zero_block.as_locked_block().set(tx)?;
                zero_block.as_leaf_block().set(tx)?;
                zero_block.as_last_executed().set(tx)?;
                zero_block.as_last_voted().set(tx)?;
                zero_block.justify().as_high_qc().set(tx)?;
                zero_block.commit_diff(tx, BlockDiff::empty(*zero_block.id()))?;
            }

            Ok(())
        })
    }

    fn publish_event(&self, event: HotstuffEvent) {
        let _ignore = self.tx_events.send(event);
    }
}

impl<TConsensusSpec: ConsensusSpec> Debug for HotstuffWorker<TConsensusSpec> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotstuffWorker")
            .field("validator_addr", &self.validator_addr)
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
