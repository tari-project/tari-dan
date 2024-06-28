//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeMap, HashSet, VecDeque};

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::{optional::Optional, Epoch, NodeAddressable, NodeHeight};
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
use tokio::sync::{broadcast, mpsc};

use super::config::HotstuffConfig;
use crate::{
    block_validations::{
        check_base_layer_block_hash,
        check_hash_and_height,
        check_network,
        check_proposed_by_leader,
        check_quorum_certificate,
        check_signature,
    },
    hotstuff::{error::HotStuffError, HotstuffEvent},
    messages::{HotstuffMessage, ProposalMessage, RequestMissingTransactionsMessage},
    traits::{hooks::ConsensusHooks, ConsensusSpec, InboundMessaging, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::inbound_messages";

pub type IncomingMessageResult<TAddr> = Result<Option<(TAddr, HotstuffMessage)>, HotStuffError>;

pub struct OnInboundMessage<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,
    network: Network,
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    vote_signing_service: TConsensusSpec::SignatureService,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    tx_msg_ready: mpsc::UnboundedSender<(TConsensusSpec::Addr, HotstuffMessage)>,
    rx_msg_ready: mpsc::UnboundedReceiver<(TConsensusSpec::Addr, HotstuffMessage)>,
    message_buffer: MessageBuffer<TConsensusSpec>,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_events: broadcast::Sender<HotstuffEvent>,
    hooks: TConsensusSpec::Hooks,
}

impl<TConsensusSpec: ConsensusSpec> OnInboundMessage<TConsensusSpec> {
    pub fn new(
        local_validator_addr: TConsensusSpec::Addr,
        network: Network,
        config: HotstuffConfig,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        vote_signing_service: TConsensusSpec::SignatureService,
        inbound_messaging: TConsensusSpec::InboundMessaging,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_events: broadcast::Sender<HotstuffEvent>,
        hooks: TConsensusSpec::Hooks,
    ) -> Self {
        let (tx_msg_ready, rx_msg_ready) = mpsc::unbounded_channel();
        Self {
            local_validator_addr,
            network,
            config,
            store,
            epoch_manager,
            leader_strategy,
            vote_signing_service,
            outbound_messaging,
            tx_msg_ready,
            rx_msg_ready,
            message_buffer: MessageBuffer::new(inbound_messaging),
            transaction_pool,
            tx_events,
            hooks,
        }
    }

    /// Returns the next message that is ready for consensus. The future returned from this function is cancel safe, and
    /// can be used with tokio::select! macro.
    pub async fn next_message(
        &mut self,
        current_epoch: Epoch,
        current_height: NodeHeight,
    ) -> IncomingMessageResult<TConsensusSpec::Addr> {
        loop {
            tokio::select! {
                biased;
                // Return all the unparked messages first
                Some(addr_and_msg) = self.rx_msg_ready.recv() => {
                    return Ok(Some(addr_and_msg));
                },

                // Then incoming messages for the current epoch/height
                result = self.message_buffer.next(current_epoch, current_height) => {
                    match result {
                        Ok(Some((from, msg))) => {
                            self.hooks.on_message_received(&msg);
                            match self.on_message(current_height, from, msg).await {
                                Ok(Some(addr_and_msg)) => return Ok(Some(addr_and_msg)),
                                Ok(None) => {
                                    // Message parked, keep polling
                                },
                                Err(err) => {
                                    self.hooks.on_error(&err);
                                    error!(target: LOG_TARGET, "Error handling message: {}", err);
                                }
                            }
                        },
                        Ok(None) => {
                            // Inbound messages terminated
                            return Ok(None)
                        },
                        Err(err) => {
                            return Err(err);
                        }
                    }
                }
            }
        }
    }

    async fn on_message(
        &mut self,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage,
    ) -> Result<Option<(TConsensusSpec::Addr, HotstuffMessage)>, HotStuffError> {
        match msg {
            HotstuffMessage::Proposal(msg) => self.process_local_proposal(current_height, msg).await,
            HotstuffMessage::ForeignProposal(ref proposal) => {
                self.check_proposal(&proposal.block).await?;
                Ok(Some((from, msg)))
            },
            msg => Ok(Some((from, msg))),
        }
    }

    /// Discards all buffered messages including ones queued up for processing and returns when complete.
    pub async fn discard(&mut self) {
        self.message_buffer.discard().await;
    }

    pub fn clear_buffer(&mut self) {
        self.message_buffer.clear_buffer();
    }

    async fn check_proposal(&self, block: &Block) -> Result<(), HotStuffError> {
        check_base_layer_block_hash::<TConsensusSpec>(block, &self.epoch_manager, &self.config).await?;
        check_network(block, self.network)?;
        check_hash_and_height(block)?;
        let committee_for_block = self
            .epoch_manager
            .get_committee_by_validator_public_key(block.epoch(), block.proposed_by())
            .await?;
        check_proposed_by_leader(&self.leader_strategy, &committee_for_block, block)?;
        check_signature(block)?;
        check_quorum_certificate::<TConsensusSpec>(block, &self.vote_signing_service, &self.epoch_manager).await?;
        Ok(())
    }

    async fn process_local_proposal(
        &mut self,
        current_height: NodeHeight,
        proposal: ProposalMessage,
    ) -> Result<Option<(TConsensusSpec::Addr, HotstuffMessage)>, HotStuffError> {
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
            return Ok(None);
        }

        self.check_proposal(&block).await?;
        let Some(ready_block) = self.handle_missing_transactions(block).await? else {
            // Block not ready -park it
            return Ok(None);
        };

        let vn = self
            .epoch_manager
            .get_validator_node_by_public_key(ready_block.epoch(), ready_block.proposed_by())
            .await?;

        Ok(Some((
            vn.address,
            HotstuffMessage::Proposal(ProposalMessage { block: ready_block }),
        )))
    }

    pub async fn update_parked_blocks(
        &mut self,
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

            let _ignore = self.tx_events.send(HotstuffEvent::ParkedBlockReady {
                block: unparked_block.as_leaf_block(),
            });

            self.notify_message_ready(
                vn.address,
                HotstuffMessage::Proposal(ProposalMessage { block: unparked_block }),
            )?;
        }
        Ok(())
    }

    fn notify_message_ready(&self, from: TConsensusSpec::Addr, msg: HotstuffMessage) -> Result<(), HotStuffError> {
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

        if missing_tx_ids.is_empty() && awaiting_execution.is_empty() {
            return Ok(Some(block));
        }

        let _ignore = self.tx_events.send(HotstuffEvent::ProposedBlockParked {
            block: block.as_leaf_block(),
            num_missing_txs: missing_tx_ids.len(),
            num_awaiting_txs: awaiting_execution.len(),
        });

        if !missing_tx_ids.is_empty() {
            let block_id = *block.id();
            let epoch = block.epoch();
            let block_proposed_by = block.proposed_by().clone();

            let vn = self
                .epoch_manager
                .get_validator_node_by_public_key(epoch, &block_proposed_by)
                .await?;

            let mut request_from_address = vn.address;

            // (Yet another) Edge case: If we're catching up, we could be the proposer but we no longer have the
            // transaction (we deleted our database) In this case, request from another random VN
            // (TODO: not 100% reliable)
            if request_from_address == self.local_validator_addr {
                let mut local_committee = self.epoch_manager.get_local_committee(epoch).await?;

                local_committee.shuffle();
                match local_committee
                    .into_iter()
                    .filter(|(addr, _)| *addr != self.local_validator_addr)
                    .next()
                {
                    Some((addr, _)) => {
                        warn!(target: LOG_TARGET, "⚠️Requesting missing transactions from another validator {addr} because we are (presumably) catching up (local_peer_id = {})", self.local_validator_addr);
                        request_from_address = addr;
                    },
                    None => {
                        warn!(
                            target: LOG_TARGET,
                            "❌NEVERHAPPEN: We're the only validator in the committee but we need to request missing transactions."
                        );
                        return Ok(None);
                    },
                }
            }

            self.outbound_messaging
                .send(
                    request_from_address,
                    HotstuffMessage::RequestMissingTransactions(RequestMissingTransactionsMessage {
                        block_id,
                        epoch,
                        transactions: missing_tx_ids,
                    }),
                )
                .await?;
        }

        Ok(None)
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
            "⏳ Block {} has {} missing transactions and {} awaiting execution", block, missing_tx_ids.len(), awaiting_execution.len(),
        );

        tx.missing_transactions_insert(block, &missing_tx_ids, &awaiting_execution)?;

        Ok((missing_tx_ids, awaiting_execution))
    }
}

pub struct MessageBuffer<TConsensusSpec: ConsensusSpec> {
    buffer: BTreeMap<(Epoch, NodeHeight), VecDeque<(TConsensusSpec::Addr, HotstuffMessage)>>,
    inbound_messaging: TConsensusSpec::InboundMessaging,
}

impl<TConsensusSpec: ConsensusSpec> MessageBuffer<TConsensusSpec> {
    pub fn new(inbound_messaging: TConsensusSpec::InboundMessaging) -> Self {
        Self {
            buffer: BTreeMap::new(),
            inbound_messaging,
        }
    }

    pub async fn next(
        &mut self,
        current_epoch: Epoch,
        current_height: NodeHeight,
    ) -> IncomingMessageResult<TConsensusSpec::Addr> {
        loop {
            // Clear buffer with lower (epoch, heights)
            self.buffer = self.buffer.split_off(&(current_epoch, current_height));

            // Check if message is in the buffer
            if let Some(buffer) = self.buffer.get_mut(&(current_epoch, current_height)) {
                if let Some(msg_tuple) = buffer.pop_front() {
                    return Ok(Some(msg_tuple));
                }
            }

            // while let Some((from, msg)) = self.next_message_or_sync(current_epoch, current_height).await? {
            while let Some(result) = self.inbound_messaging.next_message().await {
                let (from, msg) = result?;
                match msg_epoch_and_height(&msg) {
                    // Discard old message
                    Some((e, h)) if e < current_epoch || h < current_height => {
                        info!(target: LOG_TARGET, "Discard message {} is for previous view {}/{}. Current view {}/{}", msg, e, h, current_epoch,current_height);
                        continue;
                    },
                    // Buffer message for future epoch/height
                    Some((epoch, height)) if epoch > current_epoch || height > current_height => {
                        if msg.proposal().is_some() {
                            info!(target: LOG_TARGET, "🦴Proposal {msg} is for future view (Current view: {current_epoch}, {current_height})");
                        } else {
                            debug!(target: LOG_TARGET, "🦴Message {msg} is for future view (Current view: {current_epoch}, {current_height})");
                        }
                        self.push_to_buffer(epoch, height, from, msg);
                        continue;
                    },
                    // Height is irrelevant or current, return message
                    _ => return Ok(Some((from, msg))),
                }
            }

            // Inbound messaging has terminated
            return Ok(None);
        }
    }

    pub async fn discard(&mut self) {
        self.clear_buffer();
        while self.inbound_messaging.next_message().await.is_some() {}
    }

    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
    }

    // async fn next_message_or_sync(
    //     &mut self,
    //     current_epoch: Epoch,
    //     current_height: NodeHeight,
    // ) -> Result<Option<(TConsensusSpec::Addr, HotstuffMessage)>, NeedsSync<TConsensusSpec::Addr>> {
    //     // loop {
    //     //     if let Some(addr_and_msg) = self.rx_msg_ready.recv().await {
    //     //         return Ok(Some(addr_and_msg));
    //     //     }
    //     //
    //     //     // Check if we have any proposals that exceed the current view
    //     //     for queue in self.buffer.values() {
    //     //         for (from, msg) in queue {
    //     //             if let Some(proposal) = msg.proposal() {
    //     //                 if proposal.block.justify().epoch() > current_epoch ||
    //     //                     proposal.block.justify().block_height() > current_height
    //     //                 {
    //     //                     return Err(NeedsSync {
    //     //                         from: from.clone(),
    //     //                         local_height: current_height,
    //     //                         qc_height: proposal.block.justify().block_height(),
    //     //                         remote_epoch: proposal.block.justify().epoch(),
    //     //                         local_epoch: current_epoch,
    //     //                     });
    //     //                 }
    //     //             }
    //     //         }
    //     //     }
    //     //
    //     //     // Don't really like this but because we can receive proposals out of order, we need to wait a bit to
    // see     //     // if we get a proposal at our height without switching to sync.
    //     //     //     let timeout = time::sleep(time::Duration::from_secs(2));
    //     //     //     tokio::pin!(timeout);
    //     //     //     tokio::select! {
    //     //     //         msg = self.rx_msg_ready.recv() => return Ok(msg),
    //     //     //         _ = timeout.as_mut() => {
    //     //     //             // Check if we have any proposals
    //     //     //             for queue in self.buffer.values() {
    //     //     //                 for (from, msg) in queue {
    //     //     //                    if let Some(proposal) = msg.proposal() {
    //     //     //                         if proposal.block.justify().epoch() > current_epoch ||
    //     //     // proposal.block.justify().block_height() > current_height {
    //     //     // return Err(NeedsSync {                                 from: from.clone(),
    //     //     //                                 local_height: current_height,
    //     //     //                                 qc_height: proposal.block.justify().block_height(),
    //     //     //                                 remote_epoch: proposal.block.justify().epoch(),
    //     //     //                                 local_epoch: current_epoch
    //     //     //                             });
    //     //     //                         }
    //     //     //                     }
    //     //     //                 }
    //     //     //             }
    //     //     //         }
    //     //     //     }
    //     // }
    // }

    fn push_to_buffer(&mut self, epoch: Epoch, height: NodeHeight, from: TConsensusSpec::Addr, msg: HotstuffMessage) {
        self.buffer.entry((epoch, height)).or_default().push_back((from, msg));
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Needs sync: local height {local_height} is less than remote QC height {qc_height} from {from}")]
pub struct NeedsSync<TAddr: NodeAddressable> {
    pub from: TAddr,
    pub local_height: NodeHeight,
    pub qc_height: NodeHeight,
    pub remote_epoch: Epoch,
    pub local_epoch: Epoch,
}

fn msg_epoch_and_height(msg: &HotstuffMessage) -> Option<(Epoch, NodeHeight)> {
    match msg {
        HotstuffMessage::Proposal(msg) => Some((msg.block.epoch(), msg.block.height())),
        // Votes for block 2, occur at current height 3
        HotstuffMessage::Vote(msg) => Some((msg.epoch, msg.block_height.saturating_add(NodeHeight(1)))),
        _ => None,
    }
}
