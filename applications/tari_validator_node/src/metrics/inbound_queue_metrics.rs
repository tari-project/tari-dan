//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Metrics for the bounded inbound queues that buffer network traffic ahead of the services that
//! consume it.
//!
//! Each queue already tracks its own depth and what it has turned away, so
//! [`InboundQueueCollector`] reads them on each scrape rather than maintaining separate gauges —
//! the metrics are always exactly in sync with the queues and there is no background task to spawn.
//! This mirrors [`crate::epoch_metrics::EpochManagerCollector`].
//!
//! Depth against budget is the number to watch when sizing the budgets: a queue that never rises
//! above a small fraction of its budget under load is over-provisioned, while a non-zero drop count
//! means traffic is being discarded and the budget (or the drain rate behind it) needs attention.

use std::fmt;

use prometheus_client::{
    collector::Collector,
    encoding::{DescriptorEncoder, EncodeMetric},
    metrics::{counter::ConstCounter, gauge::ConstGauge},
    registry::Registry,
};
use tari_networking::{GossipQueueSender, MessageQueueSender};

/// One queue's readings, taken at scrape time.
struct QueueReading {
    queue: &'static str,
    queued_bytes: u64,
    max_queued_bytes: u64,
    queued_messages: u64,
    dropped_messages: u64,
    dropped_bytes: u64,
}

/// A metric family: its name, its help text, and how to pull its value out of a [`QueueReading`].
type MetricFamily = (&'static str, &'static str, fn(&QueueReading) -> Reading);

/// Reports depth, budget and drop totals for every inbound queue, labelled by `queue`.
pub struct InboundQueueCollector {
    transaction_gossip: GossipQueueSender,
    consensus_gossip: GossipQueueSender,
    consensus_messaging: MessageQueueSender,
}

impl InboundQueueCollector {
    pub fn new(
        transaction_gossip: GossipQueueSender,
        consensus_gossip: GossipQueueSender,
        consensus_messaging: MessageQueueSender,
    ) -> Self {
        Self {
            transaction_gossip,
            consensus_gossip,
            consensus_messaging,
        }
    }

    /// Registers under the `inbound_queue` sub-registry, so the metric names on the wire are
    /// `inbound_queue_queued_bytes`, `inbound_queue_dropped_messages_total`, and so on.
    pub fn register(self, registry: &mut Registry) {
        let registry = registry.sub_registry_with_prefix("inbound_queue");
        registry.register_collector(Box::new(self));
    }

    fn readings(&self) -> [QueueReading; 3] {
        [
            QueueReading {
                queue: "transaction_gossip",
                queued_bytes: self.transaction_gossip.queued_bytes() as u64,
                max_queued_bytes: self.transaction_gossip.max_queued_bytes() as u64,
                queued_messages: self.transaction_gossip.queued_messages() as u64,
                dropped_messages: self.transaction_gossip.dropped_messages(),
                dropped_bytes: self.transaction_gossip.dropped_bytes(),
            },
            QueueReading {
                queue: "consensus_gossip",
                queued_bytes: self.consensus_gossip.queued_bytes() as u64,
                max_queued_bytes: self.consensus_gossip.max_queued_bytes() as u64,
                queued_messages: self.consensus_gossip.queued_messages() as u64,
                dropped_messages: self.consensus_gossip.dropped_messages(),
                dropped_bytes: self.consensus_gossip.dropped_bytes(),
            },
            QueueReading {
                queue: "consensus_messaging",
                queued_bytes: self.consensus_messaging.queued_bytes() as u64,
                max_queued_bytes: self.consensus_messaging.max_queued_bytes() as u64,
                queued_messages: self.consensus_messaging.queued_messages() as u64,
                dropped_messages: self.consensus_messaging.dropped_messages(),
                dropped_bytes: self.consensus_messaging.dropped_bytes(),
            },
        ]
    }
}

impl Collector for InboundQueueCollector {
    fn encode(&self, mut encoder: DescriptorEncoder) -> Result<(), fmt::Error> {
        let readings = self.readings();

        // Units are carried in the metric names rather than passed to the encoder, which would
        // append them a second time; counter names omit `_total`, which prometheus-client appends
        // itself. Both match `EpochManagerCollector`.
        let families: [MetricFamily; 5] = [
            (
                "queued_bytes",
                "Bytes currently held by messages awaiting processing",
                |r| Reading::Gauge(r.queued_bytes),
            ),
            (
                "max_queued_bytes",
                "Byte budget this queue admits up to before dropping",
                |r| Reading::Gauge(r.max_queued_bytes),
            ),
            ("queued_messages", "Messages currently awaiting processing", |r| {
                Reading::Gauge(r.queued_messages)
            }),
            (
                "dropped_messages",
                "Messages discarded because the queue was full",
                |r| Reading::Counter(r.dropped_messages),
            ),
            ("dropped_bytes", "Bytes discarded because the queue was full", |r| {
                Reading::Counter(r.dropped_bytes)
            }),
        ];

        for (name, help, read) in families {
            let metric_type = match read(&readings[0]) {
                Reading::Gauge(_) => ConstGauge::<u64>::new(0).metric_type(),
                Reading::Counter(_) => ConstCounter::<u64>::new(0).metric_type(),
            };
            let mut family = encoder.encode_descriptor(name, help, None, metric_type)?;
            for reading in &readings {
                let labels = [("queue", reading.queue)];
                match read(reading) {
                    Reading::Gauge(v) => ConstGauge::<u64>::new(v).encode(family.encode_family(&labels)?)?,
                    Reading::Counter(v) => ConstCounter::<u64>::new(v).encode(family.encode_family(&labels)?)?,
                }
            }
        }

        Ok(())
    }
}

enum Reading {
    Gauge(u64),
    Counter(u64),
}

impl fmt::Debug for InboundQueueCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InboundQueueCollector").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use prometheus_client::encoding::text::encode;
    use tari_networking::{gossip_queue, message_queue};

    use super::*;

    /// Pins the exported names and labels. Metric naming here is not free-form: the sub-registry
    /// prefix, the `queue` label and prometheus-client's own `_total` suffix on counters all
    /// combine, and a dashboard built against the wrong names is silently empty.
    #[test]
    fn exports_each_queue_under_its_own_label() {
        let (transaction_gossip, _rx_a) = gossip_queue(8, 1024);
        let (consensus_gossip, _rx_b) = gossip_queue(8, 2048);
        let (consensus_messaging, _rx_c) = message_queue(8, 4096);

        let mut registry = Registry::default();
        InboundQueueCollector::new(transaction_gossip, consensus_gossip, consensus_messaging).register(&mut registry);

        let mut out = String::new();
        encode(&mut out, &registry).unwrap();

        for (queue, budget) in [
            ("transaction_gossip", 1024),
            ("consensus_gossip", 2048),
            ("consensus_messaging", 4096),
        ] {
            assert!(
                out.contains(&format!("inbound_queue_max_queued_bytes{{queue=\"{queue}\"}} {budget}")),
                "{queue} budget missing from:\n{out}"
            );
            assert!(
                out.contains(&format!("inbound_queue_queued_bytes{{queue=\"{queue}\"}} 0")),
                "{queue} depth missing from:\n{out}"
            );
            assert!(
                out.contains(&format!("inbound_queue_dropped_messages_total{{queue=\"{queue}\"}} 0")),
                "{queue} drop counter missing from:\n{out}"
            );
        }
    }
}
