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
    collections::HashSet,
    iter::FromIterator,
    sync::{Arc, Mutex},
};

use log::*;
use tari_comms::NodeIdentity;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    message::DanMessage,
    models::{Payload, TariDanPayload},
    services::{epoch_manager::EpochManager, infrastructure_services::OutboundService},
};
use tari_dan_engine::transaction::Transaction;
use tari_template_lib::Hash;
use tokio::sync::{broadcast, mpsc};

use super::{handle::TransactionVecMutex, MempoolError};
use crate::p2p::services::{
    epoch_manager::handle::EpochManagerHandle,
    mempool::handle::MempoolRequest,
    messaging::OutboundMessaging,
};

const LOG_TARGET: &str = "dan::mempool::service";

pub struct MempoolService {
    // TODO: Should be a HashSet
    transactions: TransactionVecMutex,
    new_transactions: mpsc::Receiver<Transaction>,
    mempool_requests: mpsc::Receiver<MempoolRequest>,
    outbound: OutboundMessaging,
    tx_valid_transactions: broadcast::Sender<(Transaction, ShardId)>,
    epoch_manager: EpochManagerHandle,
    node_identity: Arc<NodeIdentity>,
}

impl MempoolService {
    pub(super) fn new(
        new_transactions: mpsc::Receiver<Transaction>,
        mempool_requests: mpsc::Receiver<MempoolRequest>,
        outbound: OutboundMessaging,
        tx_valid_transactions: broadcast::Sender<(Transaction, ShardId)>,
        epoch_manager: EpochManagerHandle,
        node_identity: Arc<NodeIdentity>,
    ) -> Self {
        Self {
            transactions: Arc::new(Mutex::new(Vec::new())),
            new_transactions,
            mempool_requests,
            outbound,
            tx_valid_transactions,
            epoch_manager,
            node_identity,
        }
    }

    pub async fn run(mut self) -> Result<(), MempoolError> {
        loop {
            tokio::select! {
                Some(req) = self.mempool_requests.recv() => self.handle_request(req).await?,
                Some(tx) = self.new_transactions.recv() => self.handle_new_transaction(tx).await?,

                else => {
                    info!(target: LOG_TARGET, "Mempool service shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_request(&mut self, request: MempoolRequest) -> Result<(), MempoolError> {
        match request {
            MempoolRequest::SubmitTransaction(transaction) => self.handle_new_transaction(*transaction).await?,
            MempoolRequest::RemoveTransaction { transaction_hash } => self.remove_transaction(transaction_hash),
        }

        Ok(())
    }

    fn remove_transaction(&mut self, hash: Hash) {
        let mut transactions = self.transactions.lock().unwrap();
        transactions.retain(|(transaction, _)| *transaction.hash() != hash);
    }

    async fn handle_new_transaction(&mut self, transaction: Transaction) -> Result<(), MempoolError> {
        debug!(target: LOG_TARGET, "Received new transaction: {:?}", transaction);
        // TODO: validate transaction
        let payload = TariDanPayload::new(transaction.clone());

        let shards = payload.involved_shards();
        debug!(target: LOG_TARGET, "New Payload in mempool for shards: {:?}", shards);
        if shards.is_empty() {
            warn!(target: LOG_TARGET, "⚠ No involved shards for payload");
        }

        {
            let mut access = self.transactions.lock().unwrap();
            // TODO: O(n)
            if access.iter().any(|(tx, _)| tx.hash() == transaction.hash()) {
                info!(
                    target: LOG_TARGET,
                    "🎱 Transaction {} already in mempool",
                    transaction.hash()
                );
                return Err(MempoolError::TransactionAlreadyExists);
            }

            let current_node_pubkey = self.node_identity.public_key().clone();
            let mut should_process_txn = false;

            for sid in &shards {
                if self
                    .epoch_manager
                    .is_validator_in_committee_for_current_epoch(*sid, current_node_pubkey.clone())
                    .await?
                {
                    should_process_txn = true;
                    break;
                }
            }

            if should_process_txn {
                access.push((transaction.clone(), None));
            } else {
                return Err(MempoolError::TransactionNotProcessedByCurrentVN);
            }
        }
        info!(target: LOG_TARGET, "🎱 New transaction in mempool");

        self.propagate_transaction(&transaction, &shards).await;

        for shard_id in shards {
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
        // TODO: unwrap !
        let epoch = self.epoch_manager.current_epoch().await?;
        let committees = self.epoch_manager.get_committees(epoch, shards).await?;

        let msg = DanMessage::NewTransaction(transaction.clone());

        // propagate over the involved shard ids
        let committees_set = HashSet::<RistrettoPublicKey>::from_iter(committees.into_iter().flat_map(|x| {
            x.committee
                .expect("mempool_service::propagate_transaction::shard committee should be available")
                .members
        }));
        let committees = committees_set.into_iter().collect::<Vec<_>>();

        if let Err(err) = self
            .outbound
            .broadcast(self.node_identity.public_key().clone(), &committees, msg)
            .await
        {
            error!(target: LOG_TARGET, "Failed to broadcast new transaction: {}", err);
        }

        Ok(())
    }

    pub fn get_transaction(&self) -> TransactionVecMutex {
        self.transactions.clone()
    }
}
