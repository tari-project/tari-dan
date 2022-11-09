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

use std::sync::{Arc, Mutex};

use log::*;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    message::DanMessage,
    models::{HotStuffMessage, Payload, TariDanPayload},
    services::infrastructure_services::OutboundService,
};
use tari_dan_engine::transaction::Transaction;
use tokio::sync::{broadcast, mpsc};

use super::handle::TransactionVecMutex;
use crate::p2p::services::{mempool::handle::MempoolRequest, messaging::OutboundMessaging};

const LOG_TARGET: &str = "dan::mempool::service";

pub struct MempoolService {
    // TODO: Should be a HashSet
    transactions: TransactionVecMutex,
    new_transactions: mpsc::Receiver<Transaction>,
    outbound: OutboundMessaging,
    tx_valid_transactions: broadcast::Sender<(Transaction, ShardId)>,
    rx_consensus_message:
        broadcast::Receiver<(RistrettoPublicKey, HotStuffMessage<TariDanPayload, RistrettoPublicKey>)>,
}

impl MempoolService {
    pub(super) fn new(
        new_transactions: mpsc::Receiver<Transaction>,
        outbound: OutboundMessaging,
        tx_valid_transactions: broadcast::Sender<(Transaction, ShardId)>,
        rx_consensus_message: broadcast::Receiver<(
            RistrettoPublicKey,
            HotStuffMessage<TariDanPayload, RistrettoPublicKey>,
        )>,
    ) -> Self {
        Self {
            transactions: Arc::new(Mutex::new(Vec::new())),
            new_transactions,
            outbound,
            tx_valid_transactions,
            rx_consensus_message,
        }
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                Some(transaction) = self.new_transactions.recv() => {
                    self.handle_new_transaction(transaction).await;
                }

                Ok((_, msg)) = self.rx_consensus_message.recv() => {
                    // we want to remove this transaction from mempool if message has a node and the payload height is 4
                    let node = if let Some(node) = msg.node() {
                        node
                    } else {
                        // message can't be finalized at this stage
                        continue
                    };

                    if node.payload_height().as_u64() >= 4u64 {
                        let transaction = if let Some(payload) = node.payload() {
                            payload.transaction()
                        } else {
                            continue
                        };
                        // at this point the transaction should have been committed,
                        // so we can safely assume it is finalized
                        self.remove_finalized_transaction(transaction)
                    }
                }

                else => {
                    info!(target: LOG_TARGET, "Mempool service shutting down");
                    break;
                }
            }
        }
    }

    async fn handle_request(&mut self, request: MempoolRequest) {
        match request {
            MempoolRequest::SubmitTransaction(transaction) => self.handle_new_transaction(transaction).await,
            MempoolRequest::RemoveTransaction { hash } => self.remove_transaction(hash),
        }
    }

    fn remove_transaction(&mut self, hash: Vec<u8>) {
        let mut transactions = self.transactions.lock().unwrap();
        transactions.retain(|(transaction, _)| transaction.hash() != hash);
    }

    async fn handle_new_transaction(&mut self, transaction: Transaction) {
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
                return;
            }

            access.push((transaction.clone(), None));
        }
        info!(target: LOG_TARGET, "🎱 New transaction in mempool");

        // TODO: Should just propagate to shards involved
        let msg = DanMessage::NewTransaction(transaction.clone());
        if let Err(err) = self.outbound.flood(Default::default(), msg).await {
            error!(target: LOG_TARGET, "Failed to broadcast new transaction: {}", err);
        }

        for shard_id in payload.involved_shards() {
            if let Err(err) = self.tx_valid_transactions.send((transaction.clone(), shard_id)) {
                error!(
                    target: LOG_TARGET,
                    "Failed to send valid transaction to shard: {}: {}", shard_id, err
                );
            }
        }
    }

    pub fn remove_finalized_transaction(&mut self, transaction: &Transaction) {
        let mut access = self.transactions.lock().unwrap();
        access.retain(|(tx, _)| tx.hash() != transaction.hash());
    }

    pub fn get_transaction(&self) -> TransactionVecMutex {
        self.transactions.clone()
    }
}
