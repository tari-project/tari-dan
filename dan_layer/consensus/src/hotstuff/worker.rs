//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Debug, Formatter};

use log::*;
use tari_dan_common_types::{committee::CommitteeInfo, Epoch, NodeHeight, ShardGroup};
use tari_dan_storage::{
    consensus_models::{Block, BlockDiff, BurntUtxo, HighQc, LeafBlock, TransactionPool},
    StateStore,
};
use tari_epoch_manager::{EpochManagerEvent, EpochManagerReader};
use tari_shutdown::ShutdownSignal;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{broadcast, mpsc};

use super::{config::HotstuffConfig, on_receive_new_transaction::OnReceiveNewTransaction, ProposalValidationError};
use crate::{
    hotstuff::{
        error::HotStuffError,
        event::HotstuffEvent,
        on_catch_up_sync::OnCatchUpSync,
        on_inbound_message::OnInboundMessage,
        on_message_validate::{MessageValidationResult, OnMessageValidate},
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
        transaction_manager::ConsensusTransactionManager,
        vote_receiver::VoteReceiver,
    },
    messages::HotstuffMessage,
    traits::{hooks::ConsensusHooks, ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::worker";

pub struct HotstuffWorker<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,
    config: HotstuffConfig,
    hooks: TConsensusSpec::Hooks,

    tx_events: broadcast::Sender<HotstuffEvent>,
    rx_new_transactions: mpsc::Receiver<(Transaction, usize)>,
    rx_missing_transactions: mpsc::UnboundedReceiver<TransactionId>,

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
        config: HotstuffConfig,
        local_validator_addr: TConsensusSpec::Addr,
        inbound_messaging: TConsensusSpec::InboundMessaging,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        rx_new_transactions: mpsc::Receiver<(Transaction, usize)>,
        state_store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        signing_service: TConsensusSpec::SignatureService,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        transaction_executor: TConsensusSpec::TransactionExecutor,
        tx_events: broadcast::Sender<HotstuffEvent>,
        hooks: TConsensusSpec::Hooks,
        shutdown: ShutdownSignal,
    ) -> Self {
        let (tx_missing_transactions, rx_missing_transactions) = mpsc::unbounded_channel();
        let pacemaker = PaceMaker::new(config.pacemaker_max_base_time);
        let vote_receiver = VoteReceiver::new(
            config.network,
            state_store.clone(),
            leader_strategy.clone(),
            epoch_manager.clone(),
            signing_service.clone(),
            pacemaker.clone_handle(),
        );
        let transaction_manager = ConsensusTransactionManager::new(transaction_executor.clone());

        Self {
            local_validator_addr,
            config: config.clone(),
            tx_events: tx_events.clone(),
            rx_new_transactions,
            rx_missing_transactions,

            on_inbound_message: OnInboundMessage::new(inbound_messaging, hooks.clone()),
            on_message_validate: OnMessageValidate::new(
                config.clone(),
                state_store.clone(),
                epoch_manager.clone(),
                leader_strategy.clone(),
                signing_service.clone(),
                outbound_messaging.clone(),
                tx_events.clone(),
            ),

            on_next_sync_view: OnNextSyncViewHandler::new(
                state_store.clone(),
                outbound_messaging.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
            ),
            on_receive_local_proposal: OnReceiveLocalProposalHandler::new(
                state_store.clone(),
                epoch_manager.clone(),
                leader_strategy.clone(),
                pacemaker.clone_handle(),
                outbound_messaging.clone(),
                signing_service.clone(),
                transaction_pool.clone(),
                tx_events,
                transaction_manager.clone(),
                config.clone(),
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
                config.network,
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
            on_receive_new_transaction: OnReceiveNewTransaction::new(
                state_store.clone(),
                transaction_pool.clone(),
                transaction_executor.clone(),
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

    pub fn pacemaker(&self) -> &PaceMakerHandle {
        &self.pacemaker
    }

    pub async fn start(&mut self) -> Result<(), HotStuffError> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        let local_committee_info = self.epoch_manager.get_local_committee_info(current_epoch).await?;

        self.create_zero_block_if_required(current_epoch, local_committee_info.shard_group())?;

        // Resume pacemaker from the last epoch/height
        let (current_epoch, current_height, high_qc) = self.state_store.with_read_tx(|tx| {
            let leaf = LeafBlock::get(tx)?;
            let current_epoch = Some(leaf.epoch()).filter(|e| !e.is_zero()).unwrap_or(current_epoch);
            let current_height = Some(leaf.height())
                .filter(|h| !h.is_zero())
                .unwrap_or_else(NodeHeight::zero);

            Ok::<_, HotStuffError>((current_epoch, current_height, HighQc::get(tx)?))
        })?;

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

        self.run(local_committee_info).await?;
        Ok(())
    }

    async fn run(&mut self, mut local_committee_info: CommitteeInfo) -> Result<(), HotStuffError> {
        // Spawn pacemaker if not spawned already
        if let Some(pm) = self.pacemaker_worker.take() {
            pm.spawn();
        }

        let mut on_beat = self.pacemaker.get_on_beat();
        let mut on_force_beat = self.pacemaker.get_on_force_beat();
        let mut on_leader_timeout = self.pacemaker.get_on_leader_timeout();

        let mut epoch_manager_events = self.epoch_manager.subscribe().await?;

        let mut prev_height = self.pacemaker.current_view().get_height();
        let current_epoch = self.pacemaker.current_view().get_epoch();
        self.request_initial_catch_up_sync(current_epoch).await?;
        let mut prev_epoch = current_epoch;

        loop {
            let current_height = self.pacemaker.current_view().get_height() + NodeHeight(1);
            let current_epoch = self.pacemaker.current_view().get_epoch();

            // Need to update local committee info if the epoch has changed
            // TODO: we should exit consensus when we change epochs to ensure that we are synced. When this is
            // implemented, we will not need to do this.
            if prev_epoch != current_epoch {
                local_committee_info = self.epoch_manager.get_local_committee_info(current_epoch).await?;
                prev_epoch = current_epoch;
            }

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
                Some(result) = self.on_inbound_message.next_message(current_epoch, current_height) => {
                    if let Err(err) = self.on_unvalidated_message(current_epoch, current_height, result, &local_committee_info).await {
                        self.hooks.on_error(&err);
                        error!(target: LOG_TARGET, "🚨Error handling new message: {}", err);
                    }
                },

                Some((tx_id, pending)) = self.rx_new_transactions.recv() => {
                    if let Err(err) = self.on_new_transaction(tx_id, pending, current_epoch, current_height, &local_committee_info).await {
                        self.hooks.on_error(&err);
                        error!(target: LOG_TARGET, "🚨Error handling new transaction: {}", err);
                    }
                },

                Ok(event) = epoch_manager_events.recv() => {
                    self.on_epoch_manager_event(event).await?;
                },

                // TODO: This channel is used to work around some design-flaws in missing transactions handling.
                //       We cannot simply call check_if_block_can_be_unparked in dispatch_hotstuff_message as that creates a cycle.
                //       One suggestion is to refactor consensus to emit events (kinda like libp2p does) and handle those events.
                //       This should be easy to reason about and avoid a large depth of async calls and "callback channels".
                Some(tx_id) = self.rx_missing_transactions.recv() => {
                    if let Err(err) = self.check_if_block_can_be_unparked(current_epoch, current_height, &tx_id, &local_committee_info).await {
                        self.hooks.on_error(&err);
                        error!(target: LOG_TARGET, "🚨Error handling missing transaction: {}", err);
                    }
                },

                _ = on_beat.wait() => {
                    if let Err(e) = self.on_beat(current_epoch, &local_committee_info).await {
                        self.on_failure("on_beat", &e).await;
                        return Err(e);
                    }
                },

                maybe_leaf_block = on_force_beat.wait() => {
                    self.hooks.on_beat();
                    if let Err(e) = self.propose_if_leader(current_epoch, maybe_leaf_block, &local_committee_info).await {
                        self.on_failure("propose_if_leader", &e).await;
                        return Err(e);
                    }
                },

                new_height = on_leader_timeout.wait() => {
                    if let Err(e) = self.on_leader_timeout(current_epoch, new_height).await {
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

    async fn on_unvalidated_message(
        &mut self,
        current_epoch: Epoch,
        current_height: NodeHeight,
        result: Result<(TConsensusSpec::Addr, HotstuffMessage), HotStuffError>,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let (from, msg) = result?;

        match self
            .on_message_validate
            .handle(current_height, local_committee_info, from.clone(), msg)
            .await?
        {
            MessageValidationResult::Ready { from, message: msg } => {
                if let Err(e) = self
                    .dispatch_hotstuff_message(current_epoch, from, msg, local_committee_info)
                    .await
                {
                    self.on_failure("on_unvalidated_message -> dispatch_hotstuff_message", &e)
                        .await;
                    return Err(e);
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
                    let mut local_committee = self.epoch_manager.get_local_committee(epoch).await?;

                    local_committee.shuffle();
                    match local_committee
                        .into_iter()
                        .find(|(addr, _)| *addr != self.local_validator_addr)
                    {
                        Some((addr, _)) => {
                            warn!(target: LOG_TARGET, "⚠️Requesting missing transactions from another validator {addr}
                because we are (presumably) catching up (local_peer_id = {})", self.local_validator_addr);
                            request_from_address = addr;
                        },
                        None => {
                            warn!(
                                target: LOG_TARGET,
                                "❌NEVERHAPPEN: We're the only validator in the committee but we need to request missing
                transactions."             );
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
            MessageValidationResult::Invalid { err, .. } => Err(err),
        }
    }

    async fn on_new_transaction(
        &mut self,
        transaction: Transaction,
        num_pending_txs: usize,
        current_epoch: Epoch,
        current_height: NodeHeight,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let maybe_transaction = self.on_receive_new_transaction.try_sequence_transaction(
            current_epoch,
            transaction,
            local_committee_info,
        )?;

        let Some(transaction) = maybe_transaction else {
            return Ok(());
        };

        debug!(
            target: LOG_TARGET,
            "🔥 new transaction ready for consensus: {} ({} pending)",
            transaction.id(),
            num_pending_txs,
        );

        self.hooks.on_transaction_ready(transaction.id());

        if self
            .check_if_block_can_be_unparked(current_epoch, current_height, transaction.id(), local_committee_info)
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
    async fn check_if_block_can_be_unparked(
        &mut self,
        current_epoch: Epoch,
        current_height: NodeHeight,
        tx_id: &TransactionId,
        local_committee_info: &CommitteeInfo,
    ) -> Result<bool, HotStuffError> {
        let mut is_any_block_unparked = false;
        if let Some(msg) = self
            .on_message_validate
            .update_local_parked_blocks(current_height, tx_id)?
        {
            let vn = self
                .epoch_manager
                .get_validator_node_by_public_key(msg.block.epoch(), msg.block.proposed_by())
                .await?;

            if let Err(e) = self
                .dispatch_hotstuff_message(
                    current_epoch,
                    vn.address,
                    HotstuffMessage::Proposal(msg),
                    local_committee_info,
                )
                .await
            {
                self.on_failure("on_new_transaction -> dispatch_hotstuff_message", &e)
                    .await;
                return Err(e);
            }
            is_any_block_unparked = true;
        }

        let unparked_foreign_blocks = self.on_message_validate.update_foreign_parked_blocks(tx_id)?;
        is_any_block_unparked |= !unparked_foreign_blocks.is_empty();
        for parked in unparked_foreign_blocks {
            let vn = self
                .epoch_manager
                .get_validator_node_by_public_key(parked.block().epoch(), parked.block().proposed_by())
                .await?;

            if let Err(e) = self
                .dispatch_hotstuff_message(
                    current_epoch,
                    vn.address,
                    HotstuffMessage::ForeignProposal(parked.into()),
                    local_committee_info,
                )
                .await
            {
                self.on_failure("on_new_transaction -> dispatch_hotstuff_message", &e)
                    .await;
                return Err(e);
            }
        }

        Ok(is_any_block_unparked)
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
                // let leaf_block = self.state_store.with_read_tx(|tx| LeafBlock::get(tx))?;
                // if leaf_block.block_id.is_zero() {
                //     self.pacemaker.set_epoch(epoch).await?;
                // }

                // If we can propose a block end, let's not wait for the block time to do it
                self.pacemaker.beat();
            },
            EpochManagerEvent::ThisValidatorIsRegistered { .. } => {},
        }

        Ok(())
    }

    async fn request_initial_catch_up_sync(&mut self, current_epoch: Epoch) -> Result<(), HotStuffError> {
        let committee = self.epoch_manager.get_local_committee(current_epoch).await?;
        for member in committee.shuffled() {
            if *member != self.local_validator_addr {
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

    async fn on_leader_timeout(&mut self, current_epoch: Epoch, new_height: NodeHeight) -> Result<(), HotStuffError> {
        self.hooks.on_leader_timeout(new_height);
        self.on_next_sync_view.handle(current_epoch, new_height).await?;
        self.publish_event(HotstuffEvent::LeaderTimeout { new_height });
        Ok(())
    }

    async fn on_beat(&mut self, epoch: Epoch, local_committee_info: &CommitteeInfo) -> Result<(), HotStuffError> {
        self.hooks.on_beat();
        if !self.state_store.with_read_tx(|tx| {
            // Propose quickly if there are UTXOs to mint or transactions to propose
            Ok::<_, HotStuffError>(
                BurntUtxo::has_unproposed(tx)? || self.transaction_pool.has_uncommitted_transactions(tx)?,
            )
        })? {
            let current_epoch = self.epoch_manager.current_epoch().await?;
            // Propose quickly if we should end the epoch (i.e base layer epoch > pacemaker epoch)
            if current_epoch == epoch {
                debug!(target: LOG_TARGET, "[on_beat] No transactions to propose. Waiting for a timeout.");
                return Ok(());
            }
        }

        self.propose_if_leader(epoch, None, local_committee_info).await?;

        Ok(())
    }

    async fn propose_if_leader(
        &mut self,
        epoch: Epoch,
        leaf_block: Option<LeafBlock>,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let is_newview_propose = leaf_block.is_some();
        let leaf_block = match leaf_block {
            Some(leaf_block) => leaf_block,
            None => self.state_store.with_read_tx(|tx| LeafBlock::get(tx))?,
        };

        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;

        let is_leader = self.leader_strategy.is_leader_for_next_block(
            &self.local_validator_addr,
            &local_committee,
            leaf_block.height,
        );
        info!(
            target: LOG_TARGET,
            "🔥 [on_beat{}] {} Is leader: {:?}, leaf_block: {}, local_committee: {}",
            if is_newview_propose { " (NEWVIEW)"} else { "" },
            self.local_validator_addr,
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
                    local_committee_info,
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
        current_epoch: Epoch,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        // TODO: check the message comes from a local committee member (except foreign proposals which must come from a
        //       registered node)
        match msg {
            HotstuffMessage::NewView(message) => log_err(
                "on_receive_new_view",
                self.on_receive_new_view
                    .handle(from, message, local_committee_info)
                    .await,
            ),
            HotstuffMessage::Proposal(msg) => {
                // First process attached foreign proposals
                for foreign_proposal in msg.foreign_proposals {
                    log_err(
                        "on_receive_foreign_proposal",
                        self.on_receive_foreign_proposal
                            .handle(from.clone(), foreign_proposal.into(), local_committee_info)
                            .await,
                    )?;
                }

                match log_err(
                    "on_receive_local_proposal",
                    self.on_receive_local_proposal.handle(current_epoch, msg.block).await,
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
                        self.on_catch_up_sync.request_sync(current_epoch, &from).await?;
                        Ok(())
                    },
                    Err(err) => Err(err),
                }
            },
            HotstuffMessage::ForeignProposal(msg) => log_err(
                "on_receive_foreign_proposal",
                self.on_receive_foreign_proposal
                    .handle(from, msg, local_committee_info)
                    .await,
            ),
            HotstuffMessage::Vote(msg) => log_err(
                "on_receive_vote",
                self.on_receive_vote.handle(from, msg, local_committee_info).await,
            ),
            HotstuffMessage::MissingTransactionsRequest(msg) => log_err(
                "on_receive_request_missing_transactions",
                self.on_receive_request_missing_txs.handle(from, msg).await,
            ),
            HotstuffMessage::MissingTransactionsResponse(msg) => log_err(
                "on_receive_new_transaction",
                self.on_receive_new_transaction
                    .process_requested(current_epoch, from, msg, local_committee_info)
                    .await,
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

    fn create_zero_block_if_required(&self, epoch: Epoch, shard_group: ShardGroup) -> Result<(), HotStuffError> {
        self.state_store.with_write_tx(|tx| {
            // The parent for genesis blocks refer to this zero block
            let mut zero_block = Block::zero_block(self.config.network, self.config.num_preshards);
            if !zero_block.exists(&**tx)? {
                debug!(target: LOG_TARGET, "Creating zero block");
                zero_block.justify().insert(tx)?;
                zero_block.insert(tx)?;
                zero_block.set_as_justified(tx)?;
                zero_block.as_locked_block().set(tx)?;
                zero_block.as_leaf_block().set(tx)?;
                zero_block.as_last_executed().set(tx)?;
                zero_block.as_last_voted().set(tx)?;
                zero_block.justify().as_high_qc().set(tx)?;
                zero_block.commit_diff(tx, BlockDiff::empty(*zero_block.id()))?;
            }

            let mut genesis = Block::genesis(self.config.network, epoch, shard_group);
            if !genesis.exists(&**tx)? {
                info!(target: LOG_TARGET, "✨Creating genesis block {genesis}");
                genesis.justify().insert(tx)?;
                genesis.insert(tx)?;
                genesis.set_as_justified(tx)?;
                genesis.as_locked_block().set(tx)?;
                genesis.as_leaf_block().set(tx)?;
                genesis.as_last_executed().set(tx)?;
                genesis.as_last_voted().set(tx)?;
                genesis.justify().as_high_qc().set(tx)?;
                genesis.commit_diff(tx, BlockDiff::empty(*genesis.id()))?;
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
