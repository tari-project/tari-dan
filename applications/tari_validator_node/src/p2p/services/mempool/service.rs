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

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    sync::Arc,
};

use log::*;
use tari_comms::NodeIdentity;
use tari_dan_common_types::{ShardId, TreeNodeHash};
use tari_dan_core::{message::DanMessage, services::infrastructure_services::OutboundService};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManager};
use tari_template_lib::Hash;
use tari_transaction::Transaction;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::MempoolError;
use crate::{
    dry_run_transaction_processor::DryRunTransactionProcessor,
    p2p::services::{
        mempool::{handle::MempoolRequest, Validator},
        messaging::OutboundMessaging,
    },
};

const LOG_TARGET: &str = "tari::validator_node::mempool::service";

#[derive(Debug)]
pub struct MempoolService<V> {
    transactions: HashMap<Hash, (Transaction, Option<TreeNodeHash>)>,
    new_transactions: mpsc::Receiver<Transaction>,
    mempool_requests: mpsc::Receiver<MempoolRequest>,
    outbound: OutboundMessaging,
    tx_valid_transactions: broadcast::Sender<(Transaction, ShardId)>,
    epoch_manager: EpochManagerHandle,
    node_identity: Arc<NodeIdentity>,
    validator: V,
    dry_run_processor: DryRunTransactionProcessor,
}

impl<V> MempoolService<V>
where
    V: Validator<Transaction, Error = MempoolError>,
{
    pub(super) fn new(
        new_transactions: mpsc::Receiver<Transaction>,
        mempool_requests: mpsc::Receiver<MempoolRequest>,
        outbound: OutboundMessaging,
        tx_valid_transactions: broadcast::Sender<(Transaction, ShardId)>,
        epoch_manager: EpochManagerHandle,
        node_identity: Arc<NodeIdentity>,
        validator: V,
        dry_run_processor: DryRunTransactionProcessor,
    ) -> Self {
        Self {
            transactions: Default::default(),
            new_transactions,
            mempool_requests,
            outbound,
            tx_valid_transactions,
            epoch_manager,
            node_identity,
            validator,
            dry_run_processor,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                Some(req) = self.mempool_requests.recv() => self.handle_request(req).await,
                Some(tx) = self.new_transactions.recv() => {
                    if let Err(e) = self .handle_new_transaction(tx).await {
                        warn!(target: LOG_TARGET, "Mempool rejected transaction: {}", e);
                    }
                }

                else => {
                    info!(target: LOG_TARGET, "Mempool service shutting down");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, request: MempoolRequest) {
        match request {
            MempoolRequest::SubmitTransaction(transaction, reply) => {
                handle(reply, self.handle_new_transaction(*transaction).await);
            },
            MempoolRequest::RemoveTransaction { transaction_hash } => self.remove_transaction(&transaction_hash),
            MempoolRequest::GetMempoolSize { reply } => {
                let _ignore = reply.send(self.transactions.len());
            },
        }
    }

    fn remove_transaction(&mut self, hash: &Hash) {
        self.transactions.remove(hash);
    }

    async fn handle_new_transaction(&mut self, mut transaction: Transaction) -> Result<(), MempoolError> {
        debug!(
            target: LOG_TARGET,
            "Received transaction: {} {:?}",
            transaction.hash(),
            transaction
        );

        if self.transactions.contains_key(transaction.hash()) {
            info!(
                target: LOG_TARGET,
                "🎱 Transaction {} already in mempool",
                transaction.hash()
            );
            return Ok(());
        }

        self.validator.validate(&transaction).await?;

        // we do a dry run processor to precalculate shards for new outputs,
        // so the client does not need to specify them
        if let Err(e) = self.dry_run_processor.add_missing_shards(&mut transaction).await {
            error!(target: LOG_TARGET, "Could not add missing shards: {}", e);
        }

        let shards = transaction.meta().involved_shards();
        if shards.is_empty() {
            warn!(target: LOG_TARGET, "⚠ No involved shards for payload");
        }

        let current_node_pubkey = self.node_identity.public_key();

        let mut committee_shards = Vec::with_capacity(shards.len());
        // TODO(perf): n queries
        for sid in &shards {
            if self
                .epoch_manager
                .is_validator_in_committee_for_current_epoch(*sid, current_node_pubkey.clone())
                .await?
            {
                committee_shards.push(*sid);
            }
        }

        if committee_shards.is_empty() {
            info!(
                target: LOG_TARGET,
                "🙇 Not in committee for transaction {}",
                transaction.hash()
            );
        } else {
            debug!(target: LOG_TARGET, "🎱 New transaction in mempool");
            self.transactions
                .insert(*transaction.hash(), (transaction.clone(), None));
        }

        // TODO: if we're in the committee for this transaction, check if the transaction is in-progress/finalized
        //       locally. If so, do not propagate.
        if let Err(e) = self.propagate_transaction(&transaction, &shards).await {
            error!(
                target: LOG_TARGET,
                "Unable to propagate transaction among peers: {}",
                e.to_string()
            )
        }

        for shard_id in committee_shards {
            debug!(
                target: LOG_TARGET,
                " 🚀 Sending transaction {} for shard {} to consensus",
                transaction.hash(),
                shard_id
            );
            if let Err(err) = self.tx_valid_transactions.send((transaction.clone(), shard_id)) {
                error!(
                    target: LOG_TARGET,
                    "Failed to send valid transaction to shard: {}: {}", shard_id, err
                );
            }
        }
        Ok(())
    }

    pub async fn propagate_transaction(
        &mut self,
        transaction: &Transaction,
        shards: &[ShardId],
    ) -> Result<(), MempoolError> {
        let epoch = self.epoch_manager.current_epoch().await?;
        let committees = self.epoch_manager.get_committees(epoch, shards).await?;

        let msg = DanMessage::NewTransaction(Box::new(transaction.clone()));

        // propagate over the involved shard ids
        #[allow(clippy::mutable_key_type)]
        let unique_members = committees
            .into_iter()
            .flat_map(|s| s.committee.members)
            .filter(|pk| pk != self.node_identity.public_key())
            .collect::<HashSet<_>>();
        let committees = unique_members.into_iter().collect::<Vec<_>>();

        self.outbound
            .broadcast(self.node_identity.public_key().clone(), &committees, msg)
            .await?;

        Ok(())
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
