//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{num::NonZeroU32, time::Duration};

use libp2p::ping;

use crate::protocol_version::ProtocolVersion;

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct Config {
    pub protocol_version: ProtocolVersion,
    pub user_agent: String,
    pub messaging_protocol: String,
    pub ping: ping::Config,
    pub max_connections_per_peer: Option<u32>,
    pub enable_mdns: bool,
    pub enable_relay: bool,
    pub enable_messaging: bool,
    pub idle_connection_timeout: Duration,
    pub relay_circuit_limits: RelayCircuitLimits,
    pub relay_reservation_limits: RelayReservationLimits,
    pub identify_interval: Duration,
    /// The largest gossip message accepted or sent. Every other gossipsub bound below is a message
    /// count, so this is the factor that turns those counts into bytes.
    pub gossip_sub_max_message_size: usize,
    /// Heartbeat windows of full messages the gossipsub message cache retains.
    ///
    /// With `validate_messages` enabled this is also the deadline for the application's validation
    /// verdict, which is what makes it a liveness setting rather than only a memory one: a verdict
    /// arriving after the message has aged out of the cache neither forwards an accepted message
    /// nor scores a rejected sender. The verdict is reported once the message has been drained from
    /// the bounded inbound queue and validated, so this window must cover the queue's drain
    /// latency under the bursts those queues are sized to absorb.
    pub gossip_sub_history_length: usize,
    /// How many of the retained windows are advertised to peers (IHAVE).
    ///
    /// Must not exceed `gossip_sub_history_length`. The difference between the two is the margin in
    /// which an advertised message can still be served: advertising a window we no longer hold
    /// turns an IWANT into an unanswered promise, which the requester scores against us.
    pub gossip_sub_history_gossip: usize,
    /// How long a seen message's id is remembered so a duplicate arriving by another path is
    /// discarded rather than reprocessed. Ids only, so this is cheap; it should comfortably outlast
    /// the propagation time of a single message.
    pub gossip_sub_duplicate_cache_time: Duration,
    /// Messages that may queue for one connection awaiting transmission. This is the largest
    /// gossipsub memory term: each queued entry holds the full message, so the bytes a connection
    /// can hold are this count times `gossip_sub_max_message_size`. Beyond the queue, gossipsub
    /// drops for that peer rather than buffering without limit.
    ///
    /// A drop is scored against the *remote* peer as a slow receiver, so this is local pressure
    /// expressed as a judgement about someone else: sizing it too tightly graylists peers for our
    /// own backlog.
    pub gossip_sub_max_send_queue_messages: usize,
    /// Gossipsub topics on which peers are scored for delivering invalid messages. Empty disables
    /// peer scoring entirely.
    ///
    /// Only topics named here contribute to a peer's score, so a topic left out is one where
    /// invalid deliveries cost the sender nothing. Names are topic strings, matched exactly.
    pub gossip_sub_scored_topics: Vec<String>,
    pub rendezvous_server_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            protocol_version: "/tari/localnet/0.0.1".parse().unwrap(),
            user_agent: "/tari/unknown/0.0.1".to_string(),
            messaging_protocol: "/tari/messaging/0.0.1".to_string(),
            ping: ping::Config::default(),
            max_connections_per_peer: Some(3),
            enable_mdns: false,
            enable_relay: false,
            enable_messaging: true,
            idle_connection_timeout: Duration::from_secs(10 * 60),
            relay_circuit_limits: RelayCircuitLimits::default(),
            relay_reservation_limits: RelayReservationLimits::default(),
            // This is the default for identify
            identify_interval: Duration::from_secs(5 * 60),
            gossip_sub_max_message_size: 2 * 1024 * 1024,
            gossip_sub_history_length: 5,
            gossip_sub_history_gossip: 3,
            gossip_sub_duplicate_cache_time: Duration::from_secs(60),
            gossip_sub_max_send_queue_messages: 256,
            gossip_sub_scored_topics: Vec::new(),
            rendezvous_server_enabled: false,
        }
    }
}
#[derive(Debug, Clone)]
pub struct RelayCircuitLimits {
    pub max_limit: usize,
    pub max_per_peer: usize,
    pub max_duration: Duration,
    pub per_peer: Option<LimitPerInterval>,
    pub per_ip: Option<LimitPerInterval>,
    pub max_byte_limit: u64,
}

impl RelayCircuitLimits {
    pub fn high() -> Self {
        Self {
            max_limit: 64,
            max_per_peer: 8,
            max_duration: Duration::from_secs(4 * 60),
            per_peer: Some(LimitPerInterval {
                limit: NonZeroU32::new(60).expect("30 > 0"),
                interval: Duration::from_secs(2 * 60),
            }),
            per_ip: Some(LimitPerInterval {
                limit: NonZeroU32::new(120).expect("60 > 0"),
                interval: Duration::from_secs(60),
            }),
            max_byte_limit: 1 << 19, // 512KB
        }
    }
}

impl Default for RelayCircuitLimits {
    fn default() -> Self {
        // These reflect the default circuit limits in libp2p relay
        Self {
            max_limit: 16,
            max_per_peer: 4,
            max_duration: Duration::from_secs(2 * 60),
            per_peer: Some(LimitPerInterval {
                limit: NonZeroU32::new(30).expect("30 > 0"),
                interval: Duration::from_secs(2 * 60),
            }),
            per_ip: Some(LimitPerInterval {
                limit: NonZeroU32::new(60).expect("60 > 0"),
                interval: Duration::from_secs(60),
            }),
            max_byte_limit: 1 << 17, // 128KB
        }
    }
}

#[derive(Debug, Clone)]
pub struct RelayReservationLimits {
    pub max_limit: usize,
    pub max_per_peer: usize,
    pub max_duration: Duration,
    pub per_peer: Option<LimitPerInterval>,
    pub per_ip: Option<LimitPerInterval>,
}

impl RelayReservationLimits {
    pub fn high() -> Self {
        Self {
            max_limit: 128,
            max_per_peer: 8,
            max_duration: Duration::from_secs(4 * 60),
            per_peer: Some(LimitPerInterval {
                limit: NonZeroU32::new(60).expect("30 > 0"),
                interval: Duration::from_secs(2 * 60),
            }),
            per_ip: Some(LimitPerInterval {
                limit: NonZeroU32::new(120).expect("60 > 0"),
                interval: Duration::from_secs(60),
            }),
        }
    }
}

impl Default for RelayReservationLimits {
    fn default() -> Self {
        // These reflect the default reservation limits in libp2p relay
        Self {
            max_limit: 128,
            max_per_peer: 4,
            max_duration: Duration::from_secs(60 * 60),
            per_peer: Some(LimitPerInterval {
                limit: NonZeroU32::new(30).expect("30 > 0"),
                interval: Duration::from_secs(2 * 60),
            }),
            per_ip: Some(LimitPerInterval {
                limit: NonZeroU32::new(60).expect("60 > 0"),
                interval: Duration::from_secs(60),
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LimitPerInterval {
    pub limit: NonZeroU32,
    pub interval: Duration,
}
