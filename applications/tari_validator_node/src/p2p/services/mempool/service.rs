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

use std::{collections::HashSet, fmt::Display, iter, sync::Arc};

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use log::*;
use tari_comms::NodeIdentity;
use tari_dan_app_utilities::transaction_executor::{TransactionExecutor, TransactionProcessorError};
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_engine::runtime::ConsensusContext;
use tari_dan_p2p::{DanMessage, OutboundService};
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, TransactionRecord},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{mpsc, oneshot};

use super::MempoolError;
use crate::{
    p2p::services::{
        mempool::{
            executor::{execute_transaction, ExecutionResult},
            handle::MempoolRequest,
            traits::SubstateResolver,
            Validator,
        },
        messaging::OutboundMessaging,
    },
    substate_resolver::SubstateResolverError,
};

const LOG_TARGET: &str = "tari::validator_node::mempool::service";

#[derive(Debug)]
pub struct MempoolService<TValidator, TExecutor, TSubstateResolver> {
    transactions: HashSet<TransactionId>,
    pending_executions: FuturesUnordered<BoxFuture<'static, Result<ExecutionResult, MempoolError>>>,
    new_transactions: mpsc::Receiver<Transaction>,
    mempool_requests: mpsc::Receiver<MempoolRequest>,
    outbound: OutboundMessaging,
    tx_executed_transactions: mpsc::Sender<ExecutedTransaction>,
    epoch_manager: EpochManagerHandle,
    node_identity: Arc<NodeIdentity>,
    validator: TValidator,
    transaction_executor: TExecutor,
    substate_resolver: TSubstateResolver,
    state_store: SqliteStateStore,
}

impl<TValidator, TExecutor, TSubstateResolver> MempoolService<TValidator, TExecutor, TSubstateResolver>
where
    TValidator: Validator<Transaction, Error = MempoolError>,
    TExecutor: TransactionExecutor<Error = TransactionProcessorError> + Clone + Send + Sync + 'static,
    TSubstateResolver: SubstateResolver<Error = SubstateResolverError> + Clone + Send + Sync + 'static,
{
    pub(super) fn new(
        new_transactions: mpsc::Receiver<Transaction>,
        mempool_requests: mpsc::Receiver<MempoolRequest>,
        outbound: OutboundMessaging,
        tx_executed_transactions: mpsc::Sender<ExecutedTransaction>,
        epoch_manager: EpochManagerHandle,
        node_identity: Arc<NodeIdentity>,
        transaction_executor: TExecutor,
        substate_resolver: TSubstateResolver,
        validator: TValidator,
        state_store: SqliteStateStore,
    ) -> Self {
        Self {
            transactions: Default::default(),
            pending_executions: FuturesUnordered::new(),
            new_transactions,
            mempool_requests,
            outbound,
            tx_executed_transactions,
            epoch_manager,
            node_identity,
            transaction_executor,
            substate_resolver,
            validator,
            state_store,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                Some(req) = self.mempool_requests.recv() => self.handle_request(req).await,
                Some(result) = self.pending_executions.next() => {
                    if  let Err(e) = self.handle_execution_complete(result).await {
                        error!(target: LOG_TARGET, "Possible bug: handle_execution_complete failed: {}", e);
                    }
                },
                Some(tx) = self.new_transactions.recv() => {
                    if let Err(e) = self.handle_new_transaction(tx).await {
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
            MempoolRequest::RemoveTransaction {
                transaction_id: transaction_hash,
            } => self.remove_transaction(&transaction_hash),
            MempoolRequest::GetMempoolSize { reply } => {
                let _ignore = reply.send(self.transactions.len());
            },
        }
    }

    fn remove_transaction(&mut self, id: &TransactionId) {
        self.transactions.remove(id);
    }

    async fn handle_new_transaction(&mut self, transaction: Transaction) -> Result<(), MempoolError> {
        debug!(
            target: LOG_TARGET,
            "Received NEW transaction: {} {:?}",
            transaction.id(),
            transaction
        );

        if self.transactions.contains(transaction.id()) {
            info!(
                target: LOG_TARGET,
                "🎱 Transaction {} already in mempool",
                transaction.id()
            );
            return Ok(());
        }

        let transaction_exists = self
            .state_store
            .with_read_tx(|tx| TransactionRecord::exists(tx, transaction.id()))?;

        if transaction_exists {
            info!(
                target: LOG_TARGET,
                "🎱 Transaction {} already exists. Ignoring",
                transaction.id()
            );
            return Ok(());
        }

        self.validator.validate(&transaction).await?;

        self.state_store
            .with_write_tx(|tx| tx.transactions_insert(&transaction))?;

        if transaction.num_involved_shards() == 0 {
            warn!(target: LOG_TARGET, "⚠ No involved shards for payload");
        }

        let current_epoch = self.epoch_manager.current_epoch().await?;
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(current_epoch).await?;
        let shards = transaction.involved_shards_iter().copied()
            // Involved shard also includes the transaction hash shard. TODO: hacky, this helps with edge cases where there are no inputs/outputs other than the transaction hash
            .chain(iter::once(transaction.id().into_array().into()))
            .collect();

        if local_committee_shard.includes_any_shard(&shards) {
            info!(target: LOG_TARGET, "🎱 New transaction in mempool");
            self.transactions.insert(*transaction.id());
            let current_epoch = self.epoch_manager.current_epoch().await?;
            self.queue_transaction_for_execution(transaction, current_epoch);
        } else {
            info!(
                target: LOG_TARGET,
                "🙇 Not in committee for transaction {}",
                transaction.id()
            );

            if let Err(e) = self.propagate_transaction(transaction, &shards).await {
                error!(
                    target: LOG_TARGET,
                    "Unable to propagate transaction among peers: {}",
                    e.to_string()
                );
            }
        }

        Ok(())
    }

    fn queue_transaction_for_execution(&mut self, transaction: Transaction, current_epoch: Epoch) {
        let substate_resolver = self.substate_resolver.clone();
        let executor = self.transaction_executor.clone();
        let consensus_context = ConsensusContext {
            current_epoch: current_epoch.as_u64(),
        };

        self.pending_executions
            .push(execute_transaction(transaction, substate_resolver, executor, consensus_context).boxed());
    }

    async fn handle_execution_complete(
        &mut self,
        result: Result<ExecutionResult, MempoolError>,
    ) -> Result<(), MempoolError> {
        // This is due to a bug or possibly db failure only
        let (transaction_id, exec_result) = result?;

        self.transactions.remove(&transaction_id);
        let executed = match exec_result {
            Ok(mut executed) => {
                info!(
                    target: LOG_TARGET,
                    "✅ Transaction {} executed successfully ({}) in {:?}",
                    executed.transaction().id(),
                    executed.result().finalize.result,
                    executed.execution_time()
                );
                // We refuse to process the transaction if any input_refs are downed
                self.check_input_refs(&executed)?;
                // Fill the outputs that were missing so that we can propagate to output shards
                self.fill_outputs(&mut executed);

                executed
            },
            Err(e) => {
                error!(
                    target: LOG_TARGET,
                    "❌ Transaction {} failed: {}",
                    transaction_id,
                    e.to_string()
                );

                return Ok(());
            },
        };

        let shards = executed.transaction().involved_shards_iter().copied().collect();

        if let Err(e) = self
            .propagate_transaction(executed.transaction().clone(), &shards)
            .await
        {
            error!(
                target: LOG_TARGET,
                "Unable to propagate transaction among peers: {}",
                e.to_string()
            )
        }

        if self.tx_executed_transactions.send(executed).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "Executed transaction channel closed before executed transaction could be sent"
            );
        }

        Ok(())
    }

    fn check_input_refs(&self, executed: &ExecutedTransaction) -> Result<(), MempoolError> {
        let Some(diff) = executed.result().finalize.result.accept().cloned() else {
            return Ok(());
        };

        let is_input_refs_downed = diff
            .down_iter()
            .map(|(s, v)| ShardId::from_address(s, *v))
            .any(|s| executed.transaction().input_refs().contains(&s));

        if is_input_refs_downed {
            return Err(MempoolError::InputRefsDowned);
        }
        Ok(())
    }

    fn fill_outputs(&mut self, executed: &mut ExecutedTransaction) {
        let Some(diff) = executed.result().finalize.result.accept().cloned() else {
            return;
        };

        let outputs = executed.transaction().outputs();
        let filled_outputs = diff
            .up_iter()
            .map(|(addr, substate)| ShardId::from_address(addr, substate.version()))
            .filter(|shard_id| outputs.contains(shard_id))
            // NOTE: we must collect here so that we can mutate
            .collect::<Vec<_>>();
        executed.transaction_mut().filled_outputs_mut().extend(filled_outputs);
    }

    async fn propagate_transaction(
        &mut self,
        transaction: Transaction,
        shards: &HashSet<ShardId>,
    ) -> Result<(), MempoolError> {
        let epoch = self.epoch_manager.current_epoch().await?;
        let committees = self.epoch_manager.get_committees_by_shards(epoch, shards).await?;

        let msg = DanMessage::NewTransaction(Box::new(transaction));

        // propagate over the involved shard ids
        #[allow(clippy::mutable_key_type)]
        let unique_members = committees
            .into_iter()
            .flat_map(|(_, s)| s.members)
            .filter(|pk| pk != self.node_identity.public_key())
            .collect::<HashSet<_>>();
        let committees = unique_members.into_iter().collect::<Vec<_>>();

        self.outbound.broadcast(&committees, msg).await?;

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
