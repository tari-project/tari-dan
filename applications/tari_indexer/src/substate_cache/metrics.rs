//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use prometheus_client::{metrics::counter::Counter, registry::Registry};

use crate::metrics::CollectorRegister;

/// Counters for what the cache does that a hit/miss ratio alone cannot explain: entries retired by
/// the transition stream, entries evicted at the size cap, and reads refused because the shard's
/// stream has fallen behind. A miss rate that climbs alongside `refused_stale` is a sync problem,
/// not a cache one.
///
/// The hit/miss pair itself is recorded by the manager that fronts the cache, under
/// `substate_scanner_cache_hits` and `substate_scanner_cache_misses`.
#[derive(Debug, Clone)]
pub struct SubstateCacheMetrics {
    invalidations: Counter,
    evictions: Counter,
    refused_stale: Counter,
}

impl SubstateCacheMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let registry = registry.sub_registry_with_prefix("substate_cache");
        Self {
            invalidations: Counter::default().register_at(
                "invalidations",
                "Number of cached substate entries retired by a transition from the state sync stream",
                registry,
            ),
            evictions: Counter::default().register_at(
                "evictions",
                "Number of cached substate entries evicted to stay within substate_cache_max_entries",
                registry,
            ),
            refused_stale: Counter::default().register_at(
                "refused_stale",
                "Number of cache reads refused because the substate's shard was not confirmed level with its \
                 committee recently enough to serve from",
                registry,
            ),
        }
    }

    pub fn add_invalidations(&self, n: usize) {
        self.invalidations.inc_by(n as u64);
    }

    pub fn add_evictions(&self, n: usize) {
        self.evictions.inc_by(n as u64);
    }

    pub fn inc_refused_stale(&self) {
        self.refused_stale.inc();
    }
}
