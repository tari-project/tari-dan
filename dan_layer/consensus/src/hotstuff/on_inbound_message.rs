//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    ops::DerefMut,
};

use log::*;
use tari_dan_common_types::{NodeAddressable, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, TransactionRecord},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_shutdown::ShutdownSignal;
use tari_transaction::TransactionId;
use tokio::{sync::mpsc, time};

use crate::{
    block_validations::{check_hash_and_height, check_proposed_by_leader, check_quorum_certificate},
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle},
    messages::{HotstuffMessage, ProposalMessage, RequestMissingTransactionsMessage},
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::inbound_messages";

pub type IncomingMessageResult<TAddr> = Result<Option<(TAddr, HotstuffMessage<TAddr>)>, NeedsSync<TAddr>>;

pub struct OnInboundMessage<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    pacemaker: PaceMakerHandle,
    rx_hotstuff_message: mpsc::Receiver<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    tx_outbound_message: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    tx_msg_ready: mpsc::UnboundedSender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    rx_new_transactions: mpsc::Receiver<TransactionId>,
    message_buffer: MessageBuffer<TConsensusSpec::Addr>,
    shutdown: ShutdownSignal,
}

impl<TConsensusSpec> OnInboundMessage<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        pacemaker: PaceMakerHandle,
        rx_hotstuff_message: mpsc::Receiver<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
        tx_outbound_message: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
        rx_new_transactions: mpsc::Receiver<TransactionId>,
        shutdown: ShutdownSignal,
    ) -> Self {
        let (tx_msg_ready, rx_msg_ready) = mpsc::unbounded_channel();
        Self {
            store,
            epoch_manager,
            leader_strategy,
            pacemaker,
            rx_hotstuff_message,
            tx_outbound_message,
            tx_msg_ready,
            rx_new_transactions,
            message_buffer: MessageBuffer::new(rx_msg_ready),
            shutdown,
        }
    }

    pub async fn next(&mut self, current_height: NodeHeight) -> IncomingMessageResult<TConsensusSpec::Addr> {
        loop {
            tokio::select! {
                biased;

                _ = self.shutdown.wait() => { break Ok(None); }

                msg_or_sync = self.message_buffer.next(current_height) => {
                    break msg_or_sync;
                },

                Some((from, msg)) = self.rx_hotstuff_message.recv() => {
                    if let Err(err) = self.handle_hotstuff_message(current_height, from, msg).await {
                        error!(target: LOG_TARGET, "Error handling message: {}", err);
                    }
                },

                 Some(tx_id) = self.rx_new_transactions.recv() => {
                    if let Err(err) = self.check_if_parked_blocks_ready(current_height, &tx_id).await {
                        error!(target: LOG_TARGET, "Error checking parked blocks: {}", err);
                    }
                },
            }
        }
    }

    pub async fn discard(&mut self) {
        loop {
            tokio::select! {
                biased;
                _ = self.shutdown.wait() => { break; }
                _ = self.message_buffer.discard() => { }
                _ = self.rx_hotstuff_message.recv() => { },
                _ = self.rx_new_transactions.recv() => { },
            }
        }
    }

    pub fn clear_buffer(&mut self) {
        self.message_buffer.clear_buffer();
    }

    pub async fn handle_hotstuff_message(
        &self,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        match msg {
            HotstuffMessage::Proposal(msg) => {
                self.process_proposal(current_height, msg).await?;
            },
            msg => self
                .tx_msg_ready
                .send((from, msg))
                .map_err(|_| HotStuffError::InternalChannelClosed {
                    context: "tx_msg_ready in InboundMessageWorker::handle_hotstuff_message",
                })?,
        }
        Ok(())
    }

    async fn process_proposal(
        &self,
        current_height: NodeHeight,
        proposal: ProposalMessage<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = proposal;

        info!(
            target: LOG_TARGET,
            "📜 new unvalidated PROPOSAL message {} from {} (current height = {})",
            block,
            block.proposed_by(),
            current_height,
        );

        if block.height() < current_height {
            debug!(
                target: LOG_TARGET,
                "🔥 Block {} is lower than current height {}. Ignoring.",
                block,
                current_height
            );
            return Ok(());
        }

        check_hash_and_height(&block)?;
        let committee_for_block = self
            .epoch_manager
            .get_committee_by_validator_address(block.epoch(), block.proposed_by())
            .await?;
        check_proposed_by_leader(&self.leader_strategy, &committee_for_block, &block)?;
        check_quorum_certificate(&committee_for_block, &block)?;

        let Some(ready_block) = self.handle_missing_transactions(block).await? else {
            // Block not ready
            return Ok(());
        };

        self.send_ready_block(ready_block)?;

        Ok(())
    }

    async fn check_if_parked_blocks_ready(
        &self,
        current_height: NodeHeight,
        transaction_id: &TransactionId,
    ) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🚀 Consensus (height={}) READY for new transaction with id: {}",current_height,
            transaction_id
        );
        let maybe_unparked_block = self
            .store
            .with_write_tx(|tx| tx.missing_transactions_remove(current_height, transaction_id))?;

        if let Some(unparked_block) = maybe_unparked_block {
            info!(target: LOG_TARGET, "♻️ all transactions for block {unparked_block} have been executed");
            self.send_ready_block(unparked_block)?;
        }
        self.pacemaker.beat();
        Ok(())
    }

    async fn handle_missing_transactions(
        &self,
        block: Block<TConsensusSpec::Addr>,
    ) -> Result<Option<Block<TConsensusSpec::Addr>>, HotStuffError> {
        let (missing_tx_ids, awaiting_execution) = self
            .store
            .with_write_tx(|tx| self.check_for_missing_transactions(tx, &block))?;

        if !missing_tx_ids.is_empty() || !awaiting_execution.is_empty() {
            info!(
                target: LOG_TARGET,
                "🔥 Block {} has {} missing transactions and {} awaiting execution", block, missing_tx_ids.len(), awaiting_execution.len(),
            );

            let block_id = *block.id();
            let epoch = block.epoch();
            let block_proposed_by = block.proposed_by().clone();

            if !missing_tx_ids.is_empty() {
                self.send_message(
                    &block_proposed_by,
                    HotstuffMessage::RequestMissingTransactions(RequestMissingTransactionsMessage {
                        block_id,
                        epoch,
                        transactions: missing_tx_ids,
                    }),
                )
                .await;
            }

            return Ok(None);
        }

        Ok(Some(block))
    }

    fn check_for_missing_transactions(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block<TConsensusSpec::Addr>,
    ) -> Result<(HashSet<TransactionId>, HashSet<TransactionId>), HotStuffError> {
        if block.commands().is_empty() {
            return Ok((HashSet::new(), HashSet::new()));
        }
        let (transactions, missing_tx_ids) = TransactionRecord::get_any(tx.deref_mut(), block.all_transaction_ids())?;
        let awaiting_execution = transactions
            .into_iter()
            .filter(|tx| tx.result.is_none())
            .map(|tx| *tx.transaction.id())
            .collect::<HashSet<_>>();

        if missing_tx_ids.is_empty() && awaiting_execution.is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} has no missing transactions", block
            );
            return Ok((HashSet::new(), HashSet::new()));
        }

        info!(
            target: LOG_TARGET,
            "🔥 Block {} has {} missing transactions and {} awaiting execution", block, missing_tx_ids.len(), awaiting_execution.len(),
        );

        tx.missing_transactions_insert(block, &missing_tx_ids, &awaiting_execution)?;

        Ok((missing_tx_ids, awaiting_execution))
    }

    async fn send_message(&self, to: &TConsensusSpec::Addr, message: HotstuffMessage<TConsensusSpec::Addr>) {
        if self.tx_outbound_message.send((to.clone(), message)).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "tx_leader in InboundMessageWorker::send_message is closed",
            );
        }
    }

    fn send_ready_block(&self, block: Block<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        self.tx_msg_ready
            .send((
                block.proposed_by().clone(),
                HotstuffMessage::Proposal(ProposalMessage { block }),
            ))
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_msg_ready in InboundMessageWorker::process_proposal",
            })
    }
}

struct MessageBuffer<TAddr> {
    buffer: BTreeMap<NodeHeight, VecDeque<(TAddr, HotstuffMessage<TAddr>)>>,
    rx_msg_ready: mpsc::UnboundedReceiver<(TAddr, HotstuffMessage<TAddr>)>,
}

impl<TAddr: NodeAddressable> MessageBuffer<TAddr> {
    pub fn new(rx_msg_ready: mpsc::UnboundedReceiver<(TAddr, HotstuffMessage<TAddr>)>) -> Self {
        Self {
            buffer: BTreeMap::new(),
            rx_msg_ready,
        }
    }

    pub async fn next(&mut self, current_height: NodeHeight) -> IncomingMessageResult<TAddr> {
        // Clear buffer with lower heights
        self.buffer = self.buffer.split_off(&current_height);

        // Check if message is in the buffer
        if let Some(buffer) = self.buffer.get_mut(&current_height) {
            if let Some(msg_tuple) = buffer.pop_front() {
                return Ok(Some(msg_tuple));
            }
        }

        while let Some((from, msg)) = self.next_message_or_sync(current_height).await? {
            match msg_height(&msg) {
                // Discard old message
                Some(h) if h < current_height => {
                    debug!(target: LOG_TARGET, "Discard message {} is for previous height {}. Current height {}", msg, h, current_height);
                    continue;
                },
                // Buffer message for future height
                Some(h) if h > current_height => {
                    debug!(target: LOG_TARGET, "Message {} is for future block {}. Current height {}", msg, h, current_height);
                    self.push_to_buffer(h, from, msg);
                    continue;
                },
                // Height is irrelevant or current, return message
                _ => return Ok(Some((from, msg))),
            }
        }

        Ok(None)
    }

    pub async fn discard(&mut self) {
        self.clear_buffer();
        while self.rx_msg_ready.recv().await.is_some() {}
    }

    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
    }

    async fn next_message_or_sync(
        &mut self,
        current_height: NodeHeight,
    ) -> Result<Option<(TAddr, HotstuffMessage<TAddr>)>, NeedsSync<TAddr>> {
        loop {
            // Don't really like this but because we can receive proposals out of order, we need to wait a bit to see
            // if we get a proposal at our height without switching to sync.
            let timeout = time::sleep(time::Duration::from_secs(2));
            tokio::pin!(timeout);
            tokio::select! {
                msg = self.rx_msg_ready.recv() => return Ok(msg),
                _ = timeout.as_mut() => {
                    // Check if we have any proposals
                    for queue in self.buffer.values() {
                        for (from, msg) in queue {
                           if let Some(proposal) = msg.proposal() {
                                if proposal.block.justify().block_height() > current_height {
                                     return Err(NeedsSync {
                                        from: from.clone(),
                                        local_height: current_height,
                                        qc_height: proposal.block.justify().block_height(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn push_to_buffer(&mut self, height: NodeHeight, from: TAddr, msg: HotstuffMessage<TAddr>) {
        self.buffer.entry(height).or_default().push_back((from, msg));
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Needs sync: local height {local_height} is less than remote QC height {qc_height} from {from}")]
pub struct NeedsSync<TAddr: NodeAddressable> {
    pub from: TAddr,
    pub local_height: NodeHeight,
    pub qc_height: NodeHeight,
}

fn msg_height<TAddr>(msg: &HotstuffMessage<TAddr>) -> Option<NodeHeight> {
    match msg {
        HotstuffMessage::Proposal(msg) => Some(msg.block.height()),
        // Votes for block 2, occur at current height 3
        HotstuffMessage::Vote(msg) => Some(msg.block_height.saturating_add(NodeHeight(1))),
        _ => None,
    }
}
