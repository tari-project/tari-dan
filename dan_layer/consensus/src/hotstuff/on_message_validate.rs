//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{committee::CommitteeInfo, Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, ForeignParkedProposal, TransactionRecord},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_transaction::TransactionId;
use tokio::sync::broadcast;

use super::config::HotstuffConfig;
use crate::{
    block_validations,
    hotstuff::{error::HotStuffError, HotstuffEvent, ProposalValidationError},
    messages::{ForeignProposalMessage, HotstuffMessage, MissingTransactionsRequest, ProposalMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_message_validate";

pub struct OnMessageValidate<TConsensusSpec: ConsensusSpec> {
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
        config: HotstuffConfig,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        vote_signing_service: TConsensusSpec::SignatureService,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        tx_events: broadcast::Sender<HotstuffEvent>,
    ) -> Self {
        Self {
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
        local_committee_info: &CommitteeInfo,
        from: TConsensusSpec::Addr,
        msg: HotstuffMessage,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        match msg {
            HotstuffMessage::Proposal(msg) => self.process_local_proposal(current_height, from, msg).await,
            HotstuffMessage::ForeignProposal(proposal) => {
                self.process_foreign_proposal(local_committee_info, from, proposal)
                    .await
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
        info!(
            target: LOG_TARGET,
            "📜 new unvalidated PROPOSAL message {} from {} (current height = {})",
            proposal.block,
            proposal.block.proposed_by(),
            current_height,
        );

        if proposal.block.height() < current_height {
            info!(
                target: LOG_TARGET,
                "🔥 Block {} is lower than current height {}. Ignoring.",
                proposal.block,
                current_height
            );
            return Ok(MessageValidationResult::Discard);
        }

        if let Err(err) = self.check_proposal(&proposal.block).await {
            return Ok(MessageValidationResult::Invalid {
                from,
                message: HotstuffMessage::Proposal(proposal),
                err,
            });
        }

        self.handle_missing_transactions_local_block(from, proposal).await
    }

    pub fn update_local_parked_blocks(
        &self,
        current_height: NodeHeight,
        transaction_id: &TransactionId,
    ) -> Result<Option<ProposalMessage>, HotStuffError> {
        let maybe_unparked_block = self
            .store
            .with_write_tx(|tx| tx.missing_transactions_remove(current_height, transaction_id))?;

        let Some((unparked_block, foreign_proposals)) = maybe_unparked_block else {
            return Ok(None);
        };

        info!(target: LOG_TARGET, "♻️ all transactions for block {unparked_block} are ready for consensus");

        let _ignore = self.tx_events.send(HotstuffEvent::ParkedBlockReady {
            block: unparked_block.as_leaf_block(),
        });

        Ok(Some(ProposalMessage {
            block: unparked_block,
            foreign_proposals,
        }))
    }

    pub fn update_foreign_parked_blocks(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<ForeignParkedProposal>, HotStuffError> {
        let unparked_foreign_blocks = self
            .store
            .with_write_tx(|tx| ForeignParkedProposal::remove_by_transaction_id(tx, transaction_id))?;

        if unparked_foreign_blocks.is_empty() {
            return Ok(vec![]);
        };

        info!(target: LOG_TARGET, "♻️ all transactions for {} foreign block(s) are ready for consensus", unparked_foreign_blocks.len());

        Ok(unparked_foreign_blocks)
    }

    async fn check_proposal(&self, block: &Block) -> Result<(), HotStuffError> {
        block_validations::check_proposal::<TConsensusSpec>(
            block,
            &self.epoch_manager,
            &self.vote_signing_service,
            &self.leader_strategy,
            &self.config,
        )
        .await?;
        Ok(())
    }

    async fn handle_missing_transactions_local_block(
        &mut self,
        from: TConsensusSpec::Addr,
        proposal: ProposalMessage,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        // TODO: we need to check foreign proposals for missing transactions as well
        // for proposal in proposal.foreign_proposals.iter() {
        //     self.process_foreign_proposal(&CommitteeInfo::default(), from.clone(), proposal.clone())
        //         .await?;
        // }

        let missing_tx_ids = self
            .store
            .with_write_tx(|tx| self.check_for_missing_transactions(tx, &proposal))?;

        if missing_tx_ids.is_empty() {
            return Ok(MessageValidationResult::Ready {
                from,
                message: HotstuffMessage::Proposal(proposal),
            });
        }

        let _ignore = self.tx_events.send(HotstuffEvent::ProposedBlockParked {
            block: proposal.block.as_leaf_block(),
            num_missing_txs: missing_tx_ids.len(),
            // TODO: remove
            num_awaiting_txs: 0,
        });

        Ok(MessageValidationResult::ParkedProposal {
            block_id: *proposal.block.id(),
            epoch: proposal.block.epoch(),
            proposed_by: proposal.block.proposed_by().clone(),
            missing_txs: missing_tx_ids,
        })
    }

    fn check_for_missing_transactions(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        proposal: &ProposalMessage,
    ) -> Result<HashSet<TransactionId>, HotStuffError> {
        if proposal.block.commands().is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} is empty (no missing transactions)", proposal.block
            );
            return Ok(HashSet::new());
        }
        let missing_tx_ids = TransactionRecord::get_missing(&**tx, proposal.block.all_transaction_ids())?;

        if missing_tx_ids.is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} has no missing transactions", proposal.block
            );
            return Ok(HashSet::new());
        }

        info!(
            target: LOG_TARGET,
            "⏳ Block {} has {} missing transactions", proposal.block, missing_tx_ids.len(),
        );

        tx.missing_transactions_insert(&proposal.block, &proposal.foreign_proposals, &missing_tx_ids)?;

        Ok(missing_tx_ids)
    }

    async fn process_foreign_proposal(
        &mut self,
        local_committee_info: &CommitteeInfo,
        from: TConsensusSpec::Addr,
        msg: ForeignProposalMessage,
    ) -> Result<MessageValidationResult<TConsensusSpec::Addr>, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🧩 new unvalidated FOREIGN PROPOSAL message {} from {}",
            msg,
            from
        );

        if msg.block.commands().is_empty() {
            warn!(
                target: LOG_TARGET,
                "❌ Foreign proposal block {} is empty therefore it cannot involve the local shard group", msg.block
            );
            let block_id = *msg.block.id();
            return Ok(MessageValidationResult::Invalid {
                from,
                message: HotstuffMessage::ForeignProposal(msg),
                err: HotStuffError::ProposalValidationError(ProposalValidationError::NoTransactionsInCommittee {
                    block_id,
                }),
            });
        }

        if let Err(err) = self.check_proposal(&msg.block).await {
            return Ok(MessageValidationResult::Invalid {
                from,
                message: HotstuffMessage::ForeignProposal(msg),
                err,
            });
        }

        self.store.with_write_tx(|tx| {
            let mut all_involved_transactions = msg
                .block
                .all_transaction_ids_in_committee(local_committee_info)
                .peekable();
            // CASE: all foreign proposals must include evidence
            if all_involved_transactions.peek().is_none() {
                warn!(
                    target: LOG_TARGET,
                    "❌ Foreign Block {} has no transactions involving our committee", msg.block
                );
                // drop the borrow of msg.block
                drop(all_involved_transactions);
                let block_id = *msg.block.id();
                return Ok(MessageValidationResult::Invalid {
                    from,
                    message: HotstuffMessage::ForeignProposal(msg),
                    err: HotStuffError::ProposalValidationError(ProposalValidationError::NoTransactionsInCommittee {
                        block_id,
                    }),
                });
            }

            let missing_tx_ids = TransactionRecord::get_missing(&**tx, all_involved_transactions)?;

            if missing_tx_ids.is_empty() {
                debug!(
                    target: LOG_TARGET,
                    "✅ Foreign Block {} has no missing transactions", msg.block
                );
                return Ok(MessageValidationResult::Ready {
                    from,
                    message: HotstuffMessage::ForeignProposal(msg),
                });
            }

            info!(
                target: LOG_TARGET,
                "⏳ Foreign Block {} has {} missing transactions", msg.block, missing_tx_ids.len(),
            );

            let parked_block = ForeignParkedProposal::from(msg);
            parked_block.insert(tx)?;
            parked_block.add_missing_transactions(tx, &missing_tx_ids)?;

            Ok(MessageValidationResult::ParkedProposal {
                block_id: *parked_block.block().id(),
                epoch: parked_block.block().epoch(),
                proposed_by: parked_block.block().proposed_by().clone(),
                missing_txs: missing_tx_ids,
            })
        })
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
