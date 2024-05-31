//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeMap, HashSet, VecDeque};

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::{optional::Optional, NodeAddressable, NodeHeight};
use tari_dan_storage::{
    consensus_models::{
        Block,
        ExecutedTransaction,
        TransactionAtom,
        TransactionPool,
        TransactionPoolRecord,
        TransactionRecord,
    },
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::TransactionId;
use tokio::{sync::mpsc, time};

use super::config::HotstuffConfig;
use crate::{
    validations::block_validations::check_block,
    hotstuff::error::HotStuffError,
    messages::{HotstuffMessage, ProposalMessage, RequestMissingTransactionsMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::inbound_messages";

pub type IncomingMessageResult<TAddr> = Result<Option<(TAddr, HotstuffMessage)>, NeedsSync<TAddr>>;

pub struct OnInboundMessage<TConsensusSpec: ConsensusSpec> {
    network: Network,
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    vote_signing_service: TConsensusSpec::SignatureService,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    tx_msg_ready: mpsc::UnboundedSender<(TConsensusSpec::Addr, HotstuffMessage)>,
    message_buffer: MessageBuffer<TConsensusSpec::Addr>,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
}

impl<TConsensusSpec> OnInboundMessage<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        network: Network,
        config: HotstuffConfig,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        vote_signing_service: TConsensusSpec::SignatureService,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    ) -> Self {
        let (tx_msg_ready, rx_msg_ready) = mpsc::unbounded_channel();
        Self {
            network,
            config,
            store,
            epoch_manager,
            leader_strategy,
            vote_signing_service,
            outbound_messaging,
            tx_msg_ready,
            message_buffer: MessageBuffer::new(rx_msg_ready),
            transaction_pool,
        }
    }

    pub async fn handle(
        &mut self,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage,
    ) -> Result<(), HotStuffError> {
        match msg {
            HotstuffMessage::Proposal(msg) => {
                self.process_local_proposal(current_height, msg).await?;
            },
            HotstuffMessage::ForeignProposal(ref proposal) => {
                self.check_proposal(&proposal.block)
                    .await?;
                self.report_message_ready(from, msg)?;
            },
            msg => {
                self.report_message_ready(from, msg)?;
            },
        }
        Ok(())
    }

    /// Returns the next message that is ready for consensus. The future returned from this function is cancel safe, and
    /// can be used with tokio::select! macro.
    pub async fn next_message(&mut self, current_height: NodeHeight) -> IncomingMessageResult<TConsensusSpec::Addr> {
        self.message_buffer.next(current_height).await
    }

    /// Discards all buffered messages including ones queued up for processing and returns when complete.
    pub async fn discard(&mut self) {
        self.message_buffer.discard().await;
    }

    pub fn clear_buffer(&mut self) {
        self.message_buffer.clear_buffer();
    }

    async fn check_proposal(
        &mut self,
        block: &Block,
    ) -> Result<(), HotStuffError> {


        check_block::<TConsensusSpec>(
            block,
            &self.epoch_manager,
            &self.config,
            self.network,
            &self.leader_strategy,
            &self.vote_signing_service,
        )
        .await?;
        Ok(())
    }

    async fn process_local_proposal(
        &mut self,
        current_height: NodeHeight,
        proposal: ProposalMessage,
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
            info!(
                target: LOG_TARGET,
                "🔥 Block {} is lower than current height {}. Ignoring.",
                block,
                current_height
            );
            return Ok(());
        }

        self.check_proposal(&block).await?;
        let Some(ready_block) = self.handle_missing_transactions(block).await? else {
            // Block not ready
            return Ok(());
        };

        let vn = self
            .epoch_manager
            .get_validator_node_by_public_key(ready_block.epoch(), ready_block.proposed_by())
            .await?;

        self.report_message_ready(
            vn.address,
            HotstuffMessage::Proposal(ProposalMessage { block: ready_block }),
        )?;

        Ok(())
    }

    pub async fn update_parked_blocks(
        &self,
        current_height: NodeHeight,
        transaction_id: &TransactionId,
    ) -> Result<(), HotStuffError> {
        let maybe_unparked_block = self
            .store
            .with_write_tx(|tx| tx.missing_transactions_remove(current_height, transaction_id))?;

        if let Some(unparked_block) = maybe_unparked_block {
            info!(target: LOG_TARGET, "♻️ all transactions for block {unparked_block} are ready for consensus");

            // todo(hacky): ensure that all transactions are in the pool. Race condition: because we have not yet
            // received it yet in the select! loop.
            self.store.with_write_tx(|tx| {
                for tx_id in unparked_block.all_transaction_ids() {
                    if self.transaction_pool.exists(&**tx, tx_id)? {
                        continue;
                    }

                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Transaction {} is missing from the transaction pool. Attempting to recover.",
                        tx_id
                    );

                    let transaction = TransactionRecord::get(&**tx, tx_id)?;
                    // Did the mempool execute it?
                    if transaction.is_executed() {
                        // This should never fail
                        let executed = ExecutedTransaction::try_from(transaction)?;
                        self.transaction_pool.insert(tx, executed.to_atom())?;
                    } else {
                        // Deferred execution
                        self.transaction_pool
                            .insert(tx, TransactionAtom::deferred(*transaction.id()))?;
                    }
                }

                Ok::<_, HotStuffError>(())
            })?;

            let vn = self
                .epoch_manager
                .get_validator_node_by_public_key(unparked_block.epoch(), unparked_block.proposed_by())
                .await?;

            self.report_message_ready(
                vn.address,
                HotstuffMessage::Proposal(ProposalMessage { block: unparked_block }),
            )?;
        }
        Ok(())
    }

    fn report_message_ready(&self, from: TConsensusSpec::Addr, msg: HotstuffMessage) -> Result<(), HotStuffError> {
        self.tx_msg_ready
            .send((from, msg))
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_msg_ready in InboundMessageWorker::handle_hotstuff_message",
            })
    }

    async fn handle_missing_transactions(&mut self, block: Block) -> Result<Option<Block>, HotStuffError> {
        let (missing_tx_ids, awaiting_execution) = self
            .store
            .with_write_tx(|tx| self.check_for_missing_transactions(tx, &block))?;

        if !missing_tx_ids.is_empty() || !awaiting_execution.is_empty() {
            info!(
                target: LOG_TARGET,
                "🔥 Block {} has {} missing transactions and {} awaiting execution", block, missing_tx_ids.len(), awaiting_execution.len(),
            );

            if !missing_tx_ids.is_empty() {
                let block_id = *block.id();
                let epoch = block.epoch();
                let block_proposed_by = block.proposed_by().clone();

                let vn = self
                    .epoch_manager
                    .get_validator_node_by_public_key(epoch, &block_proposed_by)
                    .await?;

                self.outbound_messaging
                    .send(
                        vn.address,
                        HotstuffMessage::RequestMissingTransactions(RequestMissingTransactionsMessage {
                            block_id,
                            epoch,
                            transactions: missing_tx_ids,
                        }),
                    )
                    .await?;
            }

            return Ok(None);
        }

        Ok(Some(block))
    }

    fn check_for_missing_transactions(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
    ) -> Result<(HashSet<TransactionId>, HashSet<TransactionId>), HotStuffError> {
        if block.commands().is_empty() {
            return Ok((HashSet::new(), HashSet::new()));
        }
        let (transactions, missing_tx_ids) = TransactionRecord::get_any(&**tx, block.all_transaction_ids())?;
        let awaiting_execution_or_deferred = transactions
            .into_iter()
            .filter(|tx| tx.final_decision.is_some())
            .filter(|tx| tx.result.is_none())
            .map(|tx| *tx.transaction.id())
            .collect::<HashSet<_>>();

        // TODO(hacky): improve this. We need to account for transactions that are deferred when determining which
        // transactions are awaiting execution.
        let mut awaiting_execution = HashSet::new();
        for id in &awaiting_execution_or_deferred {
            if let Some(t) = TransactionPoolRecord::get(&**tx, id).optional()? {
                if !t.is_deferred() {
                    awaiting_execution.insert(*id);
                }
            }
        }

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
}

pub struct MessageBuffer<TAddr> {
    buffer: BTreeMap<NodeHeight, VecDeque<(TAddr, HotstuffMessage)>>,
    rx_msg_ready: mpsc::UnboundedReceiver<(TAddr, HotstuffMessage)>,
}

impl<TAddr: NodeAddressable> MessageBuffer<TAddr> {
    pub fn new(rx_msg_ready: mpsc::UnboundedReceiver<(TAddr, HotstuffMessage)>) -> Self {
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
                    if msg.proposal().is_some() {
                        info!(target: LOG_TARGET, "Proposal {} is for future block {}. Current height {}", msg, h, current_height);
                    } else {
                        debug!(target: LOG_TARGET, "Message {} is for future height {}. Current height {}", msg, h, current_height);
                    }
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
    ) -> Result<Option<(TAddr, HotstuffMessage)>, NeedsSync<TAddr>> {
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

    fn push_to_buffer(&mut self, height: NodeHeight, from: TAddr, msg: HotstuffMessage) {
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

fn msg_height(msg: &HotstuffMessage) -> Option<NodeHeight> {
    match msg {
        HotstuffMessage::Proposal(msg) => Some(msg.block.height()),
        // Votes for block 2, occur at current height 3
        HotstuffMessage::Vote(msg) => Some(msg.block_height.saturating_add(NodeHeight(1))),
        _ => None,
    }
}
