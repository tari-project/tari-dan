//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use prometheus::{IntCounter, Registry};
use tari_transaction::{Transaction, TransactionId};

use crate::metrics::CollectorRegister;

#[derive(Debug, Clone)]
pub struct PrometheusMempoolMetrics {
    transactions_received: IntCounter,
    transaction_validation_error: IntCounter,
}

impl PrometheusMempoolMetrics {
    pub fn new(registry: &Registry) -> Self {
        Self {
            transactions_received: IntCounter::new("mempool_transactions_received", "Number of transactions received")
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

    pub fn on_transaction_received(&mut self, _transaction: &Transaction) {
        self.transactions_received.inc();
    }

    pub fn on_transaction_validation_error<E: ToString>(&mut self, _transaction: &TransactionId, _err: &E) {
        self.transaction_validation_error.inc();
    }
}
