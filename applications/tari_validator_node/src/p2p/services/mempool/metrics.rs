//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use prometheus::{Histogram, HistogramOpts, IntCounter, Registry};
use tari_dan_storage::consensus_models::{ExecutedTransaction, TransactionRecord};
use tari_transaction::TransactionId;

use crate::{metrics::CollectorRegister, p2p::services::mempool::MempoolError};

#[derive(Debug, Clone)]
pub struct PrometheusMempoolMetrics {
    transactions_received: IntCounter,
    transactions_executed: IntCounter,
    transactions_execute_time: Histogram,

    transaction_execute_error: IntCounter,
    transaction_validation_error: IntCounter,
}

impl PrometheusMempoolMetrics {
    pub fn new(registry: &Registry) -> Self {
        Self {
            transactions_received: IntCounter::new("mempool_transactions_received", "Number of transactions received")
                .unwrap()
                .register_at(registry),
            transactions_executed: IntCounter::new("mempool_transactions_executed", "Number of transactions executed")
                .unwrap()
                .register_at(registry),
            transactions_execute_time: Histogram::with_opts(HistogramOpts::new(
                "mempool_transactions_execute_time",
                "Time to execute a transaction",
            ))
            .unwrap()
            .register_at(registry),
            transaction_execute_error: IntCounter::new(
                "mempool_transaction_execute_error",
                "Number of transaction execution errors",
            )
            .unwrap()
            .register_at(registry),
            transaction_validation_error: IntCounter::new(
                "mempool_transaction_validation_error",
                "Number of transaction validation errors",
            )
            .unwrap()
            .register_at(registry),
        }
    }

    pub fn on_transaction_received(&mut self, _transaction: &TransactionRecord) {
        self.transactions_received.inc();
    }

    pub fn on_transaction_executed(
        &mut self,
        _transaction_id: &TransactionId,
        execution_result: &Result<ExecutedTransaction, MempoolError>,
    ) {
        self.transactions_executed.inc();
        match execution_result {
            Ok(transaction) => {
                self.transactions_execute_time
                    .observe(transaction.execution_time().as_millis() as f64);
            },
            Err(_) => {
                self.transaction_execute_error.inc();
            },
        }
    }

    pub fn on_transaction_validation_error<E: ToString>(&mut self, _transaction: &TransactionId, _err: &E) {
        self.transaction_validation_error.inc();
    }
}
