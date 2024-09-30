//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{collections::HashSet, fmt::Display, iter};

use libp2p::{gossipsub, PeerId};
use log::*;
use tari_dan_common_types::{optional::Optional, NumPreshards, PeerAddress, ShardGroup, ToSubstateAddress};
use tari_dan_p2p::{DanMessage, NewTransactionMessage, TariMessagingSpec};
use tari_dan_storage::{consensus_models::TransactionRecord, StateStore};
use tari_engine_types::commit_result::RejectReason;
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerEvent, EpochManagerReader};
use tari_networking::NetworkingHandle;
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{mpsc, oneshot};

#[cfg(feature = "metrics")]
use super::metrics::PrometheusMempoolMetrics;
use super::MempoolError;
use crate::{
    consensus::ConsensusHandle,
    p2p::services::mempool::{gossip::MempoolGossip, handle::MempoolRequest},
    transaction_validators::TransactionValidationError,
    validator::Validator,
};

const LOG_TARGET: &str = "tari::validator_node::mempool::service";

#[derive(Debug)]
pub struct MempoolService<TValidator> {
    transactions: HashSet<TransactionId>,
    mempool_requests: mpsc::Receiver<MempoolRequest>,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    before_execute_validator: TValidator,
    state_store: SqliteStateStore<PeerAddress>,
    gossip: MempoolGossip<PeerAddress>,
    consensus_handle: ConsensusHandle,
    #[cfg(feature = "metrics")]
    metrics: PrometheusMempoolMetrics,
}

impl<TValidator> MempoolService<TValidator>
where TValidator: Validator<Transaction, Context = (), Error = TransactionValidationError>
{
    pub(super) fn new(
        num_preshards: NumPreshards,
        mempool_requests: mpsc::Receiver<MempoolRequest>,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        before_execute_validator: TValidator,
        state_store: SqliteStateStore<PeerAddress>,
        consensus_handle: ConsensusHandle,
        networking: NetworkingHandle<TariMessagingSpec>,
        rx_gossip: mpsc::UnboundedReceiver<(PeerId, gossipsub::Message)>,
        #[cfg(feature = "metrics")] metrics: PrometheusMempoolMetrics,
    ) -> Self {
        Self {
            gossip: MempoolGossip::new(num_preshards, epoch_manager.clone(), networking, rx_gossip),
            transactions: Default::default(),
            mempool_requests,
            epoch_manager,
            before_execute_validator,
            state_store,
            consensus_handle,
            #[cfg(feature = "metrics")]
            metrics,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut events = self.epoch_manager.subscribe().await?;

        loop {
            tokio::select! {
                Some(req) = self.mempool_requests.recv() => self.handle_request(req).await,
                Some(result) = self.gossip.next_message() => {
                    if let Err(e) = self.handle_new_transaction_from_remote(result).await {
                        warn!(target: LOG_TARGET, "Mempool rejected transaction: {}", e);
                    }
                }
                Ok(event) = events.recv() => {
                    if let EpochManagerEvent::EpochChanged(epoch) = event {
                        if self.epoch_manager.is_this_validator_registered_for_epoch(epoch).await?{
                            info!(target: LOG_TARGET, "Mempool service subscribing transaction messages for epoch {}", epoch);
                            self.gossip.subscribe(epoch).await?;
                        }
                    }
                },

                else => {
                    info!(target: LOG_TARGET, "Mempool service shutting down");
                    break;
                }
            }
        }

        self.gossip.unsubscribe().await?;

        Ok(())
    }

    async fn handle_request(&mut self, request: MempoolRequest) {
        match request {
            MempoolRequest::SubmitTransaction { transaction, reply } => {
                handle(reply, self.handle_new_transaction_from_local(*transaction).await);
            },
            MempoolRequest::RemoveTransactions { transaction_ids, reply } => {
                let num_found = self.remove_transactions(&transaction_ids);
                handle::<_, MempoolError>(reply, Ok(num_found));
            },
            MempoolRequest::GetMempoolSize { reply } => {
                let _ignore = reply.send(self.transactions.len());
            },
        }
    }

    fn remove_transactions(&mut self, ids: &[TransactionId]) -> usize {
        let mut num_found = 0;
        for id in ids {
            if self.transactions.remove(id) {
                num_found += 1;
            }
        }
        num_found
    }

    async fn handle_new_transaction_from_local(&mut self, transaction: Transaction) -> Result<(), MempoolError> {
        if self.transaction_exists(transaction.id())? {
            return Ok(());
        }
        info!(
            target: LOG_TARGET,
            "🎱 Received NEW transaction from local: {transaction}",
        );

        self.handle_new_transaction(transaction, None, self.gossip.get_num_incoming_messages())
            .await?;

        Ok(())
    }

    async fn handle_new_transaction_from_remote(
        &mut self,
        result: Result<(PeerAddress, DanMessage, usize), MempoolError>,
    ) -> Result<(), MempoolError> {
        let (from, msg, num_pending) = result?;
        let DanMessage::NewTransaction(msg) = msg;
        let NewTransactionMessage { transaction } = *msg;

        if !self.consensus_handle.is_running() {
            info!(
                target: LOG_TARGET,
                "🎱 Transaction {} received while not in running state. Ignoring",
                transaction.id()
            );
            return Ok(());
        }

        if self.transaction_exists(transaction.id())? {
            return Ok(());
        }
        debug!(
            target: LOG_TARGET,
            "Received NEW transaction from {}: {} {:?}",
            from,
            transaction.id(),
            transaction
        );

        let current_epoch = self.consensus_handle.current_view().get_epoch();
        let maybe_sender_committee_info = self
            .epoch_manager
            .get_committee_info_by_validator_address(current_epoch, &from)
            .await
            .optional()?;

        self.handle_new_transaction(
            transaction,
            maybe_sender_committee_info.map(|c| c.shard_group()),
            num_pending,
        )
        .await?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_new_transaction(
        &mut self,
        transaction: Transaction,
        sender_shard_group: Option<ShardGroup>,
        num_pending: usize,
    ) -> Result<(), MempoolError> {
        #[cfg(feature = "metrics")]
        self.metrics.on_transaction_received(&transaction);

        if let Err(e) = self.before_execute_validator.validate(&(), &transaction) {
            let transaction_id = *transaction.id();
            self.state_store.with_write_tx(|tx| {
                TransactionRecord::new(transaction)
                    .set_abort_reason(RejectReason::InvalidTransaction(format!(
                        "Mempool validation failed: {e}"
                    )))
                    .insert(tx)
            })?;

            #[cfg(feature = "metrics")]
            self.metrics.on_transaction_validation_error(&transaction_id, &e);
            return Err(e.into());
        }

        // Get the shards involved in claim fees.
        let fee_claims = transaction.fee_claims().collect::<Vec<_>>();

        let claim_shards = if fee_claims.is_empty() {
            HashSet::new()
        } else {
            #[allow(clippy::mutable_key_type)]
            let validator_nodes = self.epoch_manager.get_many_validator_nodes(fee_claims).await?;
            validator_nodes.values().map(|vn| vn.shard_key).collect::<HashSet<_>>()
        };

        if transaction.num_unique_inputs() == 0 && claim_shards.is_empty() {
            warn!(target: LOG_TARGET, "⚠ No involved shards for payload");
        }

        let current_epoch = self.consensus_handle.current_view().get_epoch();
        let tx_substate_address = transaction.id().to_substate_address();

        let local_committee_shard = self.epoch_manager.get_local_committee_info(current_epoch).await?;
        let is_input_shard = transaction.is_involved_inputs(&local_committee_shard);
        let is_output_shard = local_committee_shard.includes_any_address(
            // Known output shards
            // This is to allow for the txreceipt output and indicates shard involvement.
            iter::once(&tx_substate_address).chain(claim_shards.iter()),
        );

        if is_input_shard || is_output_shard {
            debug!(target: LOG_TARGET, "🎱 New transaction {} in mempool", transaction.id());
            self.transactions.insert(*transaction.id());
            self.consensus_handle
                .notify_new_transaction(transaction.clone(), num_pending)
                .await
                .map_err(|_| MempoolError::ConsensusChannelClosed)?;

            // If we received the message from our local shard group, we don't need to gossip it again on the topic
            // (prevents Duplicate errors)
            if sender_shard_group != Some(local_committee_shard.shard_group()) {
                // This validator is involved, we to send the transaction to local replicas
                if let Err(e) = self
                    .gossip
                    .forward_to_local_replicas(
                        current_epoch,
                        NewTransactionMessage {
                            transaction: transaction.clone(),
                        }
                        .into(),
                    )
                    .await
                {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Failed to propagate transaction to local replicas: {}",
                        e
                    );
                }
            }
        } else {
            debug!(
                target: LOG_TARGET,
                "🙇 Not in committee for transaction {}",
                transaction.id(),
            );
        }

        debug!(
            target: LOG_TARGET,
            "🎱 Propagating transaction {} ({} input(s))",
            transaction.id(),
            transaction.num_unique_inputs(),
        );
        if let Err(e) = self
            .gossip
            .forward_to_foreign_replicas(current_epoch, NewTransactionMessage { transaction }, sender_shard_group)
            .await
        {
            warn!(
                target: LOG_TARGET,
                "⚠️ Failed to propagate transaction to foreign committee: {}",
                e
            );
        }

        Ok(())
    }

    fn transaction_exists(&self, id: &TransactionId) -> Result<bool, MempoolError> {
        if self.transactions.contains(id) {
            debug!(
                target: LOG_TARGET,
                "🎱 Transaction {} already in mempool",
                id
            );
            return Ok(true);
        }

        let transaction_exists = self.state_store.with_read_tx(|tx| TransactionRecord::exists(tx, id))?;

        if transaction_exists {
            debug!(
                target: LOG_TARGET,
                "🎱 Transaction {} already exists. Ignoring",
                id
            );
            return Ok(true);
        }

        Ok(false)
    }
}

fn handle<T, E: Display>(reply: oneshot::Sender<Result<T, E>>, result: Result<T, E>) {
    if let Err(ref e) = result {
        error!(target: LOG_TARGET, "Request failed with error: {}", e);
    }
    if reply.send(result).is_err() {
        error!(target: LOG_TARGET, "Requester abandoned request");
    }
}
