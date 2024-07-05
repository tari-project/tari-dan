//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::{optional::Optional, NodeHeight};
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
use tokio::sync::broadcast;

use super::config::HotstuffConfig;
use crate::{
    block_validations,
    hotstuff::{error::HotStuffError, HotstuffEvent},
    messages::{HotstuffMessage, ProposalMessage, RequestMissingTransactionsMessage},
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
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_events: broadcast::Sender<HotstuffEvent>,
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
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
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
            transaction_pool,
            tx_events,
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
            msg => Ok(MessageValidationResult::Ready { from, message: msg }),
        }
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

        let Some(ready_block) = self.handle_missing_transactions(block).await? else {
            // Block not ready -park it
            return Ok(MessageValidationResult::NotReady);
        };

        // let vn = self
        //     .epoch_manager
        //     .get_validator_node_by_public_key(ready_block.epoch(), ready_block.proposed_by())
        //     .await?;

        Ok(MessageValidationResult::Ready {
            from,
            message: HotstuffMessage::Proposal(ProposalMessage { block: ready_block }),
        })
    }

    pub async fn update_parked_blocks(
        &mut self,
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
                    .find(|(addr, _)| *addr != self.local_validator_addr)
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
            debug!(
                target: LOG_TARGET,
                "✅ Block {} is empty (no missing transactions)", block
            );
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

#[derive(Debug)]
pub enum MessageValidationResult<TAddr> {
    Ready {
        from: TAddr,
        message: HotstuffMessage,
    },
    NotReady,
    Discard,
    Invalid {
        from: TAddr,
        message: HotstuffMessage,
        err: HotStuffError,
    },
}
