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

use tari_dan_common_types::ShardId;
use tari_dan_core::models::TreeNodeHash;
use tari_dan_engine::transaction::Transaction;
use tari_template_lib::Hash;
use tokio::sync::{broadcast, broadcast::error::RecvError, mpsc, mpsc::error::SendError};

pub type TransactionVecMutex = Arc<Mutex<Vec<(Transaction, Option<TreeNodeHash>)>>>;

pub enum MempoolRequest {
    SubmitTransaction(Box<Transaction>),
    RemoveTransaction { transaction_hash: Hash },
}

#[derive(Debug)]
pub struct MempoolHandle {
    rx_valid_transactions: broadcast::Receiver<(Transaction, ShardId)>,
    tx_mempool_request: mpsc::Sender<MempoolRequest>,
    transactions: TransactionVecMutex,
}

impl Clone for MempoolHandle {
    fn clone(&self) -> Self {
        MempoolHandle {
            rx_valid_transactions: self.rx_valid_transactions.resubscribe(),
            tx_mempool_request: self.tx_mempool_request.clone(),
            transactions: self.transactions.clone(),
        }
    }
}

impl MempoolHandle {
    pub(super) fn new(
        rx_valid_transactions: broadcast::Receiver<(Transaction, ShardId)>,
        tx_mempool_request: mpsc::Sender<MempoolRequest>,
        transactions: TransactionVecMutex,
    ) -> Self {
        Self {
            rx_valid_transactions,
            tx_mempool_request,
            transactions,
        }
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<(), SendError<MempoolRequest>> {
        self.tx_mempool_request
            .send(MempoolRequest::SubmitTransaction(Box::new(transaction)))
            .await
    }

    pub async fn remove_transaction(&self, transaction_hash: Hash) -> Result<(), SendError<MempoolRequest>> {
        self.tx_mempool_request
            .send(MempoolRequest::RemoveTransaction { transaction_hash })
            .await
    }

    pub async fn next_valid_transaction(&mut self) -> Result<(Transaction, ShardId), RecvError> {
        self.rx_valid_transactions.recv().await
    }

    pub fn get_mempool_size(&self) -> usize {
        self.transactions.lock().unwrap().len()
    }
}
