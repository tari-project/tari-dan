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

use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use log::*;
use tari_common_types::types::PublicKey;
use tari_comms::NodeIdentity;
use tari_dan_app_utilities::transaction_executor::{TransactionExecutor, TransactionProcessorError};
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_p2p::{DanMessage, NewTransactionMessage, OutboundService};
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, TransactionPool, TransactionRecord},
    StateStore,
};
use tari_engine_types::instruction::Instruction;
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
pub struct MempoolService<TValidator, TExecutedValidator, TExecutor, TSubstateResolver> {
    transactions: HashSet<TransactionId>,
    pending_executions: FuturesUnordered<BoxFuture<'static, Result<ExecutionResult, MempoolError>>>,
    new_transactions: mpsc::Receiver<NewTransactionMessage>,
    mempool_requests: mpsc::Receiver<MempoolRequest>,
    outbound: OutboundMessaging,
    tx_executed_transactions: mpsc::Sender<TransactionId>,
    epoch_manager: EpochManagerHandle,
    node_identity: Arc<NodeIdentity>,
    before_execute_validator: TValidator,
    after_execute_validator: TExecutedValidator,
    transaction_executor: TExecutor,
    substate_resolver: TSubstateResolver,
    state_store: SqliteStateStore<PublicKey>,
    transaction_pool: TransactionPool<SqliteStateStore<PublicKey>>,
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
        new_transactions: mpsc::Receiver<NewTransactionMessage>,
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
            before_execute_validator,
            after_execute_validator,
            state_store,
            transaction_pool: TransactionPool::new(),
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
                Some(msg) = self.new_transactions.recv() => {
                    if let Err(e) = self.handle_new_transaction(msg.transaction, msg.output_shards, true).await {
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
                    self.handle_new_transaction(*transaction, vec![], should_propagate)
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

    #[allow(clippy::too_many_lines)]
    async fn handle_new_transaction(
        &mut self,
        transaction: Transaction,
        unverified_output_shards: Vec<ShardId>,
        should_propagate: bool,
    ) -> Result<(), MempoolError> {
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
        // TODO: I can't move this into a function because I run into an issue where Sync is not implemented for the
        //       Mempool struct, because execute_transaction is not sync. I think this comes down to async_trait that
        //       cannot/does not enforce the Sync trait on async trait methods.
        let claim_instructions = transaction
            .instructions()
            .iter()
            .chain(transaction.fee_instructions())
            .filter_map(|instruction| {
                if let Instruction::ClaimValidatorFees {
                    epoch,
                    validator_public_key,
                } = instruction
                {
                    Some((Epoch(*epoch), validator_public_key.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let claim_shards = if claim_instructions.is_empty() {
            HashSet::new()
        } else {
            #[allow(clippy::mutable_key_type)]
            let validator_nodes = self.epoch_manager.get_many_validator_nodes(claim_instructions).await?;
            validator_nodes.values().map(|vn| vn.shard_key).collect::<HashSet<_>>()
        };

        if transaction.num_involved_shards() == 0 && claim_shards.is_empty() && unverified_output_shards.is_empty() {
            warn!(target: LOG_TARGET, "⚠ No involved shards for payload");
        }

        let current_epoch = self.epoch_manager.current_epoch().await?;
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(current_epoch).await?;
        // Involved shard also includes the transaction hash shard.
        let involved_shards = transaction
            .involved_shards_iter()
            .copied()
            // This is to allow for the txreceipt that gets created 
            .chain(iter::once(transaction.id().into_array().into()))
            // TODO: this is not verified, and could cause validators to waste resources. We perhaps need to check that the sender is a registered validator node from an input shard if we are an output shard to limit the scope of this attack.
            .chain(unverified_output_shards)
            .chain(claim_shards)
            .collect();

        if local_committee_shard.includes_any_shard(&involved_shards) {
            info!(target: LOG_TARGET, "🎱 New transaction in mempool");
            self.transactions.insert(*transaction.id());
            self.queue_transaction_for_execution(transaction, current_epoch);
        } else {
            info!(
                target: LOG_TARGET,
                "🙇 Not in committee for transaction {}",
                transaction.id()
            );

            if should_propagate {
                if let Err(e) = self
                    .propagate_transaction(
                        NewTransactionMessage {
                            transaction,
                            output_shards: vec![],
                        },
                        &involved_shards,
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

    fn queue_transaction_for_execution(&mut self, transaction: Transaction, current_epoch: Epoch) {
        let substate_resolver = self.substate_resolver.clone();
        let executor = self.transaction_executor.clone();

        self.pending_executions.push(Box::pin(execute_transaction(
            transaction,
            substate_resolver,
            executor,
            current_epoch,
        )));
    }

    async fn handle_execution_complete(
        &mut self,
        result: Result<ExecutionResult, MempoolError>,
    ) -> Result<(), MempoolError> {
        // This is due to a bug or possibly db failure only
        let (transaction_id, exec_result) = result?;

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
                            self.transaction_pool.insert(tx, executed.to_atom())
                        })?;
                    },
                    Err(e) => {
                        self.state_store.with_write_tx(|tx| {
                            executed
                                .set_abort(format!("Mempool after execution validation failed: {}", e))
                                .update(tx)?;
                            self.transaction_pool.insert(tx, executed.to_atom())
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
                    let mut transaction = TransactionRecord::get(tx.deref_mut(), &transaction_id)?;
                    transaction
                        .set_abort(format!("Mempool failed to execute: {}", e))
                        .update(tx)
                })?;

                return Ok(());
            },
        };

        let shards = executed
            .transaction()
            .all_inputs_iter()
            .chain(executed.resulting_outputs())
            .copied()
            .collect();

        if let Err(e) = self
            .propagate_transaction(
                NewTransactionMessage {
                    transaction: executed.transaction().clone(),
                    output_shards: executed.resulting_outputs().to_vec(),
                },
                &shards,
            )
            .await
        {
            error!(
                target: LOG_TARGET,
                "Unable to propagate transaction among peers: {}",
                e.to_string()
            )
        }

        // Notify consensus that a transaction is ready to go!
        if self.tx_executed_transactions.send(*executed.id()).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "Executed transaction channel closed before executed transaction could be sent"
            );
        }

        self.transactions.remove(&transaction_id);
        Ok(())
    }

    async fn propagate_transaction(
        &mut self,
        msg: NewTransactionMessage,
        shards: &HashSet<ShardId>,
    ) -> Result<(), MempoolError> {
        let epoch = self.epoch_manager.current_epoch().await?;
        let committees = self.epoch_manager.get_committees_by_shards(epoch, shards).await?;

        debug!(
            target: LOG_TARGET,
            "Propagating transaction {} to {} members {} shards",
            msg.transaction.id(),
            committees.values().flat_map(|c|&c.members).count(),
            shards.len()
        );

        let dan_msg = DanMessage::NewTransaction(Box::new(msg));

        // propagate over the involved shard ids
        #[allow(clippy::mutable_key_type)]
        let unique_members = committees
            .into_iter()
            .flat_map(|(_, s)| s.members)
            .filter(|pk| pk != self.node_identity.public_key())
            .collect::<HashSet<_>>();
        let committees = unique_members.into_iter().collect::<Vec<_>>();

        self.outbound.broadcast(&committees, dan_msg).await?;

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
