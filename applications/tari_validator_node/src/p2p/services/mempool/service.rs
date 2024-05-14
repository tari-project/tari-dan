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

use std::{collections::HashSet, fmt::Display, iter, ops::DerefMut};

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use log::*;
use tari_dan_app_utilities::transaction_executor::{TransactionExecutor, TransactionProcessorError};
use tari_dan_common_types::{optional::Optional, shard::Shard, Epoch, PeerAddress, SubstateAddress};
use tari_dan_p2p::{DanMessage, NewTransactionMessage};
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, SubstateRecord, TransactionRecord},
    StateStore,
};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerEvent, EpochManagerReader};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{mpsc, oneshot};

#[cfg(feature = "metrics")]
use super::metrics::PrometheusMempoolMetrics;
use super::MempoolError;
use crate::{
    consensus::ConsensusHandle,
    p2p::services::{
        mempool::{
            executor::execute_transaction,
            gossip::MempoolGossip,
            handle::MempoolRequest,
            traits::SubstateResolver,
            Validator,
        },
        messaging::Gossip,
    },
    substate_resolver::SubstateResolverError,
};

const LOG_TARGET: &str = "tari::validator_node::mempool::service";

/// Data returned from a pending execution.
struct MempoolTransactionExecution {
    transaction_id: TransactionId,
    execution: TransactionExecution,
    should_propagate: bool,
    sender_shard: Option<Shard>,
}

pub enum TransactionExecution {
    /// The transaction was executed in the mempool
    Executed {
        result: Result<ExecutedTransaction, MempoolError>,
    },
    /// Mempool execution failed due to an error that is unrelated to the transaction. IO, resources, database etc
    ExecutionFailure {
        error: MempoolError,
        transaction: Transaction,
    },
    /// Execution cannot occur in the mempool and is deferred to consensus
    Deferred { transaction: Transaction },
}

#[derive(Debug)]
pub struct MempoolService<TValidator, TExecutedValidator, TExecutor, TSubstateResolver> {
    transactions: HashSet<TransactionId>,
    pending_executions: FuturesUnordered<BoxFuture<'static, MempoolTransactionExecution>>,
    mempool_requests: mpsc::Receiver<MempoolRequest>,
    tx_executed_transactions: mpsc::Sender<(TransactionId, usize)>,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    before_execute_validator: TValidator,
    after_execute_validator: TExecutedValidator,
    transaction_executor: TExecutor,
    substate_resolver: TSubstateResolver,
    state_store: SqliteStateStore<PeerAddress>,
    gossip: MempoolGossip<PeerAddress>,
    rx_consensus_to_mempool: mpsc::UnboundedReceiver<Transaction>,
    consensus_handle: ConsensusHandle,
    #[cfg(feature = "metrics")]
    metrics: PrometheusMempoolMetrics,
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
        mempool_requests: mpsc::Receiver<MempoolRequest>,
        gossip: Gossip,
        tx_executed_transactions: mpsc::Sender<(TransactionId, usize)>,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        transaction_executor: TExecutor,
        substate_resolver: TSubstateResolver,
        before_execute_validator: TValidator,
        after_execute_validator: TExecutedValidator,
        state_store: SqliteStateStore<PeerAddress>,
        rx_consensus_to_mempool: mpsc::UnboundedReceiver<Transaction>,
        consensus_handle: ConsensusHandle,
        #[cfg(feature = "metrics")] metrics: PrometheusMempoolMetrics,
    ) -> Self {
        Self {
            gossip: MempoolGossip::new(epoch_manager.clone(), gossip),
            transactions: Default::default(),
            pending_executions: FuturesUnordered::new(),
            mempool_requests,
            tx_executed_transactions,
            epoch_manager,
            transaction_executor,
            substate_resolver,
            before_execute_validator,
            after_execute_validator,
            state_store,
            rx_consensus_to_mempool,
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
                Some(result) = self.pending_executions.next() => {
                    if  let Err(e) = self.handle_execution_task_complete(result).await {
                        error!(target: LOG_TARGET, "Possible bug: handle_execution_complete failed: {}", e);
                    }
                },
                Some(result) = self.gossip.next_message() => {
                    if let Err(e) = self.handle_new_transaction_from_remote(result).await {
                        warn!(target: LOG_TARGET, "Mempool rejected transaction: {}", e);
                    }
                }
                Some(msg) = self.rx_consensus_to_mempool.recv() => {
                    if let Err(e) = self.handle_new_transaction_from_local(msg, false).await {
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
        result: Result<(PeerAddress, DanMessage), MempoolError>,
    ) -> Result<(), MempoolError> {
        let (from, msg) = result?;
        let DanMessage::NewTransaction(msg) = msg;
        let NewTransactionMessage {
            transaction,
            output_shards: unverified_output_shards,
        } = *msg;

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
        let maybe_sender_shard = self
            .epoch_manager
            .get_committee_info_by_validator_address(current_epoch, &from)
            .await
            .optional()?.map(|c| c.shard());

        // Only input shards propagate transactions to output shards. Check that this is true.
        if !unverified_output_shards.is_empty() {
            let Some(sender_shard) = maybe_sender_shard else {
                debug!(target: LOG_TARGET, "Sender {from} isn't registered but tried to send a new transaction with
        output shards");
                return Ok(());
            };

            let mut is_input_shard = transaction
                .all_inputs_iter()
                .filter_map(|s| s.to_committee_shard(num_committees))
                .any(|s| s == sender_shard);
            // Special temporary case: if there are no input shards an output shard will also propagate. No inputs is
            // invalid, however we must support them for now because of CreateFreeTestCoin transactions.
            is_input_shard |= transaction.inputs().is_empty() && transaction.filled_inputs().is_empty();
            if !is_input_shard {
                warn!(target: LOG_TARGET, "Sender {from} sent a message with output shards but was not an input
        shard. Ignoring message.");         return Ok(());
            }
        }

        self.handle_new_transaction(transaction, unverified_output_shards, true, maybe_sender_shard)
            .await?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_new_transaction(
        &mut self,
        transaction: Transaction,
        unverified_output_shards: Vec<SubstateAddress>,
        should_propagate: bool,
        sender_shard: Option<Shard>,
    ) -> Result<(), MempoolError> {
        #[cfg(feature = "metrics")]
        self.metrics.on_transaction_received(&transaction);

        if let Err(e) = self.before_execute_validator.validate(&transaction).await {
            let transaction_id = *transaction.id();
            self.state_store.with_write_tx(|tx| {
                TransactionRecord::new(transaction)
                    .set_abort(format!("Mempool validation failed: {e}"))
                    .insert(tx)
            })?;

            #[cfg(feature = "metrics")]
            self.metrics.on_transaction_validation_error(&transaction_id, &e);
            return Err(e);
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

        if transaction.num_unique_inputs() == 0 && claim_shards.is_empty() && unverified_output_shards.is_empty() {
            warn!(target: LOG_TARGET, "⚠ No involved shards for payload");
        }

        let current_epoch = self.epoch_manager.current_epoch().await?;
        let tx_substate_address = SubstateAddress::for_transaction_receipt(transaction.id().into_receipt_address());

        let local_committee_shard = self.epoch_manager.get_local_committee_info(current_epoch).await?;
        let transaction_inputs = transaction.all_inputs_iter().filter_map(|i| i.to_substate_address());
        let mut is_input_shard = local_committee_shard.includes_any_shard(transaction_inputs);
        // Special temporary case: if there are no input shards an output shard will also propagate. No inputs is
        // invalid, however we must support them for now because of CreateFreeTestCoin transactions.
        is_input_shard |= transaction.inputs().is_empty() && transaction.filled_inputs().is_empty();
        let is_output_shard = local_committee_shard.includes_any_shard(
            // Known output shards
            // This is to allow for the txreceipt output
            iter::once(&tx_substate_address)
                .chain(unverified_output_shards.iter())
                .chain(claim_shards.iter()),
        );

        if is_input_shard || is_output_shard {
            debug!(target: LOG_TARGET, "🎱 New transaction {} in mempool", transaction.id());
            let transaction = TransactionRecord::new(transaction);
            self.state_store.with_write_tx(|tx| transaction.insert(tx))?;
            let transaction = transaction.into_transaction();
            self.transactions.insert(*transaction.id());

            self.queue_transaction_for_execution(transaction.clone(), current_epoch, should_propagate, sender_shard);

            if should_propagate {
                // This validator is involved, we to send the transaction to local replicas
                if let Err(e) = self
                    .gossip
                    .forward_to_local_replicas(
                        current_epoch,
                        NewTransactionMessage {
                            transaction: transaction.clone(),
                            output_shards: unverified_output_shards, /* Or send to local only when we are input shard
                                                                      * and if we are output send after execution */
                        }
                        .into(),
                    )
                    .await
                {
                    warn!(
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
                    let substate_addresses = transaction
                        .all_inputs_iter()
                        .map(|i| i.or_zero_version().to_substate_address())
                        .collect();
                    if let Err(e) = self
                        .gossip
                        .forward_to_foreign_replicas(
                            current_epoch,
                            substate_addresses,
                            NewTransactionMessage {
                                transaction,
                                output_shards: vec![],
                            },
                            sender_shard,
                        )
                        .await
                    {
                        warn!(
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
                let substate_addresses = transaction.involved_shards_iter().collect();
                if let Err(e) = self
                    .gossip
                    .gossip_to_foreign_replicas(current_epoch, substate_addresses, NewTransactionMessage {
                        transaction,
                        output_shards: vec![],
                    })
                    .await
                {
                    warn!(
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
        sender_shard: Option<Shard>,
    ) {
        let substate_resolver = self.substate_resolver.clone();
        let executor = self.transaction_executor.clone();
        let transaction_id = *transaction.id();

        self.pending_executions.push(Box::pin(
            execute_transaction(transaction.clone(), substate_resolver, executor, current_epoch).map(move |result| {
                match result {
                    Ok(execution_result) => MempoolTransactionExecution {
                        transaction_id,
                        execution: TransactionExecution::Executed {
                            result: execution_result,
                        },
                        should_propagate,
                        sender_shard,
                    },
                    Err(MempoolError::MustDeferExecution { .. }) => MempoolTransactionExecution {
                        transaction_id,
                        execution: TransactionExecution::Deferred { transaction },
                        should_propagate,
                        sender_shard,
                    },
                    Err(error) => MempoolTransactionExecution {
                        transaction_id,
                        // IO, Database, etc errors
                        execution: TransactionExecution::ExecutionFailure { error, transaction },
                        should_propagate,
                        sender_shard,
                    },
                }
            }),
        ));
    }

    async fn handle_execution_task_complete(
        &mut self,
        result: MempoolTransactionExecution,
    ) -> Result<(), MempoolError> {
        let MempoolTransactionExecution {
            transaction_id,
            execution,
            should_propagate,
            sender_shard,
        } = result;

        match execution {
            TransactionExecution::Executed { result } => {
                self.handle_execution_complete(transaction_id, result, should_propagate, sender_shard)
                    .await
            },
            // Bubble the error up
            TransactionExecution::ExecutionFailure { error, .. } => {
                // TODO: should we retry this transaction at some point?
                self.transactions.remove(&transaction_id);
                Err(error)
            },
            TransactionExecution::Deferred { transaction } => self.handle_deferred_execution(transaction).await,
        }
    }

    async fn handle_deferred_execution(&mut self, transaction: Transaction) -> Result<(), MempoolError> {
        let transaction_id = *transaction.id();

        let is_consensus_running = self.consensus_handle.get_current_state().is_running();

        let pending_exec_size = self.pending_executions.len();
        if is_consensus_running &&
            // Notify consensus about the transaction
            self.tx_executed_transactions
                .send((transaction_id, pending_exec_size))
                .await
                .is_err()
        {
            debug!(
                target: LOG_TARGET,
                "Executed transaction channel closed before executed transaction could be sent"
            );
        }

        self.transactions.remove(&transaction_id);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_execution_complete(
        &mut self,
        transaction_id: TransactionId,
        exec_result: Result<ExecutedTransaction, MempoolError>,
        should_propagate: bool,
        sender_shard: Option<Shard>,
    ) -> Result<(), MempoolError> {
        #[cfg(feature = "metrics")]
        self.metrics.on_transaction_executed(&transaction_id, &exec_result);

        // The avoids the case where:
        // 1. A transaction is received and start executing
        // 2. The node switches to sync mode
        // 3. Sync completes (some transactions that were finalized in sync may have been busy executing)
        // 4. Execution completes and the transaction is added to the pool even though it is finalized via sync
        // TODO: This is not guaranteed to work and is subject to races. The mempool should pause processing executed
        // transactions until consensus is in sync.
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
                    "✅ Transaction {} executed ({}) in {:?}",
                    executed.id(),
                    executed.result().finalize.result,
                    executed.execution_time()
                );
                let has_involved_shards = executed.num_inputs_and_outputs() > 0;

                match self.after_execute_validator.validate(&executed).await {
                    Ok(_) => {
                        // Add the transaction result and push it into the pool for consensus. This is done in a single
                        // transaction so that if we receive a proposal for this transaction, we
                        // either are awaiting execution OR execution is complete and it's in the pool.
                        self.state_store.with_write_tx(|tx| {
                            if !has_involved_shards {
                                match executed.result().finalize.result.full_reject() {
                                    Some(reason) => {
                                        executed
                                            .set_abort(format!("Transaction failed: {}", reason))
                                            .update(tx)?;
                                    },
                                    None => {
                                        executed
                                            .set_abort("Mempool after execution validation failed: No involved shards")
                                            .update(tx)?;
                                    },
                                }

                                return Ok::<_, MempoolError>(());
                            }

                            executed.update(tx)?;
                            Ok::<_, MempoolError>(())
                        })?;
                    },
                    Err(e) => {
                        info!(
                            target: LOG_TARGET,
                            "❌ Executed transaction {} failed validation: {}",
                            executed.id(),
                            e,
                        );
                        #[cfg(feature = "metrics")]
                        self.metrics.on_transaction_validation_error(&transaction_id, &e);
                        self.state_store.with_write_tx(|tx| {
                            match executed.result().finalize.result.full_reject() {
                                Some(reason) => {
                                    executed
                                        .set_abort(format!("Transaction failed: {}", reason))
                                        .update(tx)?;
                                },
                                None => {
                                    executed
                                        .set_abort(format!("Mempool after execution validation failed: {}", e))
                                        .update(tx)?;
                                },
                            }

                            Ok::<_, MempoolError>(())
                        })?;
                        // We want this to go though to consensus, because validation may only fail in this shard (e.g
                        // outputs already exist) so we need to send LocalPrepared(ABORT) to
                        // other shards.
                    },
                }

                // TODO: This transaction executed but no shard is involved even after execution
                //        (happens for CreateFreeTestCoin only) so we just ignore it.
                if !has_involved_shards {
                    warn!(
                        target: LOG_TARGET,
                        "Transaction {} has no involved shards after executing. Ignoring",
                        transaction_id
                    );
                    self.transactions.remove(&transaction_id);
                    return Ok(());
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

                self.transactions.remove(&transaction_id);

                return Ok(());
            },
        };

        let current_epoch = self.epoch_manager.current_epoch().await?;

        let local_committee_shard = self.epoch_manager.get_local_committee_info(current_epoch).await?;
        let all_inputs_iter = executed.all_inputs_iter().map(|i| i.to_substate_address());
        let is_input_shard = local_committee_shard.includes_any_shard(all_inputs_iter) |
            (executed.transaction().inputs().is_empty() && executed.transaction().filled_inputs().is_empty());

        if should_propagate && is_input_shard {
            // Forward the transaction to any output shards that are not part of the input shard set as these have
            // already been forwarded
            let num_committees = self.epoch_manager.get_num_committees(current_epoch).await?;
            let input_shards = executed
                .resolved_inputs()
                .into_iter()
                .flatten()
                .map(|s| s.versioned_substate_id().to_committee_shard(num_committees))
                .collect::<HashSet<_>>();
            let tx_substate_address = SubstateAddress::for_transaction_receipt(executed.id().into_receipt_address());
            let output_shards = executed
                .resulting_outputs()
                .iter()
                // All involved shards commit the transaction receipt, so we exclude the shard @ tx_substate_address from propagation and consensus.
                .map(|s| s.to_substate_address())
                .filter(|s| *s != tx_substate_address)
                .filter(|s| !input_shards.contains(&s.to_committee_shard(num_committees)))
                .collect();

            if let Err(err) = self
                .gossip
                .forward_to_foreign_replicas(
                    current_epoch,
                    output_shards,
                    NewTransactionMessage {
                        transaction: executed.transaction().clone(),
                        output_shards: executed
                            .resulting_outputs()
                            .iter()
                            .map(|s| s.to_substate_address())
                            .collect::<Vec<_>>(),
                    },
                    sender_shard,
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
        let pending_exec_size = self.pending_executions.len();
        if is_consensus_running &&
            self.tx_executed_transactions
                .send((*executed.id(), pending_exec_size))
                .await
                .is_err()
        {
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
