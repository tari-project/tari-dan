//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{
        family::Family,
        histogram::{Histogram, exponential_buckets},
    },
    registry::Registry,
};
use tari_engine_types::commit_result::TransactionResult;

use crate::metrics::CollectorRegister;

#[derive(Debug, Clone, PartialEq, Eq, Hash, EncodeLabelSet)]
struct ExecutionOutcomeLabels {
    outcome: &'static str,
}

/// Clones share the series, so every clone of the block executor reports to the same histogram.
#[derive(Debug, Clone)]
pub struct PrometheusExecutionMetrics {
    execution_seconds: Family<ExecutionOutcomeLabels, Histogram, fn() -> Histogram>,
}

impl PrometheusExecutionMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let registry = registry.sub_registry_with_prefix("execution");
        Self {
            // Most transactions execute in well under a millisecond, so buckets start at 10µs to
            // resolve that mode. 17 buckets reach ~0.65s; anything slower lands in +Inf.
            execution_seconds: Family::new_with_constructor(
                (|| Histogram::new(exponential_buckets(0.000_01, 2.0, 17))) as fn() -> Histogram,
            )
            .register_at(
                "transaction_seconds",
                "Time the engine spent executing one transaction, in seconds, by outcome",
                registry,
            ),
        }
    }

    pub fn on_executed(&self, result: &TransactionResult, elapsed: Duration) {
        let outcome = match result {
            TransactionResult::Accept(_) => "accept",
            TransactionResult::AcceptFeeRejectRest(_, _) => "fee_only",
            TransactionResult::Reject(_) => "reject",
        };
        self.execution_seconds
            .get_or_create(&ExecutionOutcomeLabels { outcome })
            .observe(elapsed.as_secs_f64());
    }
}
