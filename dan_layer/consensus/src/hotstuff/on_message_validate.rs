//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_common::configuration::Network;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, TransactionRecord},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::TransactionId;
use tokio::sync::broadcast;

use super::config::HotstuffConfig;
use crate::{
    block_validations,
    hotstuff::{error::HotStuffError, HotstuffEvent},
    messages::{HotstuffMessage, MissingTransactionsRequest, ProposalMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_message_validate";

pub struct OnMessageValidate<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,
    network: Network,
    config: HotstuffConfig,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    vote_signing_service: TConsensusSpec::SignatureService,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    tx_events: broadcast::Sender<HotstuffEvent>,
    /// Keep track of max 16 in-flight requests
    active_missing_transaction_requests: SimpleFixedArray<u32, 16>,
    current_request_id: u32,
}

impl<TConsensusSpec: ConsensusSpec> OnMessageValidate<TConsensusSpec> {
    pub fn new(
        local_validator_addr: TConsensusSpec::Addr,
        network: Network,
        config: HotstuffConfig,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        vote_signing_service: TConsensusSpec::SignatureService,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        tx_events: broadcast::Sender<HotstuffEvent>,
    ) -> Self {
        Self {
            local_validator_addr,
            network,
            config,
            store,
            epoch_manager,
            leader_strategy,
            vote_signing_service,
            outbound_messaging,
            tx_events,
            active_missing_transaction_requests: SimpleFixedArray::new(),
            current_request_id: 0,
        }
    }

    pub async fn handle(
        &mut self,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        match msg {
            HotstuffMessage::Proposal(msg) => self.process_local_proposal(current_height, from, msg).await,
            HotstuffMessage::ForeignProposal(proposal) => {
                if let Err(err) = self.check_proposal(&proposal.block).await {
                    return Ok(MessageValidationResult::Invalid {
                        from,
                        message: HotstuffMessage::Proposal(proposal),
                        err,
                    });
                }
                Ok(MessageValidationResult::Ready {
                    from,
                    message: HotstuffMessage::ForeignProposal(proposal),
                })
            },
            HotstuffMessage::MissingTransactionsResponse(msg) => {
                if !self.active_missing_transaction_requests.remove_element(&msg.request_id) {
                    warn!(target: LOG_TARGET, "❓Received missing transactions (req_id = {}) from {} that we did not request. Discarding message", msg.request_id, from);
                    return Ok(MessageValidationResult::Discard);
                }
                if msg.transactions.len() > 1000 {
                    warn!(target: LOG_TARGET, "⚠️Peer sent more than the maximum amount of transactions. Discarding message");
                    return Ok(MessageValidationResult::Discard);
                }
                Ok(MessageValidationResult::Ready {
                    from,
                    message: HotstuffMessage::MissingTransactionsResponse(msg),
                })
            },
            msg => Ok(MessageValidationResult::Ready { from, message: msg }),
        }
    }

    pub async fn request_missing_transactions(
        &mut self,
        to: TConsensusSpec::Addr,
        block_id: BlockId,
        epoch: Epoch,
        missing_txs: HashSet<TransactionId>,
    ) -> Result<(), HotStuffError> {
        let request_id = self.next_request_id();
        self.active_missing_transaction_requests.insert(request_id);
        self.outbound_messaging
            .send(
                to,
                HotstuffMessage::MissingTransactionsRequest(MissingTransactionsRequest {
                    request_id,
                    block_id,
                    epoch,
                    transactions: missing_txs,
                }),
            )
            .await?;
        Ok(())
    }

    fn next_request_id(&mut self) -> u32 {
        let req_id = self.current_request_id;
        self.current_request_id += 1;
        req_id
    }

    async fn process_local_proposal(
        &mut self,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        proposal: ProposalMessage,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        let ProposalMessage { block } = proposal;

        info!(
            target: LOG_TARGET,
            "📜 [{}] new unvalidated PROPOSAL message {} from {} (current height = {})",
            self.local_validator_addr,
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
            return Ok(MessageValidationResult::Discard);
        }

        if let Err(err) = self.check_proposal(&block).await {
            return Ok(MessageValidationResult::Invalid {
                from,
                message: HotstuffMessage::Proposal(ProposalMessage { block }),
                err,
            });
        }

        self.handle_missing_transactions(from, block).await
    }

    pub async fn update_parked_blocks(
        &self,
        current_height: NodeHeight,
        transaction_id: &TransactionId,
    ) -> Result<Option<(TConsensusSpec::Addr, HotstuffMessage)>, HotStuffError> {
        let maybe_unparked_block = self
            .store
            .with_write_tx(|tx| tx.missing_transactions_remove(current_height, transaction_id))?;

        let Some(unparked_block) = maybe_unparked_block else {
            return Ok(None);
        };

        info!(target: LOG_TARGET, "♻️ all transactions for block {unparked_block} are ready for consensus");

        let vn = self
            .epoch_manager
            .get_validator_node_by_public_key(unparked_block.epoch(), unparked_block.proposed_by())
            .await?;

        let _ignore = self.tx_events.send(HotstuffEvent::ParkedBlockReady {
            block: unparked_block.as_leaf_block(),
        });

        Ok(Some((
            vn.address,
            HotstuffMessage::Proposal(ProposalMessage { block: unparked_block }),
        )))
    }

    async fn check_proposal(&self, block: &Block) -> Result<(), HotStuffError> {
        block_validations::check_proposal::<TConsensusSpec>(
            block,
            self.network,
            &self.epoch_manager,
            &self.vote_signing_service,
            &self.leader_strategy,
            &self.config,
        )
        .await?;
        Ok(())
    }

    async fn handle_missing_transactions(
        &mut self,
        from: TConsensusSpec::Addr,
        block: Block,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        let missing_tx_ids = self
            .store
            .with_write_tx(|tx| self.check_for_missing_transactions(tx, &block))?;

        if missing_tx_ids.is_empty() {
            return Ok(MessageValidationResult::Ready {
                from,
                message: HotstuffMessage::Proposal(ProposalMessage { block }),
            });
        }

        let _ignore = self.tx_events.send(HotstuffEvent::ProposedBlockParked {
            block: block.as_leaf_block(),
            num_missing_txs: missing_tx_ids.len(),
            // TODO: remove
            num_awaiting_txs: 0,
        });

        Ok(MessageValidationResult::ParkedProposal {
            block_id: *block.id(),
            epoch: block.epoch(),
            proposed_by: block.proposed_by().clone(),
            missing_txs: missing_tx_ids,
        })
    }

    fn check_for_missing_transactions(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
    ) -> Result<HashSet<TransactionId>, HotStuffError> {
        if block.commands().is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} is empty (no missing transactions)", block
            );
            return Ok(HashSet::new());
        }
        let missing_tx_ids = TransactionRecord::get_missing(&**tx, block.all_transaction_ids())?;

        if missing_tx_ids.is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} has no missing transactions", block
            );
            return Ok(HashSet::new());
        }

        info!(
            target: LOG_TARGET,
            "⏳ Block {} has {} missing transactions", block, missing_tx_ids.len(),
        );

        tx.missing_transactions_insert(block, &missing_tx_ids, &[])?;

        Ok(missing_tx_ids)
    }
}

#[derive(Debug)]
pub enum MessageValidationResult<TAddr> {
    Ready {
        from: TAddr,
        message: HotstuffMessage,
    },
    ParkedProposal {
        block_id: BlockId,
        epoch: Epoch,
        proposed_by: PublicKey,
        missing_txs: HashSet<TransactionId>,
    },
    Discard,
    Invalid {
        from: TAddr,
        message: HotstuffMessage,
        err: HotStuffError,
    },
}

#[derive(Debug, Clone)]
struct SimpleFixedArray<T, const SZ: usize> {
    elems: [Option<T>; SZ],
    ptr: usize,
}

impl<T: Copy, const SZ: usize> SimpleFixedArray<T, SZ> {
    pub fn new() -> Self {
        Self {
            elems: [None; SZ],
            ptr: 0,
        }
    }

    pub fn insert(&mut self, elem: T) {
        // We dont care about overwriting "old" elements
        self.elems[self.ptr] = Some(elem);
        self.ptr = (self.ptr + 1) % SZ;
    }

    pub fn remove_element(&mut self, elem: &T) -> bool
    where T: PartialEq {
        for (i, e) in self.elems.iter().enumerate() {
            if e.as_ref() == Some(elem) {
                // We dont care about "holes" in the collection
                self.elems[i] = None;
                return true;
            }
        }
        false
    }
}

impl<const SZ: usize, T: Copy> Default for SimpleFixedArray<T, SZ> {
    fn default() -> Self {
        Self::new()
    }
}
