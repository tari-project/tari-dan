//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp,
    fmt::{Debug, Formatter},
    ops::DerefMut,
};

use log::*;
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::{
    consensus_models::{Block, HighQc, LastVoted, LeafBlock, TransactionPool, ValidBlock},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_shutdown::ShutdownSignal;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{broadcast, mpsc};

use super::on_receive_requested_transactions::OnReceiveRequestedTransactions;
use crate::{
    hotstuff::{
        common::CommitteeAndMessage,
        error::HotStuffError,
        event::HotstuffEvent,
        inbound_messages::{InboundHotstuffMessages, ProposalOrVote},
        on_local_block_ready::OnLocalBlockReady,
        on_new_valid_local_block::OnNewValidLocalBlock,
        on_next_sync_view::OnNextSyncViewHandler,
        on_propose::OnPropose,
        on_receive_foreign_proposal::OnReceiveForeignProposalHandler,
        on_receive_local_proposal::OnReceiveProposalHandler,
        on_receive_new_view::OnReceiveNewViewHandler,
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
    rx_block_ready: mpsc::Receiver<ValidBlock<TConsensusSpec::Addr>>,
    tx_events: broadcast::Sender<HotstuffEvent>,

    on_next_sync_view: OnNextSyncViewHandler<TConsensusSpec>,
    on_receive_local_proposal: OnReceiveProposalHandler<TConsensusSpec>,
    on_receive_foreign_proposal: OnReceiveForeignProposalHandler<TConsensusSpec>,
    on_receive_vote: OnReceiveVoteHandler<TConsensusSpec>,
    on_receive_new_view: OnReceiveNewViewHandler<TConsensusSpec>,
    on_receive_request_missing_txs: OnReceiveRequestMissingTransactions<TConsensusSpec>,
    on_receive_requested_txs: OnReceiveRequestedTransactions<TConsensusSpec>,
    on_propose: OnPropose<TConsensusSpec>,
    on_local_block_ready: OnLocalBlockReady<TConsensusSpec>,

    state_store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    inbound_messages: InboundHotstuffMessages<TConsensusSpec::Addr>,

    epoch_manager: TConsensusSpec::EpochManager,
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
        let pacemaker = PaceMaker::new();
        let (tx_block_ready, rx_block_ready) = mpsc::channel(1);
        let on_new_valid_local_block = OnNewValidLocalBlock::new(
            state_store.clone(),
            pacemaker.clone_handle(),
            tx_leader.clone(),
            tx_block_ready,
        );
        Self {
            validator_addr: validator_addr.clone(),
            rx_new_transactions,
            rx_hs_message,
            rx_block_ready,
            tx_events: tx_events.clone(),

            on_next_sync_view: OnNextSyncViewHandler::new(
                state_store.clone(),
                tx_leader.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
            ),
            on_receive_local_proposal: OnReceiveProposalHandler::new(
                state_store.clone(),
                epoch_manager.clone(),
                leader_strategy.clone(),
                on_new_valid_local_block,
            ),
            on_receive_foreign_proposal: OnReceiveForeignProposalHandler::new(
                state_store.clone(),
                epoch_manager.clone(),
                transaction_pool.clone(),
                pacemaker.clone_handle(),
            ),
            on_receive_vote: OnReceiveVoteHandler::new(
                state_store.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
                signing_service.clone(),
                pacemaker.clone_handle(),
            ),
            on_receive_new_view: OnReceiveNewViewHandler::new(
                state_store.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
                pacemaker.clone_handle(),
            ),
            on_receive_request_missing_txs: OnReceiveRequestMissingTransactions::new(
                state_store.clone(),
                tx_leader.clone(),
            ),
            on_receive_requested_txs: OnReceiveRequestedTransactions::new(tx_mempool),
            on_propose: OnPropose::new(
                state_store.clone(),
                epoch_manager.clone(),
                transaction_pool.clone(),
                tx_broadcast,
            ),
            on_local_block_ready: OnLocalBlockReady::new(
                validator_addr,
                state_store.clone(),
                epoch_manager.clone(),
                signing_service,
                leader_strategy.clone(),
                state_manager,
                transaction_pool.clone(),
                tx_leader,
                tx_events,
                pacemaker.clone_handle(),
            ),

            state_store,
            leader_strategy,
            epoch_manager,
            transaction_pool,
            inbound_messages: InboundHotstuffMessages::new(),

            pacemaker_handle: pacemaker.clone_handle(),
            pacemaker: Some(pacemaker),
            shutdown,
        }
    }
}
impl<TConsensusSpec> HotstuffWorker<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub async fn start(&mut self) -> Result<(), HotStuffError> {
        self.create_genesis_block_if_required()?;
        let (current_height, high_qc) = self.state_store.with_read_tx(|tx| {
            let leaf = LeafBlock::get(tx)?;
            let last_voted = LastVoted::get(tx)?;
            Ok::<_, HotStuffError>((cmp::max(leaf.height(), last_voted.height()), HighQc::get(tx)?))
        })?;
        info!(
            target: LOG_TARGET,
            "⏰ Pacemaker starting leaf_block: {}, high_qc: {}",
            current_height,
            high_qc
        );

        self.pacemaker_handle
            .start(current_height, high_qc.block_height())
            .await?;

        self.run().await?;
        Ok(())
    }

    async fn run(&mut self) -> Result<(), HotStuffError> {
        // Spawn pacemaker if not spawned already
        if let Some(pm) = self.pacemaker.take() {
            pm.spawn();
        }

        let mut on_beat = self.pacemaker_handle.get_on_beat();
        let mut on_force_beat = self.pacemaker_handle.get_on_force_beat();
        let mut on_leader_timeout = self.pacemaker_handle.get_on_leader_timeout();

        loop {
            // TODO: Cache the height for this loop
            // let current_height = self.state_store.with_read_tx(|tx| {
            //     let last_voted = LastVoted::get(tx)?;
            //     let leaf = LeafBlock::get(tx)?;
            //     Ok::<_, HotStuffError>(cmp::max(last_voted.height(), leaf.height()) + NodeHeight(1))
            // })?;
            let current_height = self.pacemaker_handle.current_height() + NodeHeight(1);

            debug!(
                target: LOG_TARGET,
                "🔥 Current height #{}",
                current_height.as_u64()
            );

            tokio::select! {
                Some((from, msg)) = self.rx_hs_message.recv() => {
                    if let Err(e) = self.on_new_hs_message(from, msg.clone()).await {
                        self.on_failure("on_new_hs_message", &e).await;
                       return Err(e);
                    }
                },

                result = self.inbound_messages.next(current_height) => {
                    let msg = result?;
                    if let Err(err) = self.handle_inbound_consensus_message(msg).await {
                        self.on_failure("handle_inbound_consensus_message", &err).await;
                        return Err(err);
                    }
                },

                Some(msg) = self.rx_block_ready.recv() => {
                    if let Err(err) = self.on_local_block_ready.handle(msg).await {
                        self.on_failure("on_local_block_ready", &err).await;
                        return Err(err);
                    }
                },

                Some(msg) = self.rx_new_transactions.recv() => {
                    if let Err(e) = self.on_new_executed_transaction(current_height, msg).await {
                       error!(target: LOG_TARGET, "Error while processing new payload (on_new_executed_transaction): {}", e);
                    }
                },

                _ = on_beat.wait() => {
                    if let Err(e) = self.on_beat().await {
                        self.on_failure("on_beat", &e).await;
                        return Err(e);
                    }
                },
                maybe_leaf_block = on_force_beat.wait() => {
                    if let Err(e) = self.propose_if_leader(maybe_leaf_block).await {
                        self.on_failure("propose_if_leader", &e).await;
                        return Err(e);
                    }
                }
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
        self.inbound_messages.clear_buffer();
        // This only happens if we're shutting down.
        if let Err(err) = self.pacemaker_handle.stop().await {
            debug!(target: LOG_TARGET, "Pacemaker channel dropped: {}", err);
        }

        Ok(())
    }

    async fn on_failure(&mut self, context: &str, err: &HotStuffError) {
        self.publish_event(HotstuffEvent::Failure {
            message: err.to_string(),
        });
        error!(target: LOG_TARGET, "Error ({}): {}", context, err);
        if let Err(e) = self.pacemaker_handle.stop().await {
            error!(target: LOG_TARGET, "Error while stopping pacemaker: {}", e);
        }
        self.on_receive_new_view.clear_new_views();
        self.inbound_messages.clear_buffer();
    }

    /// Read and discard messages. This should be used only when consensus is inactive.
    pub async fn discard_messages(&mut self) {
        loop {
            tokio::select! {
                _ = self.rx_hs_message.recv() => { },
                _ = self.rx_new_transactions.recv() => { },

                _ = self.shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }
    }

    pub async fn handle_inbound_consensus_message(
        &mut self,
        msg: ProposalOrVote<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        match msg {
            ProposalOrVote::Proposal(msg) => log_err(
                "on_receive_local_proposal",
                self.on_receive_local_proposal.handle(msg).await,
            ),
            ProposalOrVote::Vote(msg) => log_err("on_receive_vote", self.on_receive_vote.handle(msg).await),
        }
    }

    async fn on_new_executed_transaction(
        &mut self,
        current_height: NodeHeight,
        transaction_id: TransactionId,
    ) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🚀 Consensus (height={}) READY for new transaction with id: {}",current_height,
            transaction_id
        );
        let maybe_block = self
            .state_store
            .with_write_tx(|tx| tx.missing_transactions_remove(current_height, transaction_id))?;
        if let Some(block) = maybe_block {
            debug!(
                target: LOG_TARGET,
                "♻️ Consensus READY for new block with id: {}",
                block.id()
            );
            self.on_receive_local_proposal.reprocess_block(block).await?;
        }
        self.pacemaker_handle.beat();
        Ok(())
    }

    async fn on_leader_timeout(&mut self, new_height: NodeHeight) -> Result<(), HotStuffError> {
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
        let is_newview_propose = leaf_block.is_some();
        let leaf_block = match leaf_block {
            Some(leaf_block) => leaf_block,
            None => self.state_store.with_read_tx(|tx| LeafBlock::get(tx))?,
        };
        let current_epoch = self.epoch_manager.current_epoch().await?;
        let local_committee = self.epoch_manager.get_local_committee(current_epoch).await?;

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
            self.on_propose
                .handle(current_epoch, local_committee, leaf_block, is_newview_propose)
                .await?;
        } else if is_newview_propose {
            // We can make this a warm/error in future, but for now I want to be sure this never happens
            panic!("propose_if_leader called with is_newview_propose=true but we're not the leader");
        } else {
            // Nothing to do
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
            warn!(
                target: LOG_TARGET,
                "Received message for inactive epoch: {}", msg.epoch()
            );
            return Ok(());
        }

        // TODO: check the message comes from a local committee member (except foreign proposals which must come from a
        //       registered node)

        match msg {
            HotstuffMessage::NewView(message) => {
                self.on_receive_new_view.handle(from, message).await?;
                Ok(())
            },
            HotstuffMessage::Proposal(msg) => {
                // TODO: Validate QC
                // if msg.block.justify().is_valid(committee, &self.signing_service) {
                //     warn!(target: LOG_TARGET, "❌ Discarding message: Invalid proposal signature");
                //     return Ok(());
                // }
                self.inbound_messages.enqueue(ProposalOrVote::Proposal(msg));
                Ok(())
            },
            HotstuffMessage::ForeignProposal(msg) => log_err(
                "on_receive_foreign_proposal",
                self.on_receive_foreign_proposal.handle(from, msg).await,
            ),
            HotstuffMessage::Vote(msg) => {
                if msg.signature.public_key != from {
                    warn!(target: LOG_TARGET, "❌ Discarding message: Received vote from another node {} for a different node {}", from, msg.signature.public_key);
                    return Ok(());
                }
                self.inbound_messages.enqueue(ProposalOrVote::Vote(msg));
                Ok(())
            },
            HotstuffMessage::RequestMissingTransactions(msg) => log_err(
                "on_receive_request_missing_transactions",
                self.on_receive_request_missing_txs.handle(from, msg).await,
            ),
            HotstuffMessage::RequestedTransaction(msg) => log_err(
                "on_receive_requested_txs",
                self.on_receive_requested_txs.handle(from, msg).await,
            ),
        }
    }

    fn create_genesis_block_if_required(&self) -> Result<(), HotStuffError> {
        let mut tx = self.state_store.create_write_tx()?;

        // The parent for genesis blocks refer to this zero block
        let zero_block = Block::zero_block();
        if !zero_block.exists(tx.deref_mut())? {
            debug!(target: LOG_TARGET, "Creating zero block");
            zero_block.justify().insert(&mut tx)?;
            zero_block.insert(&mut tx)?;
            zero_block.as_locked_block().set(&mut tx)?;
            zero_block.as_leaf_block().set(&mut tx)?;
            zero_block.as_last_executed().set(&mut tx)?;
            zero_block.as_last_voted().set(&mut tx)?;
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

        Ok(())
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
            .field("pacemaker_handle", &self.pacemaker_handle)
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
