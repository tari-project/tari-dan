//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::{committee::Committee, Epoch};
use tari_dan_storage::{
    consensus_models::{Block, ExecutedTransaction, LeafBlock, TransactionAtom, TransactionPool},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::{
    hotstuff::{
        error::HotStuffError,
        event::HotstuffEvent,
        on_beat::OnBeat,
        on_propose::OnPropose,
        on_receive_new_view::OnReceiveNewViewHandler,
        on_receive_proposal::OnReceiveProposalHandler,
        on_receive_vote::OnReceiveVoteHandler,
    },
    messages::HotstuffMessage,
    traits::{ConsensusSpec, EpochManager, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::worker";

pub struct HotstuffWorker<TConsensusSpec: ConsensusSpec> {
    validator_addr: TConsensusSpec::Addr,

    rx_new_transactions: mpsc::Receiver<ExecutedTransaction>,
    rx_hs_message: mpsc::Receiver<(TConsensusSpec::Addr, HotstuffMessage)>,

    on_receive_proposal: OnReceiveProposalHandler<TConsensusSpec>,
    on_receive_vote: OnReceiveVoteHandler<TConsensusSpec>,
    on_receive_new_view: OnReceiveNewViewHandler<TConsensusSpec>,
    on_propose: OnPropose<TConsensusSpec>,

    state_store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,

    on_beat: OnBeat,
    shutdown: ShutdownSignal,
}
impl<TConsensusSpec> HotstuffWorker<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    TConsensusSpec::StateStore: Clone,
    TConsensusSpec::EpochManager: Clone,
    TConsensusSpec::LeaderStrategy: Clone,
    TConsensusSpec::VoteSignatureService: Clone,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        validator_addr: TConsensusSpec::Addr,
        rx_new_transactions: mpsc::Receiver<ExecutedTransaction>,
        rx_hs_message: mpsc::Receiver<(TConsensusSpec::Addr, HotstuffMessage)>,
        state_store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        signing_service: TConsensusSpec::VoteSignatureService,
        state_manager: TConsensusSpec::StateManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_broadcast: mpsc::Sender<(Committee<TConsensusSpec::Addr>, HotstuffMessage)>,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        shutdown: ShutdownSignal,
    ) -> Self {
        let on_beat = OnBeat::new();
        Self {
            validator_addr: validator_addr.clone(),
            rx_new_transactions,
            rx_hs_message,
            on_receive_proposal: OnReceiveProposalHandler::new(
                validator_addr,
                state_store.clone(),
                epoch_manager.clone(),
                signing_service.clone(),
                leader_strategy.clone(),
                state_manager,
                transaction_pool.clone(),
                tx_leader,
                tx_events,
                on_beat.clone(),
            ),
            on_receive_vote: OnReceiveVoteHandler::new(
                state_store.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
                signing_service,
                on_beat.clone(),
            ),
            on_receive_new_view: OnReceiveNewViewHandler::new(
                state_store.clone(),
                leader_strategy.clone(),
                epoch_manager.clone(),
                on_beat.clone(),
            ),
            on_propose: OnPropose::new(
                state_store.clone(),
                epoch_manager.clone(),
                transaction_pool.clone(),
                tx_broadcast,
            ),
            state_store,
            leader_strategy,
            epoch_manager,
            transaction_pool,
            on_beat,
            shutdown,
        }
    }

    pub fn spawn(self) -> JoinHandle<Result<(), HotStuffError>> {
        tokio::spawn(self.run())
    }

    pub async fn run(mut self) -> Result<(), HotStuffError> {
        // TODO: this should happen for every epoch change / need to merge chain(s) from previous epoch
        let current_epoch = self.epoch_manager.current_epoch().await?;
        self.create_genesis_block_if_required(current_epoch)?;

        self.on_beat.beat();

        loop {
            tokio::select! {
                biased;

                Some(msg) = self.rx_new_transactions.recv() => {
                    if let Err(e) = self.on_new_executed_transaction(msg).await {
                       error!(target: LOG_TARGET, "Error while processing new payload (on_new_executed_transaction): {}", e);
                    }
                },
                Some((from, msg)) = self.rx_hs_message.recv() => {
                    if let Err(e) = self.on_new_hs_message(from, msg).await {
                        // self.publish_event(HotStuffEvent::Failed(e.to_string()));
                        error!(target: LOG_TARGET, "Error while processing new hotstuff message (on_new_hs_message): {}", e);
                    }
                },

                _ = self.on_beat.wait() => {
                    if let Err(e) = self.on_beat().await {
                        error!(target: LOG_TARGET, "Error (on_beat): {}", e);
                    }
                }

                _ = self.shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn on_new_executed_transaction(&mut self, executed: ExecutedTransaction) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "Received new transaction with id: {}",
            executed.transaction().hash()
        );
        self.state_store.with_write_tx(|tx| {
            executed.insert(tx)?;

            self.transaction_pool.insert(tx, TransactionAtom {
                id: *executed.transaction().hash(),
                involved_shards: executed.transaction().involved_shards_iter().copied().collect(),
                decision: executed.as_decision(),
                evidence: executed.to_initial_evidence(),
                fee: executed
                    .result()
                    .fee_receipt
                    .as_ref()
                    .and_then(|f| f.total_fee_payment.as_u64_checked())
                    .unwrap_or(0),
            })?;

            Ok::<_, HotStuffError>(())
        })?;
        self.on_beat.beat();
        Ok(())
    }

    async fn on_beat(&mut self) -> Result<(), HotStuffError> {
        let epoch = self.epoch_manager.current_epoch().await?;

        // Is the VN registered?
        if !self.epoch_manager.is_epoch_active(epoch).await? {
            info!(
                target: LOG_TARGET,
                "[on_beat] Validator is not active within this epoch"
            );
            return Ok(());
        }

        // Are there any transactions in the pools? The block may still be empty if non are ready but we still need to
        // propose a block to get to a 3-chain.
        if !self
            .state_store
            .with_read_tx(|tx| self.transaction_pool.has_transactions(tx))?
        {
            debug!(target: LOG_TARGET, "[on_beat] No transactions to propose");
            return Ok(());
        }

        // Are we the leader?
        let leaf_block = self.state_store.with_read_tx(|tx| LeafBlock::get(tx, epoch))?;
        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;
        let is_leader = self
            .leader_strategy
            .is_leader(&self.validator_addr, &local_committee, &leaf_block.block_id, 0);
        if is_leader {
            self.on_propose.handle(epoch, local_committee, leaf_block).await?;
        }

        Ok(())
    }

    async fn on_new_hs_message(
        &mut self,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage,
    ) -> Result<(), HotStuffError> {
        if !self.epoch_manager.is_epoch_active(msg.epoch()).await? {
            return Err(HotStuffError::EpochNotActive {
                epoch: msg.epoch(),
                details: "Received message for inactive epoch".to_string(),
            });
        }

        match msg {
            HotstuffMessage::NewView(msg) => self.on_receive_new_view.handle(from, msg).await?,
            HotstuffMessage::Proposal(msg) => self.on_receive_proposal.handle(from, msg).await?,
            HotstuffMessage::Vote(msg) => self.on_receive_vote.handle(from, msg).await?,
        }
        Ok(())
    }

    fn create_genesis_block_if_required(&self, epoch: Epoch) -> Result<(), HotStuffError> {
        let mut tx = self.state_store.create_write_tx()?;

        // The parent for all genesis blocks refer to this zero block
        let zero_block = Block::zero_block();
        if !zero_block.exists(tx.deref_mut())? {
            zero_block.justify().insert(&mut tx)?;
            zero_block.insert(&mut tx)?;
        }

        let genesis = Block::genesis(epoch);
        if !genesis.exists(tx.deref_mut())? {
            genesis.insert(&mut tx)?;
            genesis.set_as_locked(&mut tx)?;
            genesis.set_as_leaf(&mut tx)?;
            genesis.set_as_last_executed(&mut tx)?;
            genesis.justify().set_as_high_qc(&mut tx)?;
        }

        tx.commit()?;

        Ok(())
    }
}
