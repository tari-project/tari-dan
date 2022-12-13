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

use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use log::*;
use tari_comms::NodeIdentity;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    message::DanMessage,
    models::{Payload, TariDanPayload},
    services::{epoch_manager::EpochManager, infrastructure_services::OutboundService},
};
use tari_dan_engine::transaction::Transaction;
use tari_engine_types::instruction::Instruction;
use tari_template_lib::Hash;
use tokio::sync::{broadcast, mpsc};

use super::{handle::TransactionPool, MempoolError};
use crate::p2p::services::{
    epoch_manager::handle::EpochManagerHandle,
    mempool::{handle::MempoolRequest, Validator},
    messaging::OutboundMessaging,
    template_manager::{TemplateManager, TemplateManagerError},
};

const LOG_TARGET: &str = "dan::mempool::service";

pub struct MempoolService {
    transactions: TransactionPool,
    new_transactions: mpsc::Receiver<Transaction>,
    mempool_requests: mpsc::Receiver<MempoolRequest>,
    outbound: OutboundMessaging,
    tx_valid_transactions: broadcast::Sender<(Transaction, ShardId)>,
    epoch_manager: EpochManagerHandle,
    node_identity: Arc<NodeIdentity>,
    validator: MempoolTransactionValidator,
}

pub struct MempoolTransactionValidator {
    template_manager: TemplateManager,
}

impl MempoolTransactionValidator {
    pub(crate) fn new(template_manager: TemplateManager) -> Self {
        Self { template_manager }
    }
}
#[async_trait]
impl Validator<Transaction> for MempoolTransactionValidator {
    type Error = MempoolError;

    async fn validate(&self, inner: &Transaction) -> Result<(), MempoolError> {
        let instructions = inner.instructions();
        for instruction in instructions {
            match instruction {
                Instruction::CallFunction { template_address, .. } => {
                    let template_exists = self.template_manager.template_exists(template_address);
                    match template_exists {
                        Err(e) => return Err(MempoolError::InvalidTemplateAddress(e)),
                        Ok(false) => {
                            return Err(MempoolError::InvalidTemplateAddress(
                                TemplateManagerError::TemplateNotFound {
                                    address: *template_address,
                                },
                            ))
                        },
                        _ => continue,
                    }
                },
                _ => continue,
            }
        }

        Ok(())
    }
}

impl MempoolService {
    pub(super) fn new(
        new_transactions: mpsc::Receiver<Transaction>,
        mempool_requests: mpsc::Receiver<MempoolRequest>,
        outbound: OutboundMessaging,
        tx_valid_transactions: broadcast::Sender<(Transaction, ShardId)>,
        epoch_manager: EpochManagerHandle,
        node_identity: Arc<NodeIdentity>,
        validator: MempoolTransactionValidator,
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
        }
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                Some(req) = self.mempool_requests.recv() => self.handle_request(req).await,
                Some(tx) = self.new_transactions.recv() => self.handle_new_transaction(tx).await,

                else => {
                    info!(target: LOG_TARGET, "Mempool service shutting down");
                    break;
                }
            }
        }
    }

    async fn handle_request(&mut self, request: MempoolRequest) {
        match request {
            MempoolRequest::SubmitTransaction(transaction) => self.handle_new_transaction(*transaction).await,
            MempoolRequest::RemoveTransaction { transaction_hash } => self.remove_transaction(&transaction_hash),
        }
    }

    fn remove_transaction(&mut self, hash: &Hash) {
        let mut transactions = self.transactions.lock().unwrap();
        transactions.remove(hash);
    }

    async fn handle_new_transaction(&mut self, transaction: Transaction) {
        debug!(target: LOG_TARGET, "Received new transaction: {:?}", transaction);

        if let Err(e) = self.validator.validate(&transaction).await {
            error!(
                target: LOG_TARGET,
                "⚠ Invalid templates found for transaction: {}",
                e.to_string(),
            );
            return;
        }

        let payload = TariDanPayload::new(transaction.clone());

        let shards = payload.involved_shards();
        debug!(target: LOG_TARGET, "New Payload in mempool for shards: {:?}", shards);
        if shards.is_empty() {
            warn!(target: LOG_TARGET, "⚠ No involved shards for payload");
        }

        {
            let access = self.transactions.lock().unwrap();
            if access.contains_key(transaction.hash()) {
                info!(
                    target: LOG_TARGET,
                    "🎱 Transaction {} already in mempool",
                    transaction.hash()
                );
                return;
            }
        }

        {
            let current_node_pubkey = self.node_identity.public_key().clone();
            let mut should_process_txn = false;

            for sid in &shards {
                match self
                    .epoch_manager
                    .is_validator_in_committee_for_current_epoch(*sid, current_node_pubkey.clone())
                    .await
                {
                    Ok(b) => {
                        if b {
                            should_process_txn = true;
                            break;
                        }
                    },
                    Err(e) => error!(
                        target: LOG_TARGET,
                        "Failed to retrieve validator in the committee for current epoch: {}",
                        e.to_string(),
                    ),
                }
            }

            let mut access = self.transactions.lock().unwrap();

            if should_process_txn {
                info!(target: LOG_TARGET, "🎱 New transaction in mempool");
                access.insert(*transaction.hash(), (transaction.clone(), None));
            } else {
                info!(
                    target: LOG_TARGET,
                    "🙇 Not in committee for transaction {}",
                    transaction.hash()
                );
            }
        }

        match self.propagate_transaction(&transaction, &shards).await {
            Ok(()) => (),
            Err(e) => error!(
                target: LOG_TARGET,
                "Unable to propagate transaction among peers: {}",
                e.to_string()
            ),
        }

        for shard_id in shards {
            if let Err(err) = self.tx_valid_transactions.send((transaction.clone(), shard_id)) {
                error!(
                    target: LOG_TARGET,
                    "Failed to send valid transaction to shard: {}: {}", shard_id, err
                );
            }
        }
    }

    pub async fn propagate_transaction(
        &mut self,
        transaction: &Transaction,
        shards: &[ShardId],
    ) -> Result<(), MempoolError> {
        let epoch = self
            .epoch_manager
            .current_epoch()
            .await
            .map_err(|e| MempoolError::EpochManagerError(Box::new(e)))?;
        let committees = self
            .epoch_manager
            .get_committees(epoch, shards)
            .await
            .map_err(|e| MempoolError::EpochManagerError(Box::new(e)))?;

        let msg = DanMessage::NewTransaction(transaction.clone());

        // propagate over the involved shard ids
        #[allow(clippy::mutable_key_type)]
        let unique_members = committees
            .into_iter()
            .flat_map(|s| s.committee.members)
            .collect::<HashSet<_>>();
        let committees = unique_members.into_iter().collect::<Vec<_>>();

        self.outbound
            .broadcast(self.node_identity.public_key().clone(), &committees, msg)
            .await?;

        Ok(())
    }

    pub fn get_transaction(&self) -> TransactionPool {
        self.transactions.clone()
    }
}
