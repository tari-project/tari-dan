//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::sync::Arc;

use async_trait::async_trait;
use tari_dan_common_types::ShardId;
use tari_dan_engine::instruction::Transaction;
use tokio::sync::{
    broadcast,
    broadcast::{channel, Receiver, Sender},
    Mutex,
};

use super::outbound::MempoolOutboundService;
use crate::{
    digital_assets_error::DigitalAssetError,
    models::{Payload, TariDanPayload, TreeNodeHash},
};

#[async_trait]
pub trait MempoolService: Sync + Send + 'static {
    async fn submit_transaction(&mut self, transaction: &Transaction) -> Result<(), DigitalAssetError>;
    async fn size(&self) -> usize;
}

pub struct ConcreteMempoolService {
    tx_new: Sender<(TariDanPayload, ShardId)>,
    transactions: Vec<(Transaction, Option<TreeNodeHash>)>,
    outbound_service: Option<Box<dyn MempoolOutboundService>>,
}

impl ConcreteMempoolService {
    pub fn new(tx_new: Sender<(TariDanPayload, ShardId)>) -> Self {
        Self {
            tx_new,
            transactions: vec![],
            outbound_service: None,
        }
    }
}

#[async_trait]
impl MempoolService for ConcreteMempoolService {
    async fn submit_transaction(&mut self, transaction: &Transaction) -> Result<(), DigitalAssetError> {
        // TODO: validate the transaction
        self.transactions.push((transaction.clone(), None));

        if let Some(outbound_service) = &mut self.outbound_service {
            outbound_service.propagate_transaction(transaction.clone()).await?;
        }

        let payload = TariDanPayload::new(transaction.clone());
        for shard in &payload.involved_shards() {
            self.tx_new
                .send((payload.clone(), *shard))
                .map_err(|_| DigitalAssetError::SendError {
                    context: "Sending from mempool".to_string(),
                })?;
        }

        Ok(())
    }

    // async fn remove_instructions(&mut self, instructions: &[Instruction]) -> Result<(), DigitalAssetError> {
    //     let mut result = self.instructions.clone();
    //     for i in instructions {
    //         if let Some(position) = result.iter().position(|r| r == i) {
    //             result.remove(position);
    //         }
    //     }
    //     self.instructions = result;
    //     Ok(())
    // }

    async fn size(&self) -> usize {
        self.transactions
            .iter()
            .fold(0, |a, b| if b.1.is_none() { a + 1 } else { a })
    }
}

pub struct MempoolServiceHandle {
    mempool: Arc<Mutex<ConcreteMempoolService>>,
    rx_new: Receiver<(TariDanPayload, ShardId)>,
}

impl Clone for MempoolServiceHandle {
    fn clone(&self) -> Self {
        Self {
            mempool: self.mempool.clone(),
            rx_new: self.rx_new.resubscribe(),
        }
    }
}

impl MempoolServiceHandle {
    pub fn new() -> Self {
        let (tx_new, rx_new) = channel(1);
        let mempool_service = ConcreteMempoolService::new(tx_new);

        Self {
            mempool: Arc::new(Mutex::new(mempool_service)),
            rx_new,
        }
    }

    pub async fn set_outbound_service(&mut self, outbound_service: Box<dyn MempoolOutboundService>) {
        self.mempool.lock().await.outbound_service = Some(outbound_service);
    }

    pub fn new_payload(&mut self) -> &mut broadcast::Receiver<(TariDanPayload, ShardId)> {
        &mut self.rx_new
    }
}

impl Default for MempoolServiceHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MempoolService for MempoolServiceHandle {
    async fn submit_transaction(&mut self, transaction: &Transaction) -> Result<(), DigitalAssetError> {
        self.mempool.lock().await.submit_transaction(transaction).await
    }

    async fn size(&self) -> usize {
        self.mempool.lock().await.size().await
    }
}
