//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Prometheus instrumentation for the epoch manager and the epoch event oracle.
//!
//! The epoch manager exposes its current epoch via an [`Arc<AtomicU64>`] on its handle,
//! so [`EpochManagerCollector`] reads it on each scrape rather than maintaining a separate
//! gauge — the metric is always exactly in sync with the source of truth and there is no
//! background task to spawn.
//!
//! The epoch oracle is observed by wrapping the [`EpochEventOracle`] implementation with
//! [`MeteredEpochOracle`], which counts the events it forwards and tracks the highest
//! epoch ever seen across all event variants.

use std::fmt;

use prometheus_client::{
    collector::Collector,
    encoding::{DescriptorEncoder, EncodeMetric},
    metrics::{
        counter::Counter,
        gauge::{ConstGauge, Gauge},
    },
    registry::Registry,
};
use tari_common_types::types::FixedHash;
use tari_epoch_manager::{
    epoch_event_oracle::{EpochEvent, EpochEventOracle},
    service::EpochManagerHandle,
};
use tari_ootle_common_types::Epoch;
use tari_ootle_p2p::PeerAddress;

use crate::metrics::CollectorRegister;

type UnsignedGauge = Gauge<u64, std::sync::atomic::AtomicU64>;

// === Epoch oracle metrics ===

/// Counters/gauges for every event variant produced by an [`EpochEventOracle`].
#[derive(Debug, Clone)]
pub struct PrometheusEpochOracleMetrics {
    /// Highest epoch observed across all event variants the oracle has emitted. Useful as a
    /// liveness signal — a stuck oracle stops bumping this even if the validator node is
    /// otherwise healthy.
    last_event_epoch: UnsignedGauge,
    // Counter names are registered without a `_total` suffix: prometheus-client appends
    // `_total` itself during text/OpenMetrics encoding, so e.g. `epoch_changed` is exported
    // as `epoch_changed_total`. This matches the existing consensus/mempool metric structs.
    errors: Counter,
    epoch_changed: Counter,
    active_validator_set_changed: Counter,
    new_validator_registered: Counter,
    new_validator_exit: Counter,
    done_for_now: Counter,
}

impl PrometheusEpochOracleMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let registry = registry.sub_registry_with_prefix("epoch_oracle");
        Self {
            last_event_epoch: UnsignedGauge::default().register_at(
                "last_event_epoch",
                "Highest epoch observed across all events emitted by the epoch oracle",
                registry,
            ),
            errors: Counter::default().register_at(
                "errors",
                "Number of Error events emitted by the epoch oracle",
                registry,
            ),
            epoch_changed: Counter::default().register_at(
                "epoch_changed",
                "Number of EpochChanged events emitted by the epoch oracle",
                registry,
            ),
            active_validator_set_changed: Counter::default().register_at(
                "active_validator_set_changed",
                "Number of ActiveValidatorNodeSetChanged events emitted by the epoch oracle",
                registry,
            ),
            new_validator_registered: Counter::default().register_at(
                "new_validator_registered",
                "Number of NewValidatorRegistered events emitted by the epoch oracle",
                registry,
            ),
            new_validator_exit: Counter::default().register_at(
                "new_validator_exit",
                "Number of NewValidatorNodeExit events emitted by the epoch oracle",
                registry,
            ),
            done_for_now: Counter::default().register_at(
                "done_for_now",
                "Number of DoneForNow events emitted by the epoch oracle",
                registry,
            ),
        }
    }

    fn record(&self, event: &EpochEvent) {
        match event {
            EpochEvent::Error(_) => {
                self.errors.inc();
            },
            EpochEvent::ActiveValidatorNodeSetChanged { epoch, .. } => {
                self.active_validator_set_changed.inc();
                self.last_event_epoch.set(epoch.as_u64());
            },
            EpochEvent::NewValidatorRegistered { epoch, .. } => {
                self.new_validator_registered.inc();
                self.last_event_epoch.set(epoch.as_u64());
            },
            EpochEvent::NewValidatorNodeExit { epoch, .. } => {
                self.new_validator_exit.inc();
                self.last_event_epoch.set(epoch.as_u64());
            },
            EpochEvent::EpochChanged { epoch, .. } => {
                self.epoch_changed.inc();
                self.last_event_epoch.set(epoch.as_u64());
            },
            EpochEvent::DoneForNow { epoch, .. } => {
                self.done_for_now.inc();
                self.last_event_epoch.set(epoch.as_u64());
            },
        }
    }
}

// === MeteredEpochOracle wrapper ===

/// [`EpochEventOracle`] decorator that records every event flowing into the epoch manager.
///
/// Wraps any oracle implementation transparently — the epoch manager sees the same event
/// stream it would otherwise have received.
pub struct MeteredEpochOracle<O> {
    inner: O,
    metrics: PrometheusEpochOracleMetrics,
}

impl<O> MeteredEpochOracle<O> {
    pub fn new(inner: O, metrics: PrometheusEpochOracleMetrics) -> Self {
        Self { inner, metrics }
    }
}

impl<O: EpochEventOracle + Send> EpochEventOracle for MeteredEpochOracle<O> {
    async fn next_epoch_event(&mut self) -> Option<EpochEvent> {
        let event = self.inner.next_epoch_event().await;
        if let Some(ref e) = event {
            self.metrics.record(e);
        }
        event
    }

    fn is_within_epoch_end_spread(&self, current_epoch: Epoch) -> bool {
        self.inner.is_within_epoch_end_spread(current_epoch)
    }

    fn observed_epoch_boundary_hash(&self, epoch: Epoch) -> Option<FixedHash> {
        self.inner.observed_epoch_boundary_hash(epoch)
    }
}

// === Epoch manager current-epoch collector ===

/// Read-on-scrape collector that emits the epoch manager's current epoch.
///
/// The handle's `Arc<AtomicU64>` is the authoritative current-epoch value inside the
/// node, so we read it directly during encoding — no background task, no risk of the
/// metric drifting from the manager's state.
pub struct EpochManagerCollector {
    handle: EpochManagerHandle<PeerAddress>,
}

impl EpochManagerCollector {
    pub fn new(handle: EpochManagerHandle<PeerAddress>) -> Self {
        Self { handle }
    }

    /// Registers the collector under the `epoch_manager` sub-registry; the resulting
    /// metric name on the wire is `epoch_manager_current_epoch`.
    pub fn register(self, registry: &mut Registry) {
        let registry = registry.sub_registry_with_prefix("epoch_manager");
        registry.register_collector(Box::new(self));
    }
}

impl fmt::Debug for EpochManagerCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EpochManagerCollector").finish_non_exhaustive()
    }
}

impl Collector for EpochManagerCollector {
    fn encode(&self, mut encoder: DescriptorEncoder) -> Result<(), fmt::Error> {
        let epoch = self.handle.get_current_epoch().as_u64();
        let gauge = ConstGauge::<u64>::new(epoch);
        let metric_encoder = encoder.encode_descriptor(
            "current_epoch",
            "Current epoch as tracked by the epoch manager",
            None,
            gauge.metric_type(),
        )?;
        gauge.encode(metric_encoder)?;
        Ok(())
    }
}
