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

use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{mpsc, oneshot};

use crate::p2p::services::mempool::MempoolError;

pub enum MempoolRequest {
    SubmitTransaction {
        transaction: Box<Transaction>,
        /// If true, the transaction will be propagated to peers
        should_propagate: bool,
        reply: oneshot::Sender<Result<(), MempoolError>>,
    },
    RemoveTransaction {
        transaction_id: TransactionId,
    },
    GetMempoolSize {
        reply: oneshot::Sender<usize>,
    },
}

#[derive(Debug)]
pub struct MempoolHandle {
    tx_mempool_request: mpsc::Sender<MempoolRequest>,
}

impl Clone for MempoolHandle {
    fn clone(&self) -> Self {
        MempoolHandle {
            tx_mempool_request: self.tx_mempool_request.clone(),
        }
    }
}

impl MempoolHandle {
    pub(super) fn new(tx_mempool_request: mpsc::Sender<MempoolRequest>) -> Self {
        Self { tx_mempool_request }
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<(), MempoolError> {
        let (reply, rx) = oneshot::channel();
        self.tx_mempool_request
            .send(MempoolRequest::SubmitTransaction {
                transaction: Box::new(transaction),
                should_propagate: true,
                reply,
            })
            .await?;
        rx.await?
    }

    pub async fn submit_transaction_without_propagating(&self, transaction: Transaction) -> Result<(), MempoolError> {
        let (reply, rx) = oneshot::channel();
        self.tx_mempool_request
            .send(MempoolRequest::SubmitTransaction {
                transaction: Box::new(transaction),
                should_propagate: false,
                reply,
            })
            .await?;
        rx.await?
    }

    pub async fn remove_transaction(&self, transaction_id: TransactionId) -> Result<(), MempoolError> {
        self.tx_mempool_request
            .send(MempoolRequest::RemoveTransaction { transaction_id })
            .await?;
        Ok(())
    }

    pub async fn get_mempool_size(&self) -> Result<usize, MempoolError> {
        let (tx, rx) = oneshot::channel();
        self.tx_mempool_request
            .send(MempoolRequest::GetMempoolSize { reply: tx })
            .await?;
        rx.await.map_err(Into::into)
    }
}
