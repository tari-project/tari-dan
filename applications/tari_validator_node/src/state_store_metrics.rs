//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Metrics for the memory the state store holds.
//!
//! The store opens RocksDB with one block cache shared by every column family, and charges memtable
//! memory to that same cache, so a single capacity bounds both. [`StateStoreMemoryCollector`] reads
//! the live figures off that budget on each scrape rather than maintaining background gauges, in the
//! same shape as [`crate::inbound_queue_metrics::InboundQueueCollector`].
//!
//! Usage against capacity is what turns the node's derived memory ceiling into a claim that can be
//! checked continuously: usage that sits far below capacity means the budget is larger than the
//! workload needs, and usage pinned at capacity means reads are being served from disk that the
//! budget was meant to keep in memory.

use std::fmt;

use prometheus_client::{
    collector::Collector,
    encoding::{DescriptorEncoder, EncodeMetric},
    metrics::gauge::ConstGauge,
    registry::Registry,
};
use tari_state_store_rocksdb::RocksDbMemoryBudget;

/// Reports the state store's block cache capacity and what is currently charged against it.
pub struct StateStoreMemoryCollector {
    budget: RocksDbMemoryBudget,
}

impl StateStoreMemoryCollector {
    pub fn new(budget: RocksDbMemoryBudget) -> Self {
        Self { budget }
    }

    /// Registers under the `state_store` sub-registry, so the metric names on the wire are
    /// `state_store_memory_capacity_bytes`, `state_store_memtable_bytes`, and so on.
    pub fn register(self, registry: &mut Registry) {
        let registry = registry.sub_registry_with_prefix("state_store");
        registry.register_collector(Box::new(self));
    }
}

impl Collector for StateStoreMemoryCollector {
    fn encode(&self, mut encoder: DescriptorEncoder) -> Result<(), fmt::Error> {
        // Units are carried in the metric names rather than passed to the encoder, which would
        // append them a second time. This matches `InboundQueueCollector`.
        let readings: [(&str, &str, u64); 5] = [
            (
                "memory_capacity_bytes",
                "Bytes the shared block cache admits, memtable charges included",
                self.budget.capacity_bytes() as u64,
            ),
            (
                "memtable_capacity_bytes",
                "Bytes memtables may occupy across all column families before flushes are triggered",
                self.budget.memtable_capacity_bytes() as u64,
            ),
            (
                "memtable_bytes",
                "Bytes currently held by live memtables across all column families",
                self.budget.memtable_bytes() as u64,
            ),
            (
                "block_cache_bytes",
                "Bytes currently charged to the shared block cache, including the memtable reservation",
                self.budget.cache_bytes() as u64,
            ),
            (
                "block_cache_pinned_bytes",
                "Bytes in the shared block cache that cannot be evicted",
                self.budget.cache_pinned_bytes() as u64,
            ),
        ];

        for (name, help, value) in readings {
            let gauge = ConstGauge::<u64>::new(value);
            let metric_encoder = encoder.encode_descriptor(name, help, None, gauge.metric_type())?;
            gauge.encode(metric_encoder)?;
        }

        Ok(())
    }
}

impl fmt::Debug for StateStoreMemoryCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateStoreMemoryCollector")
            .field("budget", &self.budget)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use prometheus_client::encoding::text::encode;

    use super::*;

    /// Pins the exported names: the sub-registry prefix combines with each metric name, and a
    /// dashboard built against the wrong names is silently empty.
    #[test]
    fn exports_capacity_and_usage() {
        let budget = RocksDbMemoryBudget::new(768 * 1024 * 1024, 384 * 1024 * 1024);

        let mut registry = Registry::default();
        StateStoreMemoryCollector::new(budget).register(&mut registry);

        let mut out = String::new();
        encode(&mut out, &registry).unwrap();

        assert!(
            out.contains("state_store_memory_capacity_bytes 805306368"),
            "capacity missing from:\n{out}"
        );
        assert!(
            out.contains("state_store_memtable_capacity_bytes 402653184"),
            "memtable capacity missing from:\n{out}"
        );
        assert!(
            out.contains("state_store_memtable_bytes 0"),
            "usage missing from:\n{out}"
        );
        assert!(
            out.contains("state_store_block_cache_bytes"),
            "cache usage missing from:\n{out}"
        );
    }
}
