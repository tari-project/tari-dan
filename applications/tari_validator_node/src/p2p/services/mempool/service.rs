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

use std::{collections::HashSet, fmt::Display, iter, ops::DerefMut, sync::Arc};

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use log::*;
use tari_common_types::types::PublicKey;
use tari_comms::{types::CommsPublicKey, NodeIdentity};
use tari_dan_app_utilities::transaction_executor::{TransactionExecutor, TransactionProcessorError};
use tari_dan_common_types::{optional::Optional, shard_bucket::ShardBucket, Epoch, ShardId};
use tari_dan_p2p::NewTransactionMessage;
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, SubstateRecord, TransactionPool, TransactionRecord},
    StateStore,
};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{mpsc, oneshot};

use super::MempoolError;
use crate::{
    consensus::ConsensusHandle,
    p2p::services::{
        mempool::{
            executor::{execute_transaction, ExecutionResult},
            gossip::Gossip,
            handle::MempoolRequest,
            traits::SubstateResolver,
            Validator,
        },
        messaging::OutboundMessaging,
    },
    substate_resolver::SubstateResolverError,
};

const LOG_TARGET: &str = "tari::validator_node::mempool::service";

/// Data returned from a pending execution.
struct MempoolTransactionExecution {
    result: Result<ExecutionResult, MempoolError>,
    should_propagate: bool,
    sender_bucket: Option<ShardBucket>,
}

#[derive(Debug)]
pub struct MempoolService<TValidator, TExecutedValidator, TExecutor, TSubstateResolver> {
    transactions: HashSet<TransactionId>,
    pending_executions: FuturesUnordered<BoxFuture<'static, MempoolTransactionExecution>>,
    new_transactions: mpsc::Receiver<(CommsPublicKey, NewTransactionMessage)>,
    mempool_requests: mpsc::Receiver<MempoolRequest>,
    tx_executed_transactions: mpsc::Sender<TransactionId>,
    epoch_manager: EpochManagerHandle,
    before_execute_validator: TValidator,
    after_execute_validator: TExecutedValidator,
    transaction_executor: TExecutor,
    substate_resolver: TSubstateResolver,
    state_store: SqliteStateStore<PublicKey>,
    transaction_pool: TransactionPool<SqliteStateStore<PublicKey>>,
    gossip: Gossip,
    rx_consensus_to_mempool: mpsc::UnboundedReceiver<Transaction>,
    consensus_handle: ConsensusHandle,
}

impl<TValidator, TExecutedValidator, TExecutor, TSubstateResolver>
    MempoolService<TValidator, TExecutedValidator, TExecutor, TSubstateResolver>
where
    TValidator: Validator<Transaction, Error = MempoolError>,
    TExecutedValidator: Validator<ExecutedTransaction, Error = MempoolError>,
    TExecutor: TransactionExecutor<Error = TransactionProcessorError> + Clone + Send + Sync + 'static,
    TSubstateResolver: SubstateResolver<Error = SubstateResolverError> + Clone + Send + Sync + 'static,
{
    pub(super) fn new(
        new_transactions: mpsc::Receiver<(CommsPublicKey, NewTransactionMessage)>,
        mempool_requests: mpsc::Receiver<MempoolRequest>,
        outbound: OutboundMessaging,
        tx_executed_transactions: mpsc::Sender<TransactionId>,
        epoch_manager: EpochManagerHandle,
        node_identity: Arc<NodeIdentity>,
        transaction_executor: TExecutor,
        substate_resolver: TSubstateResolver,
        before_execute_validator: TValidator,
        after_execute_validator: TExecutedValidator,
        state_store: SqliteStateStore<PublicKey>,
        rx_consensus_to_mempool: mpsc::UnboundedReceiver<Transaction>,
        consensus_handle: ConsensusHandle,
    ) -> Self {
        Self {
            gossip: Gossip::new(epoch_manager.clone(), outbound, node_identity.public_key().clone()),
            transactions: Default::default(),
            pending_executions: FuturesUnordered::new(),
            new_transactions,
            mempool_requests,
            tx_executed_transactions,
            epoch_manager,
            transaction_executor,
            substate_resolver,
            before_execute_validator,
            after_execute_validator,
            state_store,
            transaction_pool: TransactionPool::new(),
            rx_consensus_to_mempool,
            consensus_handle,
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
                Some((from, msg)) = self.new_transactions.recv() => {
                    if let Err(e) = self.handle_new_transaction_from_remote(from, msg).await {
                        warn!(target: LOG_TARGET, "Mempool rejected transaction: {}", e);
                    }
                }
                Some(msg) = self.rx_consensus_to_mempool.recv() => {
                    if let Err(e) = self.handle_new_transaction_from_local(msg, false).await {
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
            MempoolRequest::SubmitTransaction {
                transaction,
                should_propagate,
                reply,
            } => {
                handle(
                    reply,
                    self.handle_new_transaction_from_local(*transaction, should_propagate)
                        .await,
                );
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

    async fn handle_new_transaction_from_local(
        &mut self,
        transaction: Transaction,
        should_propagate: bool,
    ) -> Result<(), MempoolError> {
        if self.transaction_exists(transaction.id())? {
            return Ok(());
        }
        debug!(
            target: LOG_TARGET,
            "Received NEW transaction from local: {} {:?}",
            transaction.id(),
            transaction
        );

        self.handle_new_transaction(transaction, vec![], should_propagate, None)
            .await?;

        Ok(())
    }

    async fn handle_new_transaction_from_remote(
        &mut self,
        from: CommsPublicKey,
        msg: NewTransactionMessage,
    ) -> Result<(), MempoolError> {
        let NewTransactionMessage {
            transaction,
            output_shards: unverified_output_shards,
        } = msg;

        if !self.consensus_handle.get_current_state().is_running() {
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

        let current_epoch = self.epoch_manager.current_epoch().await?;
        let num_committees = self.epoch_manager.get_num_committees(current_epoch).await?;
        let maybe_sender_bucket = self
            .epoch_manager
            .get_validator_node(current_epoch, &from)
            .await
            .optional()?
            .and_then(|s| s.committee_bucket);

        // Only input shards propagate transactions to output shards. Check that this is true.
        if !unverified_output_shards.is_empty() {
            let Some(sender_bucket) = maybe_sender_bucket else {
                debug!(target: LOG_TARGET, "Sender {from} isn't registered but tried to send a new transaction with output shards");
                return Ok(());
            };
            let mut is_input_shard = transaction
                .all_inputs_iter()
                .any(|s| s.to_committee_bucket(num_committees) == sender_bucket);
            // Special temporary case: if there are no input shards an output shard will also propagate. No inputs is
            // invalid, however we must support them for now because of CreateFreeTestCoin transactions.
            is_input_shard |= transaction.inputs().len() + transaction.input_refs().len() == 0;
            if !is_input_shard {
                warn!(target: LOG_TARGET, "Sender {from} sent a message with output shards but was not an input shard. Ignoring message.");
                return Ok(());
            }
        }

        self.handle_new_transaction(transaction, unverified_output_shards, true, maybe_sender_bucket)
            .await?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_new_transaction(
        &mut self,
        transaction: Transaction,
        unverified_output_shards: Vec<ShardId>,
        should_propagate: bool,
        sender_bucket: Option<ShardBucket>,
    ) -> Result<(), MempoolError> {
        let mut transaction = TransactionRecord::new(transaction);
        self.state_store.with_write_tx(|tx| transaction.insert(tx))?;

        if let Err(e) = self.before_execute_validator.validate(transaction.transaction()).await {
            self.state_store.with_write_tx(|tx| {
                transaction
                    .set_abort(format!("Mempool validation failed: {}", e))
                    .update(tx)
            })?;
            return Err(e);
        }

        let transaction = transaction.into_transaction();

        // Get the shards involved in claim fees.
        let fee_claims = transaction.fee_claims().collect::<Vec<_>>();

        let claim_shards = if fee_claims.is_empty() {
            HashSet::new()
        } else {
            #[allow(clippy::mutable_key_type)]
            let validator_nodes = self.epoch_manager.get_many_validator_nodes(fee_claims).await?;
            validator_nodes.values().map(|vn| vn.shard_key).collect::<HashSet<_>>()
        };

        if transaction.num_involved_shards() == 0 && claim_shards.is_empty() && unverified_output_shards.is_empty() {
            warn!(target: LOG_TARGET, "⚠ No involved shards for payload");
        }

        let tx_shard_id = ShardId::from(transaction.id().into_array());

        let current_epoch = self.epoch_manager.current_epoch().await?;
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(current_epoch).await?;

        let mut is_input_shard = local_committee_shard.includes_any_shard(transaction.all_inputs_iter());
        // Special temporary case: if there are no input shards an output shard will also propagate. No inputs is
        // invalid, however we must support them for now because of CreateFreeTestCoin transactions.
        is_input_shard |= transaction.inputs().len() + transaction.input_refs().len() == 0;
        let is_output_shard = local_committee_shard.includes_any_shard(
            // Known output shards
            // This is to allow for the txreceipt output
            iter::once(&tx_shard_id)
                .chain(transaction.outputs())
                .chain(unverified_output_shards.iter())
                .chain(claim_shards.iter()),
        );

        if is_input_shard || is_output_shard {
            debug!(target: LOG_TARGET, "🎱 New transaction {} in mempool", transaction.id());
            self.transactions.insert(*transaction.id());
            self.queue_transaction_for_execution(transaction.clone(), current_epoch, should_propagate, sender_bucket);

            if should_propagate {
                // This validator is involved, we to send the transaction to local replicas
                if let Err(e) = self
                    .gossip
                    .forward_to_local_replicas(
                        current_epoch,
                        NewTransactionMessage {
                            transaction: transaction.clone(),
                            output_shards: vec![],
                        }
                        .into(),
                    )
                    .await
                {
                    error!(
                        target: LOG_TARGET,
                        "Unable to propagate transaction among peers: {}",
                        e
                    );
                }

                // Only input shards propagate to foreign shards
                if is_input_shard {
                    // Forward to foreign replicas.
                    // We assume that at least f other local replicas receive this transaction and also forward to their
                    // matching replica(s)
                    if let Err(e) = self
                        .gossip
                        .forward_to_foreign_replicas(
                            current_epoch,
                            transaction.involved_shards_iter().copied().collect(),
                            NewTransactionMessage {
                                transaction,
                                output_shards: vec![],
                            }
                            .into(),
                            sender_bucket,
                        )
                        .await
                    {
                        error!(
                            target: LOG_TARGET,
                            "Unable to propagate transaction among peers: {}",
                            e
                        );
                    }
                }
            }
        } else {
            debug!(
                target: LOG_TARGET,
                "🙇 Not in committee for transaction {}",
                transaction.id()
            );

            if should_propagate {
                // This validator is not involved, so we forward the transaction to f + 1 replicas per distinct shard
                // per input shard ID because we may be the only validator that has received this transaction.
                if let Err(e) = self
                    .gossip
                    .gossip_to_foreign_replicas(
                        current_epoch,
                        transaction.involved_shards_iter().copied().collect(),
                        NewTransactionMessage {
                            transaction,
                            output_shards: vec![],
                        }
                        .into(),
                    )
                    .await
                {
                    error!(
                        target: LOG_TARGET,
                        "Unable to propagate transaction among peers: {}",
                        e
                    );
                }
            }
        }

        Ok(())
    }

    fn queue_transaction_for_execution(
        &mut self,
        transaction: Transaction,
        current_epoch: Epoch,
        should_propagate: bool,
        sender_bucket: Option<ShardBucket>,
    ) {
        let substate_resolver = self.substate_resolver.clone();
        let executor = self.transaction_executor.clone();

        self.pending_executions.push(Box::pin(
            execute_transaction(transaction, substate_resolver, executor, current_epoch).map(move |result| {
                MempoolTransactionExecution {
                    result,
                    should_propagate,
                    sender_bucket,
                }
            }),
        ));
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_execution_complete(&mut self, result: MempoolTransactionExecution) -> Result<(), MempoolError> {
        let MempoolTransactionExecution {
            result,
            should_propagate,
            sender_bucket,
        } = result;
        // This is due to a bug or possibly db failure only
        let (transaction_id, exec_result) = result?;

        // The avoids the case where:
        // 1. A transaction is received and start executing
        // 2. The node switches to sync mode
        // 3. Sync completes (some transactions that were finalized in sync may have been busy executing)
        // 4. Execution completes and the transaction is added to the pool even though it is finalized via sync
        if self
            .state_store
            .with_read_tx(|tx| SubstateRecord::exists_for_transaction(tx, &transaction_id))?
        {
            debug!(
                target: LOG_TARGET,
                "🎱 Transaction {} already processed. Ignoring",
                transaction_id
            );
            return Ok(());
        }

        let is_consensus_running = self.consensus_handle.get_current_state().is_running();

        let executed = match exec_result {
            Ok(mut executed) => {
                info!(
                    target: LOG_TARGET,
                    "✅ Transaction {} executed successfully ({}) in {:?}",
                    executed.id(),
                    executed.result().finalize.result,
                    executed.execution_time()
                );
                match self.after_execute_validator.validate(&executed).await {
                    Ok(_) => {
                        // Add the transaction result and push it into the pool for consensus. This is done in a single
                        // transaction so that if we receive a proposal for this transaction, we
                        // either are awaiting execution OR execution is complete and it's in the pool.
                        self.state_store.with_write_tx(|tx| {
                            executed.update(tx)?;
                            if is_consensus_running &&
                                !SubstateRecord::exists_for_transaction(tx.deref_mut(), &transaction_id)?
                            {
                                self.transaction_pool.insert(tx, executed.to_atom())?;
                            }
                            Ok::<_, MempoolError>(())
                        })?;
                    },
                    Err(e) => {
                        self.state_store.with_write_tx(|tx| {
                            executed
                                .set_abort(format!("Mempool after execution validation failed: {}", e))
                                .update(tx)?;
                            if is_consensus_running &&
                                !SubstateRecord::exists_for_transaction(tx.deref_mut(), &transaction_id)?
                            {
                                self.transaction_pool.insert(tx, executed.to_atom())?;
                            }
                            Ok::<_, MempoolError>(())
                        })?;
                        // We want this to go though to consensus, because validation may only fail in this shard (e.g
                        // outputs already exist) so we need to send LocalPrepared(ABORT) to
                        // other shards.
                    },
                }

                executed
            },
            Err(e) => {
                error!(
                    target: LOG_TARGET,
                    "❌ Transaction {} failed: {}",
                    transaction_id,
                    e
                );
                self.state_store.with_write_tx(|tx| {
                    TransactionRecord::get(tx.deref_mut(), &transaction_id)?
                        .set_abort(format!("Mempool failed to execute: {}", e))
                        .update(tx)
                })?;

                return Ok(());
            },
        };

        let current_epoch = self.epoch_manager.current_epoch().await?;
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(current_epoch).await?;
        let is_input_shard = local_committee_shard.includes_any_shard(executed.transaction().all_inputs_iter());

        if should_propagate && is_input_shard {
            // Forward the transaction to any output shards that are not part of the input shard set as these have
            // already been forwarded
            let num_committees = self.epoch_manager.get_num_committees(current_epoch).await?;
            let input_buckets = executed
                .involved_shards_iter()
                .map(|s| s.to_committee_bucket(num_committees))
                .collect::<HashSet<_>>();
            let output_shards = executed
                .resulting_outputs()
                .iter()
                .filter(|s| !input_buckets.contains(&s.to_committee_bucket(num_committees)))
                .copied()
                .collect();

            if let Err(err) = self
                .gossip
                .forward_to_foreign_replicas(
                    current_epoch,
                    output_shards,
                    NewTransactionMessage {
                        transaction: executed.transaction().clone(),
                        output_shards: executed.resulting_outputs().to_vec(),
                    }
                    .into(),
                    sender_bucket,
                )
                .await
            {
                error!(
                    target: LOG_TARGET,
                    "Unable to propagate transaction among peers: {}", err
                );
            }
        }

        // Notify consensus that a transaction is ready to go!
        if is_consensus_running && self.tx_executed_transactions.send(*executed.id()).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "Executed transaction channel closed before executed transaction could be sent"
            );
        }

        self.transactions.remove(&transaction_id);
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
