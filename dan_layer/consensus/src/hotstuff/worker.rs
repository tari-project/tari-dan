//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, HighQc, LeafBlock, TransactionPool},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::{EpochManagerEvent, EpochManagerReader};
use tari_shutdown::ShutdownSignal;
use tari_transaction::{Transaction, TransactionId};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use super::on_receive_requested_transactions::OnReceiveRequestedTransactions;
use crate::{
    hotstuff::{
        common::CommitteeAndMessage,
        error::HotStuffError,
        event::HotstuffEvent,
        on_next_sync_view::OnNextSyncViewHandler,
        on_propose::OnPropose,
        on_receive_new_view::OnReceiveNewViewHandler,
        on_receive_proposal::OnReceiveProposalHandler,
        on_receive_request_missing_transactions::OnReceiveRequestMissingTransactions,
        on_receive_vote::OnReceiveVoteHandler,
        pacemaker::PaceMaker,
        pacemaker_handle::PaceMakerHandle,
    },
    messages::HotstuffMessage,
    traits::{ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::worker";

pub struct HotstuffWorker<TConsensusSpec: ConsensusSpec> {
    validator_addr: TConsensusSpec::Addr,

    rx_new_transactions: mpsc::Receiver<TransactionId>,
    rx_hs_message: mpsc::Receiver<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    tx_events: broadcast::Sender<HotstuffEvent>,

    // on_leader_timeout: OnLeaderTimeout,
    on_next_sync_view: OnNextSyncViewHandler<TConsensusSpec>,
    on_receive_proposal: OnReceiveProposalHandler<TConsensusSpec>,
    on_receive_vote: OnReceiveVoteHandler<TConsensusSpec>,
    on_receive_new_view: OnReceiveNewViewHandler<TConsensusSpec>,
    on_receive_request_missing_txs: OnReceiveRequestMissingTransactions<TConsensusSpec>,
    on_receive_requested_txs: OnReceiveRequestedTransactions<TConsensusSpec>,
    on_propose: OnPropose<TConsensusSpec>,

    state_store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,

    epoch_manager: TConsensusSpec::EpochManager,
    epoch_events: broadcast::Receiver<EpochManagerEvent>,
    latest_epoch: Option<Epoch>,
    is_epoch_synced: bool,

    pacemaker: Option<PaceMaker>,
    pacemaker_handle: PaceMakerHandle,
    shutdown: ShutdownSignal,
}
impl<TConsensusSpec> HotstuffWorker<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    TConsensusSpec::StateStore: Clone,
    TConsensusSpec::EpochManager: Clone,
    TConsensusSpec::LeaderStrategy: Clone,
    TConsensusSpec::VoteSignatureService: Clone,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        validator_addr: TConsensusSpec::Addr,
        rx_new_transactions: mpsc::Receiver<TransactionId>,
        rx_hs_message: mpsc::Receiver<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
        state_store: TConsensusSpec::StateStore,
        epoch_events: broadcast::Receiver<EpochManagerEvent>,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        signing_service: TConsensusSpec::VoteSignatureService,
        state_manager: TConsensusSpec::StateManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_broadcast: mpsc::Sender<CommitteeAndMessage<TConsensusSpec::Addr>>,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        tx_mempool: mpsc::UnboundedSender<Transaction>,
        shutdown: ShutdownSignal,
    ) -> Self {
        let pacemaker = PaceMaker::new(shutdown.clone());
        Self {
            validator_addr: validator_addr.clone(),
            rx_new_transactions,
            rx_hs_message,
            tx_events: tx_events.clone(),
            on_next_sync_view: OnNextSyncViewHandler::new(
                state_store.clone(),
                tx_leader.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
            ),
            on_receive_proposal: OnReceiveProposalHandler::new(
                validator_addr,
                state_store.clone(),
                epoch_manager.clone(),
                signing_service.clone(),
                leader_strategy.clone(),
                state_manager,
                transaction_pool.clone(),
                tx_leader.clone(),
                tx_events,
                pacemaker.clone_handle(),
            ),
            on_receive_vote: OnReceiveVoteHandler::new(
                state_store.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
                signing_service,
                pacemaker.clone_handle(),
            ),
            on_receive_new_view: OnReceiveNewViewHandler::new(
                state_store.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
                pacemaker.clone_handle(),
            ),
            on_receive_request_missing_txs: OnReceiveRequestMissingTransactions::new(state_store.clone(), tx_leader),
            on_receive_requested_txs: OnReceiveRequestedTransactions::new(tx_mempool),
            on_propose: OnPropose::new(
                state_store.clone(),
                epoch_manager.clone(),
                transaction_pool.clone(),
                tx_broadcast,
            ),
            // on_leader_timeout: OnLeaderTimeout::qnew(shutdown.clone()),
            state_store,
            leader_strategy,
            epoch_manager,
            epoch_events,
            latest_epoch: None,
            is_epoch_synced: false,
            transaction_pool,
            pacemaker_handle: pacemaker.clone_handle(),
            pacemaker: Some(pacemaker),
            shutdown,
        }
    }

    pub fn spawn(self) -> JoinHandle<Result<(), anyhow::Error>> {
        tokio::spawn(async move {
            self.run().await?;
            Ok(())
        })
    }

    pub async fn run(mut self) -> Result<(), HotStuffError> {
        self.create_genesis_block_if_required(Epoch(0))?;
        let (mut on_beat, mut on_force_beat, mut on_leader_timeout) = self.pacemaker.take().map(|p| p.spawn()).unwrap();

        loop {
            tokio::select! {
                Ok(event) = self.epoch_events.recv() => {
                    if let Err(e) = self.on_epoch_event(event).await {
                        error!(target: LOG_TARGET, "Error while processing epoch change (on_epoch_event): {}", e);
                    }
                },
                Some(msg) = self.rx_new_transactions.recv() => {
                    if let Err(e) = self.on_new_executed_transaction(msg).await {
                       error!(target: LOG_TARGET, "Error while processing new payload (on_new_executed_transaction): {}", e);
                    }
                },
                Some((from, msg)) = self.rx_hs_message.recv() => {
                    if let Err(e) = self.on_new_hs_message(from, msg.clone()).await {
                        self.publish_event(HotstuffEvent::Failure { message: e.to_string() });
                        debug!(target: LOG_TARGET, "on_new_hs_message error: {} {:?}", e, msg);
                        error!(target: LOG_TARGET, "Error while processing new hotstuff message (on_new_hs_message): {}", e);
                    }
                },

                _ = on_beat.wait() => {
                    if let Err(e) = self.on_beat().await {
                        error!(target: LOG_TARGET, "Error (on_beat): {}", e);
                    }
                },
                maybe_leaf_block = on_force_beat.wait() => {
                    if let Err(e) = self.propose_if_leader(maybe_leaf_block).await {
                        error!(target: LOG_TARGET, "Error (propose_if_leader): {}", e);
                    }
                }
                new_height = on_leader_timeout.wait() => {
                    if let Err(e) = self.on_leader_timeout(new_height).await {
                        error!(target: LOG_TARGET, "Error (on_leader_timeout): {}", e);
                    }
                },
                // _ = self.on_leader_timeout.o() => {
                //     if let Err(e) = self.on_leader_timeout().await {
                //         error!(target: LOG_TARGET, "Error (on_leader_timeout): {}", e);
                //     }
                // }
                _ = self.shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn on_epoch_event(&mut self, event: EpochManagerEvent) -> Result<(), HotStuffError> {
        match event {
            EpochManagerEvent::EpochChanged(epoch) => {
                self.is_epoch_synced = true;
                // TODO: merge chain(s) from previous epoch?

                if self
                    .epoch_manager
                    .is_local_validator_registered_for_epoch(epoch)
                    .await?
                {
                    let (leaf_block, high_qc) = self
                        .state_store
                        .with_read_tx(|tx| Ok::<_, HotStuffError>((LeafBlock::get(tx)?, HighQc::get(tx)?)))?;
                    info!(
                        target: LOG_TARGET,
                        "⏰ Pacemaker starting for epoch {}, leaf_block: {}, high_qc: {}",
                        epoch,
                        leaf_block,
                        high_qc
                    );
                    self.pacemaker_handle
                        .start(leaf_block.height(), high_qc.block_height())
                        .await?;
                } else {
                    self.pacemaker_handle.stop().await?;
                }
            },
            EpochManagerEvent::ThisValidatorIsRegistered { .. } => {},
        }

        Ok(())
    }

    async fn on_new_executed_transaction(&mut self, transaction_id: TransactionId) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🚀 Consensus READY for new transaction with id: {}",
            transaction_id
        );
        let maybe_block_id = self
            .state_store
            .with_write_tx(|tx| tx.remove_missing_transaction(transaction_id))?;
        if let Some(block_id) = maybe_block_id {
            self.on_receive_proposal.reprocess_block(&block_id).await?;
        } else {
            let height = self
                .state_store
                .with_read_tx(|tx| Block::get_tip(tx))
                .map(|block| block.height())
                .unwrap_or(NodeHeight(0));
            let high_qc = self
                .state_store
                .with_read_tx(|tx| HighQc::get(tx))
                .map(|qc| qc.block_height())
                .unwrap_or(NodeHeight(0));
            self.pacemaker_handle.reset_leader_timeout(height, high_qc).await?;
        }
        self.pacemaker_handle.beat().await?;
        Ok(())
    }

    async fn on_leader_timeout(&mut self, new_height: NodeHeight) -> Result<(), HotStuffError> {
        // TODO: perhaps the leader should not be increasing the timeout
        if !self.is_epoch_synced {
            warn!(target: LOG_TARGET, "Waiting for epoch change before worrying about leader timeout");
            return Ok(());
        }

        let epoch = self.epoch_manager.current_epoch().await?;
        // Is the VN registered?
        if !self.epoch_manager.is_epoch_active(epoch).await? {
            info!(
                target: LOG_TARGET,
                "[on_leader_timeout] Validator is not active within this epoch"
            );
            return Ok(());
        }

        self.on_next_sync_view.handle(epoch, new_height).await?;

        self.publish_event(HotstuffEvent::LeaderTimeout { new_height });

        Ok(())
    }

    async fn on_beat(&mut self) -> Result<(), HotStuffError> {
        // TODO: This is a temporary hack to ensure that the VN has synced the blockchain before proposing
        if !self.is_epoch_synced {
            warn!(target: LOG_TARGET, "Waiting for epoch change before proposing");
            return Ok(());
        }

        // Are there any transactions in the pools?
        if !self
            .state_store
            .with_read_tx(|tx| self.transaction_pool.has_uncommitted_transactions(tx))?
        {
            debug!(target: LOG_TARGET, "[on_beat] No transactions to propose. Waiting for a timeout.");
            return Ok(());
        }

        self.propose_if_leader(None).await?;

        Ok(())
    }

    async fn propose_if_leader(&mut self, leaf_block: Option<LeafBlock>) -> Result<(), HotStuffError> {
        // TODO: This is a temporary hack to ensure that the VN has synced the blockchain before proposing
        if !self.is_epoch_synced {
            warn!(target: LOG_TARGET, "Waiting for epoch change before proposing");
            return Ok(());
        }

        let must_propose = leaf_block.is_some();
        let leaf_block = match leaf_block {
            Some(leaf_block) => leaf_block,
            None => self.state_store.with_read_tx(|tx| LeafBlock::get(tx))?,
        };
        let current_epoch = self.epoch_manager.current_epoch().await?;
        debug!(target: LOG_TARGET, "[on_beat] Epoch: {}", current_epoch);
        let local_committee = self.epoch_manager.get_local_committee(current_epoch).await?;
        // TODO: If there were leader failures, the leaf block would be empty and we need to create empty blocks.
        let is_leader =
            self.leader_strategy
                .is_leader_for_next_block(&self.validator_addr, &local_committee, leaf_block.height);
        info!(
            target: LOG_TARGET,
            "🔥 [on_beat] Is leader: {:?}, leaf_block: {}, local_committee: {}, must_propose: {}",
            is_leader,
            leaf_block,
            local_committee
                .len(),
            must_propose
        );
        if is_leader {
            self.on_propose
                .handle(current_epoch, local_committee, leaf_block)
                .await?;
        }
        Ok(())
    }

    async fn on_new_hs_message(
        &mut self,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        if !self
            .epoch_manager
            .is_local_validator_registered_for_epoch(msg.epoch())
            .await?
        {
            return Err(HotStuffError::EpochNotActive {
                epoch: msg.epoch(),
                details: "Received message for inactive epoch".to_string(),
            });
        }

        match msg {
            HotstuffMessage::NewView(msg) => {
                log_err("on_receive_new_view", self.on_receive_new_view.handle(from, msg).await)
            },
            HotstuffMessage::Proposal(msg) => {
                log_err("on_receive_proposal", self.on_receive_proposal.handle(from, msg).await)
            },
            HotstuffMessage::Vote(msg) => log_err("on_receive_vote", self.on_receive_vote.handle(from, msg).await),
            HotstuffMessage::RequestMissingTransactions(msg) => log_err(
                "on_receive_request_missing_transactions",
                self.on_receive_request_missing_txs.handle(from, msg).await,
            ),
            HotstuffMessage::RequestedTransaction(msg) => log_err(
                "on_receive_requested_txs",
                self.on_receive_requested_txs.handle(from, msg).await,
            ),
        }
        Ok(())
    }

    fn create_genesis_block_if_required(&mut self, epoch: Epoch) -> Result<(), HotStuffError> {
        // If we've already created the genesis block for this epoch then we can return early
        if self.latest_epoch.map(|e| e >= epoch).unwrap_or(false) {
            return Ok(());
        }

        let mut tx = self.state_store.create_write_tx()?;

        // The parent for all genesis blocks refer to this zero block
        let zero_block = Block::zero_block();
        if !zero_block.exists(tx.deref_mut())? {
            debug!(target: LOG_TARGET, "Creating zero block");
            zero_block.justify().insert(&mut tx)?;
            zero_block.insert(&mut tx)?;
            zero_block.as_locked().set(&mut tx)?;
            zero_block.as_leaf_block().set(&mut tx)?;
            zero_block.as_last_executed().set(&mut tx)?;
            zero_block.justify().as_high_qc().set(&mut tx)?;
            zero_block.commit(&mut tx)?;
        }

        // let genesis = Block::genesis();
        // if !genesis.exists(tx.deref_mut())? {
        //     debug!(target: LOG_TARGET, "Creating genesis block");
        //     genesis.justify().save(&mut tx)?;
        //     genesis.insert(&mut tx)?;
        //     genesis.as_locked().set(&mut tx)?;
        //     genesis.as_leaf_block().set(&mut tx)?;
        //     genesis.as_last_executed().set(&mut tx)?;
        //     genesis.justify().as_high_qc().set(&mut tx)?;
        // }

        tx.commit()?;

        info!(
            target: LOG_TARGET,
            "🚀 Epoch changed to {}",
            epoch
        );

        self.latest_epoch = Some(epoch);

        Ok(())
    }

    fn publish_event(&self, event: HotstuffEvent) {
        let _ignore = self.tx_events.send(event);
    }
}

fn log_err(context: &'static str, result: Result<(), HotStuffError>) {
    if let Err(e) = result {
        error!(target: LOG_TARGET, "Error while processing new hotstuff message ({context}): {e}");
    }
}
