//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use libp2p::{PeerId, gossipsub};
use tokio::sync::mpsc;

pub const TOPIC_DELIMITER: &str = "-";

#[derive(Debug, thiserror::Error)]
pub enum GossipSendError {
    #[error("Invalid token topic: {0}")]
    InvalidToken(String),
    #[error("Inbound gossip channel closed")]
    InboundGossipChannelClosed,
    #[error(
        "Inbound gossip queue for topic {topic} is full ({queued_messages} messages, {queued_bytes} bytes queued); \
         dropped a {len} byte message"
    )]
    QueueFull {
        topic: String,
        queued_messages: usize,
        queued_bytes: usize,
        len: usize,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum MessageSendError {
    #[error("Inbound message channel closed")]
    InboundMessageChannelClosed,
    #[error(
        "Inbound message queue is full ({queued_messages} messages, {queued_bytes} bytes queued); dropped a {len} \
         byte message from {peer_id}"
    )]
    QueueFull {
        peer_id: PeerId,
        queued_messages: usize,
        queued_bytes: usize,
        len: usize,
    },
}

/// An inbound gossipsub message, carrying the identifiers needed to report its validation verdict.
///
/// gossipsub runs in `validate_messages` mode, so a message is held and propagated to nobody until a
/// verdict is reported for it. A consumer MUST call
/// [`crate::NetworkingService::report_gossip_validation`] for every message it receives, or that
/// topic stops propagating across the network.
#[derive(Debug)]
pub struct GossipMessage {
    /// The peer that authored the message.
    pub source: PeerId,
    /// The peer we received it from, which may differ from `source`.
    pub propagation_source: PeerId,
    pub message_id: gossipsub::MessageId,
    pub message: gossipsub::Message,
    /// Holds this message's reservation against its topic queue's byte budget for as long as it is
    /// queued, releasing it when the message is dropped by the consumer. `None` until the message
    /// is admitted to a queue.
    _queue_permit: Option<QueuePermit>,
}

impl GossipMessage {
    pub fn new(
        source: PeerId,
        propagation_source: PeerId,
        message_id: gossipsub::MessageId,
        message: gossipsub::Message,
    ) -> Self {
        Self {
            source,
            propagation_source,
            message_id,
            message,
            _queue_permit: None,
        }
    }

    /// The pair identifying this message to [`crate::NetworkingService::report_gossip_validation`].
    pub fn validation_key(&self) -> (gossipsub::MessageId, PeerId) {
        (self.message_id.clone(), self.propagation_source)
    }

    fn with_queue_permit(mut self, permit: QueuePermit) -> Self {
        self._queue_permit = Some(permit);
        self
    }
}

/// Sender half of a bounded inbound gossip queue for one topic.
///
/// Bounds the queue by total size as well as message count. A count alone cannot do the job: every
/// topic admits messages up to the swarm's `gossip_sub_max_message_size`, so a count low enough to
/// bound worst-case memory is far too low for ordinary traffic, while a count high enough for
/// ordinary traffic admits that many maximum-size messages. Sizing by bytes is simultaneously
/// generous for real traffic (small messages, so many of them fit) and tight against a flood of
/// maximum-size ones.
///
/// Admission never blocks the networking worker: one worker task serves every topic, so waiting for
/// capacity on one topic would stall the others, consensus included. A message that does not fit is
/// rejected, and the worker reports `Ignore` for it — a full queue is this node's condition, not
/// misbehaviour by the peer that sent it, so it must not count against that peer's score.
#[derive(Debug, Clone)]
pub struct GossipQueueSender {
    tx: mpsc::Sender<GossipMessage>,
    budget: QueueAccounting,
}

/// Creates a bounded inbound gossip queue for a single topic.
pub fn gossip_queue(
    max_queued_messages: usize,
    max_queued_bytes: usize,
) -> (GossipQueueSender, mpsc::Receiver<GossipMessage>) {
    let (tx, rx) = mpsc::channel(max_queued_messages);
    (
        GossipQueueSender {
            tx,
            budget: QueueAccounting::new(max_queued_bytes),
        },
        rx,
    )
}

impl GossipQueueSender {
    /// Bytes currently held by queued messages.
    pub fn queued_bytes(&self) -> usize {
        self.budget.queued()
    }

    /// The byte budget this queue admits up to.
    pub fn max_queued_bytes(&self) -> usize {
        self.budget.max
    }

    /// Messages currently queued.
    pub fn queued_messages(&self) -> usize {
        self.tx.max_capacity() - self.tx.capacity()
    }

    /// Messages turned away because the queue was full, since start-up.
    pub fn dropped_messages(&self) -> u64 {
        self.budget.dropped_messages()
    }

    /// Bytes turned away because the queue was full, since start-up.
    pub fn dropped_bytes(&self) -> u64 {
        self.budget.dropped_bytes()
    }

    fn try_send(&self, msg: GossipMessage) -> Result<(), GossipSendError> {
        let len = msg.message.data.len();
        let full = |msg: &GossipMessage| {
            self.budget.record_drop(len);
            GossipSendError::QueueFull {
                topic: msg.message.topic.to_string(),
                queued_messages: self.queued_messages(),
                queued_bytes: self.queued_bytes(),
                len,
            }
        };

        let Some(permit) = self.budget.reserve(len) else {
            return Err(full(&msg));
        };

        match self.tx.try_send(msg.with_queue_permit(permit)) {
            Ok(()) => Ok(()),
            // The rejected message carries its permit, so the reservation is released as it drops.
            Err(mpsc::error::TrySendError::Full(msg)) => Err(full(&msg)),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(GossipSendError::InboundGossipChannelClosed),
        }
    }
}

/// An inbound direct message, carrying its reservation against the inbound queue's byte budget.
///
/// The message is held in the encoding it arrived in and decoded by the consumer. A decoded message can occupy many
/// times its encoded size, so holding the encoding is what lets the queue charge exactly what it holds.
#[derive(Debug)]
pub struct InboundMessage {
    pub peer_id: PeerId,
    pub message: Bytes,
    /// Released when this message is dropped by the consumer. `None` until it is admitted.
    _queue_permit: Option<QueuePermit>,
}

/// Sender half of the bounded inbound queue for direct (non-gossip) messages.
///
/// Bounded on the same basis as [`GossipQueueSender`], and for the same reason: the queue drains
/// serially into consensus while messages vary from small votes to block-sized proposals, so a
/// count alone cannot bound memory without also throttling ordinary traffic. Admission does not
/// block the networking worker; a message that does not fit is dropped.
#[derive(Debug, Clone)]
pub struct MessageQueueSender {
    tx: mpsc::Sender<InboundMessage>,
    budget: QueueAccounting,
}

/// Creates a bounded inbound queue for direct messages.
pub fn message_queue(
    max_queued_messages: usize,
    max_queued_bytes: usize,
) -> (MessageQueueSender, mpsc::Receiver<InboundMessage>) {
    let (tx, rx) = mpsc::channel(max_queued_messages);
    (
        MessageQueueSender {
            tx,
            budget: QueueAccounting::new(max_queued_bytes),
        },
        rx,
    )
}

impl MessageQueueSender {
    /// Bytes currently held by queued messages.
    pub fn queued_bytes(&self) -> usize {
        self.budget.queued()
    }

    /// The byte budget this queue admits up to.
    pub fn max_queued_bytes(&self) -> usize {
        self.budget.max
    }

    /// Messages currently queued.
    pub fn queued_messages(&self) -> usize {
        self.tx.max_capacity() - self.tx.capacity()
    }

    /// Messages turned away because the queue was full, since start-up.
    pub fn dropped_messages(&self) -> u64 {
        self.budget.dropped_messages()
    }

    /// Bytes turned away because the queue was full, since start-up.
    pub fn dropped_bytes(&self) -> u64 {
        self.budget.dropped_bytes()
    }

    fn try_send(&self, peer_id: PeerId, message: Bytes) -> Result<(), MessageSendError> {
        let len = message.len();
        let full = || {
            self.budget.record_drop(len);
            MessageSendError::QueueFull {
                peer_id,
                queued_messages: self.queued_messages(),
                queued_bytes: self.queued_bytes(),
                len,
            }
        };

        let Some(permit) = self.budget.reserve(len) else {
            return Err(full());
        };

        match self.tx.try_send(InboundMessage {
            peer_id,
            message,
            _queue_permit: Some(permit),
        }) {
            Ok(()) => Ok(()),
            // The rejected message carries its permit, so the reservation is released as it drops.
            Err(mpsc::error::TrySendError::Full(_)) => Err(full()),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(MessageSendError::InboundMessageChannelClosed),
        }
    }
}

/// Accounting for one bounded inbound queue: the bytes it currently holds, and what it has turned
/// away. Reservations are released when the message they travel with is dropped.
///
/// Drop counters are maintained unconditionally rather than behind a metrics feature — they are two
/// relaxed atomic adds on a path that has already decided to discard a message, and a queue whose
/// drops are invisible cannot be sized against real traffic.
#[derive(Debug, Clone)]
struct QueueAccounting {
    queued: Arc<AtomicUsize>,
    max: usize,
    dropped_messages: Arc<AtomicU64>,
    dropped_bytes: Arc<AtomicU64>,
}

impl QueueAccounting {
    fn new(max: usize) -> Self {
        Self {
            queued: Arc::new(AtomicUsize::new(0)),
            max,
            dropped_messages: Arc::new(AtomicU64::new(0)),
            dropped_bytes: Arc::new(AtomicU64::new(0)),
        }
    }

    fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    fn dropped_messages(&self) -> u64 {
        self.dropped_messages.load(Ordering::Relaxed)
    }

    fn dropped_bytes(&self) -> u64 {
        self.dropped_bytes.load(Ordering::Relaxed)
    }

    fn record_drop(&self, len: usize) {
        self.dropped_messages.fetch_add(1, Ordering::Relaxed);
        self.dropped_bytes.fetch_add(len as u64, Ordering::Relaxed);
    }

    /// Reserves `len` bytes, or `None` when that would exceed the budget.
    fn reserve(&self, len: usize) -> Option<QueuePermit> {
        let previous = self.queued.fetch_add(len, Ordering::AcqRel);
        if previous.saturating_add(len) > self.max {
            self.queued.fetch_sub(len, Ordering::AcqRel);
            return None;
        }
        Some(QueuePermit {
            queued_bytes: self.queued.clone(),
            len,
        })
    }
}

/// Releases a queued message's share of its topic queue's byte budget when dropped.
#[derive(Debug)]
struct QueuePermit {
    queued_bytes: Arc<AtomicUsize>,
    len: usize,
}

impl Drop for QueuePermit {
    fn drop(&mut self) {
        self.queued_bytes.fetch_sub(self.len, Ordering::AcqRel);
    }
}

pub enum MessagingMode {
    Enabled {
        tx_messages: MessageQueueSender,
        tx_gossip_messages_by_topic: HashMap<String, GossipQueueSender>,
    },
    /// Gossip topics are consumed, but the direct messaging protocol is not negotiated with peers.
    /// For a node that takes part in gossip meshes without being addressable for point-to-point
    /// messages.
    GossipOnly {
        tx_gossip_messages_by_topic: HashMap<String, GossipQueueSender>,
    },
    Disabled,
}

impl MessagingMode {
    /// Whether the direct messaging protocol should be negotiated. Gossip is carried by gossipsub
    /// regardless, so this is false under [`MessagingMode::GossipOnly`].
    pub fn is_enabled(&self) -> bool {
        matches!(self, MessagingMode::Enabled { .. })
    }

    /// Queues a direct message received from `peer_id`, still in the encoding it arrived in.
    pub fn send_message(&self, peer_id: PeerId, msg: Bytes) -> Result<(), MessageSendError> {
        if let MessagingMode::Enabled { tx_messages, .. } = self {
            tx_messages.try_send(peer_id, msg)?;
        }
        Ok(())
    }

    pub fn send_gossip_message(&self, msg: GossipMessage) -> Result<(), GossipSendError> {
        let queues = match self {
            MessagingMode::Enabled {
                tx_gossip_messages_by_topic,
                ..
            } |
            MessagingMode::GossipOnly {
                tx_gossip_messages_by_topic,
            } => tx_gossip_messages_by_topic,
            // The worker turns this error into an `Ignore` verdict. Without it a subscribed topic
            // whose messages nobody consumes leaves every message pinned in gossipsub's validation
            // cache, never propagated and never released.
            MessagingMode::Disabled => return Err(GossipSendError::InvalidToken(msg.message.topic.to_string())),
        };

        // Topics may be a bare prefix (single global topic, e.g. "consensus") or a prefixed topic
        // (e.g. "transactions-0-15"). Route on the prefix before the first delimiter, falling back to the
        // whole topic when there is no delimiter.
        let queue = {
            let topic = msg.message.topic.as_str();
            let prefix = topic.split_once(TOPIC_DELIMITER).map_or(topic, |(prefix, _)| prefix);
            queues
                .get(prefix)
                .ok_or_else(|| GossipSendError::InvalidToken(topic.to_string()))?
        };
        queue.try_send(msg)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(len: usize) -> GossipMessage {
        GossipMessage::new(
            PeerId::random(),
            PeerId::random(),
            gossipsub::MessageId::new(b"id"),
            gossipsub::Message {
                source: None,
                data: vec![0u8; len],
                sequence_number: None,
                topic: gossipsub::IdentTopic::new("test").hash(),
            },
        )
    }

    #[test]
    fn admits_messages_until_the_byte_budget_is_reached() {
        let (tx, _rx) = gossip_queue(16, 100);

        tx.try_send(message(60)).unwrap();
        assert_eq!(tx.queued_bytes(), 60);

        let err = tx.try_send(message(60)).unwrap_err();
        assert!(matches!(err, GossipSendError::QueueFull { .. }));
        assert_eq!(tx.queued_bytes(), 60, "a rejected message must not hold budget");

        tx.try_send(message(40)).unwrap();
        assert_eq!(tx.queued_bytes(), 100, "a message that exactly fits is admitted");
    }

    #[test]
    fn budget_is_released_once_a_message_is_consumed() {
        let (tx, mut rx) = gossip_queue(16, 100);
        tx.try_send(message(100)).unwrap();

        let err = tx.try_send(message(1)).unwrap_err();
        assert!(matches!(err, GossipSendError::QueueFull { .. }));

        drop(rx.try_recv().unwrap());
        assert_eq!(tx.queued_bytes(), 0);
        tx.try_send(message(100)).unwrap();
    }

    #[test]
    fn drops_are_counted_on_both_bounds() {
        // Sizing the budgets against real traffic depends on these counters, so a rejection by
        // either bound must be visible.
        let (tx, _rx) = gossip_queue(1, 100);
        assert_eq!(tx.dropped_messages(), 0);

        tx.try_send(message(10)).unwrap();

        // Rejected by the byte budget.
        tx.try_send(message(200)).unwrap_err();
        assert_eq!(tx.dropped_messages(), 1);
        assert_eq!(tx.dropped_bytes(), 200);

        // Rejected by the message count, which is already at its limit of one.
        tx.try_send(message(1)).unwrap_err();
        assert_eq!(tx.dropped_messages(), 2);
        assert_eq!(tx.dropped_bytes(), 201);

        assert_eq!(
            tx.queued_bytes(),
            10,
            "rejections must not disturb the admitted message"
        );
    }

    #[test]
    fn direct_messages_are_charged_the_bytes_they_hold() {
        let (tx, mut rx) = message_queue(16, 100);
        let peer = PeerId::random();

        tx.try_send(peer, Bytes::from(vec![0u8; 60])).unwrap();
        assert_eq!(tx.queued_bytes(), 60);

        let err = tx.try_send(peer, Bytes::from(vec![0u8; 60])).unwrap_err();
        assert!(matches!(err, MessageSendError::QueueFull { .. }));
        assert_eq!(tx.queued_bytes(), 60, "a rejected message must not hold budget");

        drop(rx.try_recv().unwrap());
        assert_eq!(tx.queued_bytes(), 0);
        tx.try_send(peer, Bytes::from(vec![0u8; 100])).unwrap();
    }

    #[test]
    fn message_count_is_bounded_independently_of_size() {
        // A flood of tiny messages carries per-message overhead the byte budget does not see.
        let (tx, _rx) = gossip_queue(2, 1024 * 1024);
        tx.try_send(message(1)).unwrap();
        tx.try_send(message(1)).unwrap();

        let err = tx.try_send(message(1)).unwrap_err();
        assert!(matches!(err, GossipSendError::QueueFull { .. }));
        assert_eq!(tx.queued_bytes(), 2, "the rejected message released its reservation");
    }
}
