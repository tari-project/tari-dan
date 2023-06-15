//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use futures::{stream::FuturesUnordered, StreamExt};
use log::warn;
use tari_dan_storage::consensus_models::{ExecutedTransaction, Transaction, TransactionId};
use tokio::{sync::mpsc, task, task::JoinHandle};

use crate::transaction_executor::traits::TransactionExecutable;

const LOG_TARGET: &str = "tari::dan::consensus::transaction_executor";

type TransactionExecutorResult<E> = (TransactionId, Result<ExecutedTransaction, E>);

pub struct TransactionExecutor<TExecutable: TransactionExecutable> {
    executable: TExecutable,
    pending_transaction: FuturesUnordered<JoinHandle<TransactionExecutorResult<TExecutable::Error>>>,
    rx_new_transactions: mpsc::Receiver<Transaction>,
    tx_executed_transaction: mpsc::Sender<ExecutedTransaction>,
}

impl<TExecutable> TransactionExecutor<TExecutable>
where
    TExecutable: TransactionExecutable + Clone + Send + Sync + 'static,
    TExecutable::Error: Display,
{
    pub fn new(
        executable: TExecutable,
        rx_new_transactions: mpsc::Receiver<Transaction>,
        tx_executed_transaction: mpsc::Sender<ExecutedTransaction>,
    ) -> Self {
        Self {
            executable,
            pending_transaction: FuturesUnordered::new(),
            rx_new_transactions,
            tx_executed_transaction,
        }
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                Some(transaction) = self.rx_new_transactions.recv() => {
                    let executable = self.executable.clone();
                    let tx_hash = *transaction.hash();
                    self.pending_transaction.push(task::spawn_blocking(move || {
                        let result = executable.execute(transaction);
                        (
                            tx_hash,
                            result,
                        )
                    }));
                },
                Some(Ok((tx_id, result))) = self.pending_transaction.next() => {
                    self.on_execution_result(tx_id, result).await;

                },
                else => break,
            }
        }
    }

    async fn on_execution_result(
        &self,
        transaction_id: TransactionId,
        result: Result<ExecutedTransaction, TExecutable::Error>,
    ) {
        match result {
            Ok(executed) => {
                if self.tx_executed_transaction.send(executed).await.is_err() {
                    warn!(target: LOG_TARGET, "tx_executed_transaction is closed.");
                }
            },
            // Error results only happens if there is some kind of system failure or a malformed transaction. We discard
            // in these cases.
            Err(err) => {
                warn!(target: LOG_TARGET, "Transaction execution failed: {}. Discarding transaction {}", err, transaction_id);
            },
        }
    }
}
