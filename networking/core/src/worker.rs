//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, hash_map::Entry},
    hash::Hash,
    sync::Arc,
    time::{Duration, Instant},
};

use libp2p::{
    Multiaddr,
    PeerId,
    StreamProtocol,
    autonat,
    autonat::NatStatus,
    core::ConnectedPoint,
    dcutr,
    futures::StreamExt,
    gossipsub,
    gossipsub::{MessageId, PublishError, TopicHash},
    identify,
    identity,
    mdns,
    multiaddr::Protocol,
    ping,
    relay,
    swarm::{
        ConnectionId,
        DialError,
        SwarmEvent,
        dial_opts::{DialOpts, PeerCondition},
    },
};
use log::*;
use tari_rpc_framework::Substream;
use tari_shutdown::ShutdownSignal;
use tari_swarm::{
    TariNodeBehaviourEvent,
    TariSwarm,
    is_supported_multiaddr,
    messaging,
    messaging::{prost, prost::ProstCodec},
    protocol_ids,
    rendezvous,
    rendezvous::Namespace,
    substream,
    substream::{NegotiatedSubstream, ProtocolNotification, StreamId},
};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time,
};

use crate::{
    GossipMessage,
    MessageSpec,
    MessagingMode,
    NetworkingError,
    ReachabilityMode,
    connection::Connection,
    event::NetworkingEvent,
    global_ip::GlobalIp,
    handle::NetworkingRequest,
    notify::Notifiers,
    relay_state::RelayState,
    rendezvous_state::RendezvousState,
};

const LOG_TARGET: &str = "tari::networking::service::worker";

type ReplyTx<T> = oneshot::Sender<Result<T, NetworkingError>>;

// const PEER_ANNOUNCE_TOPIC: &str = "peer-announce";

pub struct NetworkingWorker<TMsg>
where
    TMsg: MessageSpec,
    TMsg::Message: prost::Message + Default + Clone + 'static,
    TMsg::TransactionGossipMessage: prost::Message + Default + Clone + 'static,
    TMsg::ConsensusGossipMessage: prost::Message + Default + Clone + 'static,
{
    _keypair: identity::Keypair,
    rx_request: mpsc::Receiver<NetworkingRequest<TMsg>>,
    tx_events: broadcast::Sender<NetworkingEvent>,
    messaging_mode: MessagingMode<TMsg>,
    active_connections: HashMap<PeerId, Vec<Connection>>,
    pending_substream_requests: HashMap<StreamId, ReplyTx<NegotiatedSubstream<Substream>>>,
    pending_dial_requests: HashMap<PeerId, Vec<ReplyTx<()>>>,
    substream_notifiers: Notifiers<Substream>,
    topic_peers: HashMap<TopicHash, Vec<PeerId>>,
    swarm: TariSwarm<ProstCodec<TMsg::Message>>,
    config: crate::Config,
    rendezvous_state: RendezvousState,
    relays: RelayState,
    seed_peers: Vec<(Option<PeerId>, Multiaddr)>,
    is_initial_bootstrap_complete: bool,
    #[cfg(feature = "metrics")]
    metrics: Option<tari_swarm::metrics::Metrics>,
    shutdown_signal: ShutdownSignal,
}

impl<TMsg> NetworkingWorker<TMsg>
where
    TMsg: MessageSpec,
    TMsg::Message: prost::Message + Default + Clone + 'static,
    TMsg::TransactionGossipMessage: prost::Message + Default + Clone + 'static,
    TMsg::ConsensusGossipMessage: prost::Message + Default + Clone + 'static,
{
    pub(crate) fn new(
        keypair: identity::Keypair,
        rx_request: mpsc::Receiver<NetworkingRequest<TMsg>>,
        tx_events: broadcast::Sender<NetworkingEvent>,
        messaging_mode: MessagingMode<TMsg>,
        swarm: TariSwarm<ProstCodec<TMsg::Message>>,
        config: crate::Config,
        known_relay_nodes: Vec<(PeerId, Multiaddr)>,
        seed_peers: Vec<(Option<PeerId>, Multiaddr)>,
        shutdown_signal: ShutdownSignal,
    ) -> Self {
        // Rendezvous servers are assumed to be the configured seed peers that have a known peer id. This is
        // best-effort: seed peers that do not run a rendezvous server simply yield failed (and harmless)
        // register/discover attempts.
        let rendezvous_state = RendezvousState::new(
            seed_peers
                .iter()
                .filter_map(|(peer_id, addr)| Some(((*peer_id)?, addr.clone()))),
        );
        Self {
            _keypair: keypair,
            rx_request,
            tx_events,
            messaging_mode,
            substream_notifiers: Notifiers::new(),
            active_connections: HashMap::new(),
            pending_substream_requests: HashMap::new(),
            pending_dial_requests: HashMap::new(),
            relays: RelayState::new(known_relay_nodes),
            topic_peers: HashMap::new(),
            rendezvous_state,
            seed_peers,
            swarm,
            config,
            is_initial_bootstrap_complete: false,
            #[cfg(feature = "metrics")]
            metrics: None,
            shutdown_signal,
        }
    }

    #[cfg(feature = "metrics")]
    pub fn with_metrics(mut self, registry: &mut prometheus_client::registry::Registry) -> Self {
        self.metrics = Some(tari_swarm::metrics::Metrics::new(registry));
        self
    }

    pub fn add_protocol_notifier(
        &mut self,
        protocol: StreamProtocol,
        sender: mpsc::UnboundedSender<ProtocolNotification<Substream>>,
    ) {
        self.substream_notifiers.add(protocol, sender);
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        info!(target: LOG_TARGET, "🌐 Starting networking service {:?}", self.config);
        if self.config.listeners.is_empty() {
            self.config.reachability_mode = ReachabilityMode::Private;
            info!(target: LOG_TARGET, "No listeners configured, this node will not accept incoming connections");
        }
        // Setup listeners
        for listener in &self.config.listeners {
            if !is_supported_multiaddr(listener) {
                warn!(target: LOG_TARGET, "Unsupported listener multiaddr {}", listener);
                continue;
            }
            self.swarm.listen_on(listener.clone())?;
        }

        if self.config.reachability_mode.is_private() {
            self.attempt_relay_reservation();
        }

        let mut check_connections_interval = time::interval(self.config.check_connections_interval);

        // self.swarm
        //     .behaviour_mut()
        //     .gossipsub
        //     .subscribe(&IdentTopic::new(PEER_ANNOUNCE_TOPIC))?;

        loop {
            tokio::select! {
                Some(request) = self.rx_request.recv() => {
                    self.handle_request(request).await?;
                }
                Some(event) = self.swarm.next() => {
                    if let Err(err) = self.on_swarm_event(event).await {
                        error!(target: LOG_TARGET, "🚨 Swarm event error: {}", err);
                    }
                },
                _ =  check_connections_interval.tick() => {
                    if let Err(err) = self.bootstrap().await {
                        error!(target: LOG_TARGET, "🚨 Failed to bootstrap: {}", err);
                    }
                },

                _ = self.shutdown_signal.wait() => {
                    info!(target: LOG_TARGET, "💤 Networking service shutting down");
                    break;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_request(&mut self, request: NetworkingRequest<TMsg>) -> Result<(), NetworkingError> {
        match request {
            NetworkingRequest::DialPeer { dial_opts, reply_tx } => {
                let (tx_waiter, rx_waiter) = oneshot::channel();
                let maybe_peer_id = dial_opts.get_peer_id();
                info!(target: LOG_TARGET, "🤝 Dialing peer {:?}", dial_opts);

                match self.swarm.dial(dial_opts) {
                    Ok(_) => {
                        if let Some(peer_id) = maybe_peer_id {
                            self.pending_dial_requests.entry(peer_id).or_default().push(tx_waiter);
                        }
                        let _ignore = reply_tx.send(Ok(rx_waiter.into()));
                    },
                    Err(err) => {
                        info!(target: LOG_TARGET, "🚨 Failed to dial peer: {}",  err);
                        let _ignore = reply_tx.send(Err(err.into()));
                    },
                }
            },
            NetworkingRequest::GetConnectedPeers { reply_tx } => {
                let peers = self.swarm.connected_peers().copied().collect();
                let _ignore = reply_tx.send(Ok(peers));
            },
            NetworkingRequest::SendMessage {
                peer,
                message,
                reply_tx,
            } => {
                match self
                    .swarm
                    .behaviour_mut()
                    .messaging
                    .as_mut()
                    .map(|m| m.send_message(peer, message))
                {
                    Some(Ok(_)) => {
                        debug!(target: LOG_TARGET, "📢 Queued message to peer {}", peer);
                        let _ignore = reply_tx.send(Ok(()));
                    },
                    Some(Err(err)) => {
                        debug!(target: LOG_TARGET, "🚨 Failed to queue message to peer {}: {}", peer, err);
                        let _ignore = reply_tx.send(Err(err.into()));
                    },
                    None => {
                        warn!(target: LOG_TARGET, "Sent message but messaging is disabled");
                        let _ignore = reply_tx.send(Err(NetworkingError::MessagingDisabled));
                    },
                }
            },
            NetworkingRequest::SendMulticast {
                destination,
                message,
                reply_tx,
            } => {
                let len = destination.len();
                let Some(messaging_mut) = &mut self.swarm.behaviour_mut().messaging.as_mut() else {
                    warn!(target: LOG_TARGET, "Sent multicast message but messaging is disabled");
                    let _ignore = reply_tx.send(Err(NetworkingError::MessagingDisabled));
                    return Ok(());
                };

                let mut num_sent = 0;
                for peer in destination {
                    match messaging_mut.send_message(peer, message.clone()) {
                        Ok(_) => {
                            num_sent += 1;
                        },
                        Err(err) => {
                            debug!(target: LOG_TARGET, "🚨 Failed to queue message to peer {}: {}", peer, err);
                        },
                    }
                }
                debug!(target: LOG_TARGET, "📢 Queued message to {num_sent} out of {len} peers");
                let _ignore = reply_tx.send(Ok(num_sent));
            },
            NetworkingRequest::PublishGossip {
                topic,
                message,
                reply_tx,
            } => match self.swarm.behaviour_mut().gossipsub.publish(topic.hash(), message) {
                Ok(msg_id) => {
                    debug!(target: LOG_TARGET, "📢 Published gossipsub message on {topic}: {}", msg_id);
                    let _ignore = reply_tx.send(Ok(()));
                },
                Err(err @ PublishError::Duplicate) => {
                    debug!(target: LOG_TARGET, "Not publishing duplicate message on {topic}");
                    let _ignore = reply_tx.send(Err(err.into()));
                },
                Err(err) => {
                    debug!(target: LOG_TARGET, "🚨 Failed to publish gossipsub message on {topic}: {}", err);
                    let _ignore = reply_tx.send(Err(err.into()));
                },
            },
            NetworkingRequest::ReportGossipValidation {
                message_id,
                propagation_source,
                acceptance,
            } => {
                if !self.swarm.behaviour_mut().gossipsub.report_message_validation_result(
                    &message_id,
                    &propagation_source,
                    acceptance,
                ) {
                    warn!(target: LOG_TARGET, "Unable to report gossip validation result for {message_id} because message was not in cache");
                }
            },
            NetworkingRequest::SubscribeTopic {
                topic,
                explicit_topic_peers,
                reply_tx,
            } => {
                if !explicit_topic_peers.is_empty() {
                    info!(target: LOG_TARGET, "📢 Adding {} explicit peers to topic {}", explicit_topic_peers.len(), topic);
                    for peer in &explicit_topic_peers {
                        self.swarm.behaviour_mut().gossipsub.add_explicit_peer(peer);
                    }
                    self.topic_peers.insert(topic.hash(), explicit_topic_peers);
                }
                match self.swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                    Ok(_) => {
                        debug!(target: LOG_TARGET, "📢 Subscribed to gossipsub topic: {}", topic);
                        let _ignore = reply_tx.send(Ok(()));
                    },
                    Err(err) => {
                        error!(target: LOG_TARGET, "🚨 Failed to subscribe to gossipsub topic: {}", err);
                        let _ignore = reply_tx.send(Err(err.into()));
                    },
                }
            },
            NetworkingRequest::UnsubscribeTopic { topic, reply_tx } => {
                if let Some(peers) = self.topic_peers.remove(&topic.hash()) {
                    for peer in peers {
                        self.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer);
                    }
                }

                if self.swarm.behaviour_mut().gossipsub.unsubscribe(&topic) {
                    debug!(target: LOG_TARGET, "📢 Unsubscribed from gossipsub topic: {}", topic);
                } else {
                    debug!(target: LOG_TARGET, "Unsubscribe from topic {} that is not subscribed", topic);
                }
                let _ignore = reply_tx.send(Ok(()));
            },
            NetworkingRequest::IsSubscribedTopic { topic, reply_tx } => {
                let hash = topic.hash();
                let found = self.swarm.behaviour_mut().gossipsub.topics().any(|t| *t == hash);
                let _ignore = reply_tx.send(Ok(found));
            },
            NetworkingRequest::OpenSubstream {
                peer_id,
                protocol_id,
                reply_tx,
            } => {
                let stream_id = self
                    .swarm
                    .behaviour_mut()
                    .substream
                    .open_substream(peer_id, protocol_id.clone());
                self.pending_substream_requests.insert(stream_id, reply_tx);
            },
            NetworkingRequest::AddProtocolNotifier { protocols, tx_notifier } => {
                for protocol in protocols {
                    self.add_protocol_notifier(protocol.clone(), tx_notifier.clone());
                    self.swarm.behaviour_mut().substream.add_protocol(protocol);
                }
            },
            NetworkingRequest::GetActiveConnections { reply_tx } => {
                let connections = self.active_connections.values().flatten().cloned().collect();
                let _ignore = reply_tx.send(Ok(connections));
            },
            NetworkingRequest::GetLocalPeerInfo { reply_tx } => {
                let peer = crate::peer::PeerInfo {
                    peer_id: *self.swarm.local_peer_id(),
                    protocol_version: self.config.swarm.protocol_version.to_string(),
                    agent_version: self.config.swarm.user_agent.clone(),
                    listen_addrs: self.swarm.listeners().cloned().collect(),
                    // TODO: this isnt all the protocols, not sure if there is an easy way to get them all
                    protocols: self.swarm.behaviour_mut().substream.supported_protocols().to_vec(),
                    // observed_addr: (),
                };
                let _ignore = reply_tx.send(Ok(peer));
            },
            NetworkingRequest::SetWantPeers(peers) => {
                info!(target: LOG_TARGET, "🧭 (CURRENTLY NO-OP) Setting want peers to {:?} ", peers);
            },
        }

        Ok(())
    }

    async fn bootstrap(&mut self) -> Result<(), NetworkingError> {
        if !self.is_initial_bootstrap_complete {
            // NOTE: rendezvous servers are a subset of the seed peers (those with a known peer id), so they are dialed
            // by the loop below. Reconnection (e.g. after an idle timeout) is handled in
            // `discover_rendezvous_peers`.
            info!(target: LOG_TARGET, "🥾 BOOTSTRAP: dialing {} seed peers", self.seed_peers.len());
            for (peer, addr) in self.seed_peers.drain(..) {
                let opts = match peer {
                    Some(peer) => {
                        self.swarm
                            .behaviour_mut()
                            .peer_store
                            .store_mut()
                            .add_address(&peer, &addr);
                        DialOpts::peer_id(peer)
                            .addresses(vec![addr])
                            .condition(PeerCondition::DisconnectedAndNotDialing)
                            .extend_addresses_through_behaviour()
                            .build()
                    },
                    None => DialOpts::unknown_peer_id().address(addr).build(),
                };

                self.swarm.dial(opts).or_else(|err| {
                    // Peer already has pending dial or established connection - OK
                    if matches!(&err, DialError::DialPeerConditionFalse(_)) {
                        Ok(())
                    } else {
                        Err(err)
                    }
                })?;
            }
        }

        if self.active_connections.len() < self.relays.num_possible_relays() {
            info!(target: LOG_TARGET, "🥾 BOOTSTRAP: dialing {} known relay peers", self.relays.num_possible_relays());
            for (peer, addrs) in self.relays.possible_relays() {
                for addr in addrs {
                    self.swarm
                        .behaviour_mut()
                        .peer_store
                        .store_mut()
                        .add_address(peer, addr);
                }
                self.swarm
                    .dial(
                        DialOpts::peer_id(*peer)
                            .addresses(addrs.iter().cloned().collect())
                            .condition(PeerCondition::DisconnectedAndNotDialing)
                            .extend_addresses_through_behaviour()
                            .build(),
                    )
                    .or_else(|err| {
                        // Peer already has pending dial or established connection - OK
                        if matches!(&err, DialError::DialPeerConditionFalse(_)) {
                            Ok(())
                        } else {
                            Err(err)
                        }
                    })?;
            }

            self.is_initial_bootstrap_complete = true;
        }

        // Periodically refresh the peer list from the rendezvous servers (if any) to pick up newly-registered peers.
        self.discover_rendezvous_peers(None);

        Ok(())
    }

    async fn on_swarm_event(
        &mut self,
        event: SwarmEvent<TariNodeBehaviourEvent<ProstCodec<TMsg::Message>>>,
    ) -> Result<(), NetworkingError> {
        match event {
            SwarmEvent::Behaviour(event) => self.on_behaviour_event(event).await?,
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                num_established,
                concurrent_dial_errors,
                established_in,
            } => self.on_connection_established(
                peer_id,
                connection_id,
                endpoint,
                num_established.get(),
                concurrent_dial_errors.map(|c| c.len()).unwrap_or(0),
                established_in,
            )?,
            SwarmEvent::ConnectionClosed {
                peer_id,
                endpoint,
                cause,
                connection_id,
                ..
            } => {
                info!(target: LOG_TARGET, "🔌 Connection closed: peer_id={}, endpoint={:?}, cause={:?}", peer_id, endpoint, cause);
                match self.active_connections.entry(peer_id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().retain(|c| c.connection_id != connection_id);
                        if entry.get().is_empty() {
                            entry.remove_entry();
                        }
                    },
                    Entry::Vacant(_) => {
                        debug!(target: LOG_TARGET, "Connection closed for peer {peer_id} but this connection is not in the active connections list");
                    },
                }
                shrink_hashmap_if_required(&mut self.active_connections);
                if let Some(selected) = self.relays.selected_relay() &&
                    selected.circuit_connection_id == Some(connection_id)
                {
                    // Our selected relay has disconnected, attempt to reserve another
                    self.attempt_relay_reservation();
                }
            },
            SwarmEvent::OutgoingConnectionError {
                peer_id: Some(peer_id),
                error,
                ..
            } => {
                debug!(target: LOG_TARGET, "🚨 Outgoing connection error: peer_id={}, error={}", peer_id, error);
                let Some(waiters) = self.pending_dial_requests.remove(&peer_id) else {
                    debug!(target: LOG_TARGET, "No pending dial requests initiated by this service for peer {}", peer_id);
                    return Ok(());
                };
                shrink_hashmap_if_required(&mut self.pending_dial_requests);

                for waiter in waiters {
                    let _ignore = waiter.send(Err(NetworkingError::OutgoingConnectionError(error.to_string())));
                }

                if matches!(error, DialError::NoAddresses) {
                    self.discover_rendezvous_peers(None);
                }
            },
            SwarmEvent::ExternalAddrConfirmed { address } => {
                info!(target: LOG_TARGET, "🌍️ External address confirmed: {}", address);
            },
            SwarmEvent::Dialing { peer_id, connection_id } => {
                if let Some(peer_id) = peer_id {
                    info!(target: LOG_TARGET, "🤝 Dialing peer {peer_id} for connection({connection_id})");
                } else {
                    info!(target: LOG_TARGET, "🤝 Dialing unknown peer for connection({connection_id})");
                }
            },
            e => {
                trace!(target: LOG_TARGET, "🌎️ Swarm event: {:?}", e);
            },
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn on_behaviour_event(
        &mut self,
        event: TariNodeBehaviourEvent<ProstCodec<TMsg::Message>>,
    ) -> Result<(), NetworkingError> {
        use TariNodeBehaviourEvent::*;
        #[cfg(feature = "metrics")]
        self.record_metrics(&event);
        match event {
            Ping(ping::Event {
                peer,
                connection,
                result,
            }) => match &result {
                Ok(t) => {
                    if let Some(c) = self
                        .active_connections
                        .get_mut(&peer)
                        .and_then(|c| c.iter_mut().find(|c| c.connection_id == connection))
                    {
                        c.ping_latency = Some(*t);
                        c.num_ping_failures = 0;
                    }
                    if self.config.high_ping_warning_threshold.is_some_and(|th| th < *t) {
                        warn!(target: LOG_TARGET, "🏓 Slow ping: peer={}, connection={}, t={:.2?}", peer, connection, t);
                    } else {
                        trace!(target: LOG_TARGET, "🏓 Ping: peer={}, connection={}, t={:.2?}", peer, connection, t);
                    }
                },
                // A peer that does not speak the ping protocol is not unreachable, so it must not count towards the
                // failure budget.
                Err(ping::Failure::Unsupported) => {
                    debug!(target: LOG_TARGET, "🏓 Peer {} does not support the ping protocol on connection {}", peer, connection);
                },
                Err(err) => {
                    let mut num_failures = 0;
                    if let Some(c) = self
                        .active_connections
                        .get_mut(&peer)
                        .and_then(|c| c.iter_mut().find(|c| c.connection_id == connection))
                    {
                        c.ping_latency = None;
                        c.num_ping_failures = c.num_ping_failures.saturating_add(1);
                        num_failures = c.num_ping_failures;
                    }
                    warn!(target: LOG_TARGET, "🏓 Ping failed: peer={}, connection={}, failures={}, error={}", peer, connection, num_failures, err);

                    if self
                        .config
                        .max_consecutive_ping_failures
                        .is_some_and(|max| num_failures >= max)
                    {
                        // The peer stays in the peer store, so the messaging behaviour re-dials it the next time it
                        // has something to send, trying every address held for the peer.
                        warn!(target: LOG_TARGET, "🔌 Closing unresponsive connection={} to peer={} after {} consecutive ping failures", connection, peer, num_failures);
                        self.swarm.close_connection(connection);
                    }
                },
            },
            Dcutr(dcutr::Event { remote_peer_id, result }) => match &result {
                Ok(_) => {
                    info!(target: LOG_TARGET, "📡 Dcutr successful: peer={}", remote_peer_id);
                },
                Err(err) => {
                    info!(target: LOG_TARGET, "📡 Dcutr failed: peer={}, error={}", remote_peer_id, err);
                },
            },
            Identify(identify::Event::Received {
                peer_id,
                info,
                connection_id,
            }) => {
                debug!(target: LOG_TARGET, "👋 Received identify from {} with {} addresses (id={connection_id})", peer_id, info.listen_addrs.len());
                self.on_peer_identified(connection_id, peer_id, info)?;
            },
            Identify(event) => {
                debug!(target: LOG_TARGET, "ℹ️ Identify event: {:?}", event);
            },
            RelayClient(relay::client::Event::ReservationReqAccepted {
                relay_peer_id,
                renewal,
                limit,
            }) => {
                info!(
                    "🌍️ Relay accepted our reservation request: peer_id={}, renewal={:?}, limit={:?}",
                    relay_peer_id, renewal, limit
                );
            },

            RelayClient(event) => {
                info!(target: LOG_TARGET, "🌎️ RelayClient event: {:?}", event);
            },
            Relay(event) => {
                info!(target: LOG_TARGET, "ℹ️ Relay event: {:?}", event);
            },
            Gossipsub(gossipsub::Event::Message {
                message_id,
                message,
                propagation_source,
            }) => {
                match message.source {
                    Some(source) => {
                        info!(target: LOG_TARGET, "📢 Gossipsub message: [{topic}] {message_id} ({bytes} bytes) from {source}", topic = message.topic, bytes = message.data.len());
                        self.on_gossipsub_message(message_id, propagation_source, source, message)
                            .await?;
                    },
                    None => {
                        // We accept all messages as we cannot validate them in this service.
                        // We could allow users to report back the validation result e.g. if a proposal is valid,
                        // however a naive implementation would likely incur a substantial cost
                        // for many messages.
                        if !self.swarm.behaviour_mut().gossipsub.report_message_validation_result(
                            &message_id,
                            &propagation_source,
                            gossipsub::MessageAcceptance::Ignore,
                        ) {
                            warn!(target: LOG_TARGET, "Unable to report_message_validation_result reject for topic {}, {} bytes because message was not in cache", message.topic, message.data.len());
                        }
                        warn!(target: LOG_TARGET, "📢 Discarding Gossipsub message [{topic}] ({bytes} bytes) with no source propagated by {propagation_source}", topic=message.topic, bytes=message.data.len());
                    },
                }
            },
            Gossipsub(event) => {
                info!(target: LOG_TARGET, "ℹ️ Gossipsub event: {:?}", event);
            },
            Messaging(messaging::Event::ReceivedMessage {
                peer_id,
                message,
                length,
            }) => {
                info!(target: LOG_TARGET, "📧 Rx Messaging: peer {peer_id} ({length} bytes)");
                if let Err(e) = self.messaging_mode.send_message(peer_id, message, length) {
                    warn!(target: LOG_TARGET, "📧 Inbound message dropped: {e}");
                }
            },
            Messaging(event) => {
                debug!(target: LOG_TARGET, "ℹ️ Messaging event: {:?}", event);
            },
            Substream(event) => {
                self.on_substream_event(event);
            },
            ConnectionLimits(_) => {
                // This is unreachable as connection-limits has no events
                info!(target: LOG_TARGET, "ℹ️ ConnectionLimits event");
            },
            Mdns(event) => {
                self.on_mdns_event(event)?;
            },
            Autonat(event) => {
                self.on_autonat_event(event)?;
            },
            RendezvousServer(event) => {
                info!(target: LOG_TARGET, "ℹ️ RendezvousServer event: {:?}", event);
            },
            RendezvousClient(rendezvous::client::Event::Discovered {
                rendezvous_node,
                registrations,
                cookie,
            }) => {
                info!(
                    target: LOG_TARGET,
                    "🌐 RendezvousClient {} discovered {} node(s) on namespace {}",
                    rendezvous_node,
                    registrations.len(),
                    cookie.namespace().map(|s| format!("'{s}'")).unwrap_or_else(|| "''".to_string()),
                );

                let local_peer_id = *self.swarm.local_peer_id();
                for registration in &registrations {
                    let peer_id = registration.record.peer_id();
                    if peer_id == local_peer_id {
                        continue;
                    }
                    let addresses = registration.record.addresses();
                    debug!(target: LOG_TARGET, "🌐 Discovered peer {peer_id} with {} address(es) via rendezvous", addresses.len());
                    for address in addresses {
                        if !is_supported_multiaddr(address) {
                            continue;
                        }
                        self.swarm
                            .behaviour_mut()
                            .peer_store
                            .store_mut()
                            .add_address(&peer_id, address);
                    }
                }

                self.rendezvous_state.set_cookie(&rendezvous_node, cookie);
            },
            RendezvousClient(rendezvous::client::Event::Registered {
                rendezvous_node,
                ttl,
                namespace,
            }) => {
                info!(
                    target: LOG_TARGET,
                    "🌐 Registered with rendezvous server {rendezvous_node} on namespace '{namespace}' (ttl={ttl}s). Discovering peers",
                );
                // Now that we're registered, pull the current peer list from this server.
                self.discover_rendezvous_peers(Some(rendezvous_node));
            },
            RendezvousClient(event) => {
                info!(target: LOG_TARGET, "ℹ️ RendezvousClient event: {:?}", event);
            },
            PeerStore(event) => {
                debug!(target: LOG_TARGET, "🧑‍🧑‍🧒‍🧒 PeerStore event: {:?}", event);
            },
        }

        Ok(())
    }

    async fn on_gossipsub_message(
        &mut self,
        message_id: MessageId,
        propagation_source: PeerId,
        source: PeerId,
        message: gossipsub::Message,
    ) -> Result<(), NetworkingError> {
        // Propagation is withheld until the consumer reports a verdict via
        // `NetworkingRequest::ReportGossipValidation`, so an invalid message is never relayed on our
        // behalf. If no consumer is subscribed to the topic there is nobody to produce a verdict, so
        // report `Ignore` here rather than leaving the message pinned in the validation cache.
        let topic = message.topic.clone();
        let len = message.data.len();
        if let Err(e) = self.messaging_mode.send_gossip_message(GossipMessage::new(
            source,
            propagation_source,
            message_id.clone(),
            message,
        )) {
            warn!(target: LOG_TARGET, "📢 Gossipsub message failed to be handled: {}", e);
            self.report_gossip_validation(
                &message_id,
                &propagation_source,
                gossipsub::MessageAcceptance::Ignore,
                &topic,
                len,
            );
        }
        Ok(())
    }

    fn report_gossip_validation(
        &mut self,
        message_id: &gossipsub::MessageId,
        propagation_source: &PeerId,
        acceptance: gossipsub::MessageAcceptance,
        topic: &gossipsub::TopicHash,
        len: usize,
    ) {
        if !self.swarm.behaviour_mut().gossipsub.report_message_validation_result(
            message_id,
            propagation_source,
            acceptance,
        ) {
            warn!(target: LOG_TARGET, "Unable to report gossip validation result for topic {topic}, {len} bytes because message was not in cache");
        }
    }

    #[cfg(feature = "metrics")]
    fn record_metrics(&mut self, event: &TariNodeBehaviourEvent<ProstCodec<TMsg::Message>>) {
        use tari_swarm::metrics::Recorder;
        if let Some(metrics) = self.metrics.as_ref() {
            metrics.record(event);
        }
    }

    fn on_mdns_event(&mut self, event: mdns::Event) -> Result<(), NetworkingError> {
        match event {
            mdns::Event::Discovered(peers_and_addrs) => {
                for (peer, addr) in peers_and_addrs {
                    debug!(target: LOG_TARGET, "📡 mDNS discovered peer {} at {}", peer, addr);
                    // Persist the address so a later peer-id-only dial (e.g. an RPC session) can
                    // resolve it through the peer store, rather than only succeeding while the
                    // connection opened below is still live.
                    self.swarm
                        .behaviour_mut()
                        .peer_store
                        .store_mut()
                        .add_address(&peer, &addr);
                    self.swarm
                        .dial(
                            DialOpts::peer_id(peer)
                                .addresses(vec![addr])
                                .extend_addresses_through_behaviour()
                                .build(),
                        )
                        .or_else(|err| {
                            // Peer already has pending dial or established connection - OK
                            if matches!(&err, DialError::DialPeerConditionFalse(_)) {
                                Ok(())
                            } else {
                                Err(err)
                            }
                        })?;
                }
            },
            mdns::Event::Expired(addrs_list) => {
                for (peer_id, multiaddr) in addrs_list {
                    debug!(target: LOG_TARGET, "MDNS got expired peer with ID: {peer_id:#?} and Address: {multiaddr:#?}");
                }
            },
        }
        Ok(())
    }

    fn on_autonat_event(&mut self, event: autonat::Event) -> Result<(), NetworkingError> {
        use autonat::Event::*;
        match event {
            StatusChanged { old, new } => {
                // If we are/were "Private", let's establish a relay reservation with a known relay
                if (self.config.reachability_mode.is_private() ||
                    new == NatStatus::Private ||
                    old == NatStatus::Private) &&
                    !self.relays.has_active_relay()
                {
                    info!(target: LOG_TARGET, "🌍️ Reachability status changed to Private. Dialing relay");
                    self.attempt_relay_reservation();
                }

                info!(target: LOG_TARGET, "🌍️ Autonat status changed from {:?} to {:?}", old, new);
            },
            _ => {
                debug!(target: LOG_TARGET, "🌍️ Autonat event: {:?}", event);
            },
        }

        Ok(())
    }

    fn attempt_relay_reservation(&mut self) {
        self.relays.select_random_relay();
        if let Some(relay) = self.relays.selected_relay() &&
            let Err(err) = self.swarm.dial(
                DialOpts::peer_id(relay.peer_id)
                    .addresses(relay.addresses.clone())
                    .condition(PeerCondition::NotDialing)
                    .extend_addresses_through_behaviour()
                    .build(),
            )
        {
            if is_dial_error_caused_by_remote(&err) {
                self.relays.clear_selected_relay();
            }
            warn!(target: LOG_TARGET, "🚨 Failed to dial relay: {}", err);
        }
    }

    fn on_connection_established(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        endpoint: ConnectedPoint,
        num_established: u32,
        num_concurrent_dial_errors: usize,
        established_in: Duration,
    ) -> Result<(), NetworkingError> {
        debug!(
            target: LOG_TARGET,
            "🤝 Connection established: peer_id={}, connection_id={}, endpoint={:?}, num_established={}, \
             concurrent_dial_errors={}, established_in={:?}",
            peer_id,
            connection_id,
            endpoint,
            num_established,
            num_concurrent_dial_errors,
            established_in
        );

        if let Some(relay) = self.relays.selected_relay_mut() &&
            endpoint.is_dialer() &&
            relay.peer_id == peer_id
        {
            relay.remote_address = Some(endpoint.get_remote_address().clone());
        }

        self.active_connections.entry(peer_id).or_default().push(Connection {
            connection_id,
            peer_id,
            created_at: Instant::now(),
            endpoint,
            num_established,
            num_concurrent_dial_errors,
            established_in,
            ping_latency: None,
            num_ping_failures: 0,
            user_agent: None,
        });

        let Some(waiters) = self.pending_dial_requests.remove(&peer_id) else {
            debug!(target: LOG_TARGET, "No pending dial requests initiated by this service for peer {}", peer_id);
            return Ok(());
        };

        for waiter in waiters {
            let _ignore = waiter.send(Ok(()));
        }

        Ok(())
    }

    fn on_peer_identified(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        info: identify::Info,
    ) -> Result<(), NetworkingError> {
        if !self.config.swarm.protocol_version.is_compatible(&info.protocol_version) {
            info!(target: LOG_TARGET, "🚨 Peer {} is using an incompatible protocol version: {}. Our version {}", peer_id, info.protocol_version, self.config.swarm.protocol_version);
            // Error can be ignored as the docs indicate that an error only occurs if there was no connection to the
            // peer.
            let _ignore = self.swarm.disconnect_peer_id(peer_id);
            return Ok(());
        }

        // Not sure if this can happen but just in case
        if *self.swarm.local_peer_id() == peer_id {
            warn!(target: LOG_TARGET, "Dialled ourselves");
            return Ok(());
        }

        if self.rendezvous_state.contains(&peer_id) {
            info!(target: LOG_TARGET, "Connected to rendezvous server {}. Registering", peer_id);
            let ns = self.get_rendezvous_namespace();
            if let Err(err) = self
                .swarm
                .behaviour_mut()
                .rendezvous_client
                // Default Ttl of 10 hours
                .register(ns, peer_id, Some(10 * 60 * 20))
            {
                warn!(target: LOG_TARGET, "Failed to register with rendezvous server {}: {}", peer_id, err);
            }
        }

        self.update_connected_peers(&peer_id, &info);

        let is_relay = info.protocols.contains(&protocol_ids::RELAY_HOP_PROTOCOL);

        let is_connected_through_relay = self
            .active_connections
            .get(&peer_id)
            .map(|conns| {
                conns
                    .iter()
                    .any(|c| c.endpoint.is_dialer() && is_through_relay_address(c.endpoint.get_remote_address()))
            })
            .unwrap_or(false);

        for address in info.listen_addrs {
            if is_p2p_address(&address) && address.is_global_ip() {
                // If the peer has a p2p-circuit address, immediately upgrade to a direct connection (DCUtR /
                // hole-punching) if we're connected to them through a relay
                if is_connected_through_relay {
                    info!(target: LOG_TARGET, "📡 Peer {} has a p2p-circuit address. Upgrading to DCUtR", peer_id);
                    // Ignore as connection failures are logged in events, or an error here is because the peer is
                    // already connected/being dialed
                    let _ignore = self
                        .swarm
                        .dial(DialOpts::peer_id(peer_id).addresses(vec![address.clone()]).build());
                } else if is_relay && !is_through_relay_address(&address) {
                    // Otherwise, if the peer advertises as a relay we'll add them
                    info!(target: LOG_TARGET, "📡 Adding peer {peer_id} {address} as a relay");
                    self.relays.add_possible_relay(peer_id, address.clone());
                    if !self.relays.has_active_relay() {
                        self.relays.set_relay_peer(peer_id, Some(address.clone()));
                    }
                } else {
                    // Nothing to do
                }
            }
        }

        // If this peer is the selected relay that was dialed previously, listen on the circuit address
        // Note we only select a relay if autonat says we are not publicly accessible.
        if is_relay {
            self.establish_relay_circuit_on_connect(&peer_id, connection_id);
        }

        self.publish_event(NetworkingEvent::NewIdentifiedPeer {
            peer_id,
            public_key: info.public_key,
            supported_protocols: info.protocols,
        });
        Ok(())
    }

    fn update_connected_peers(&mut self, peer_id: &PeerId, info: &identify::Info) {
        let Some(conns_mut) = self.active_connections.get_mut(peer_id) else {
            return;
        };

        let user_agent = Arc::new(info.agent_version.clone());
        for conn_mut in conns_mut {
            conn_mut.user_agent = Some(user_agent.clone());
        }
    }

    /// Establishes a relay circuit for the given peer if it is the selected relay peer. Returns true if the circuit
    /// was established from this call.
    fn establish_relay_circuit_on_connect(&mut self, peer_id: &PeerId, connection_id: ConnectionId) -> bool {
        let Some(relay) = self.relays.selected_relay() else {
            return false;
        };

        // If the peer we've connected with is the selected relay that we previously dialed, then continue
        if relay.peer_id != *peer_id {
            return false;
        }

        // If we've already established a circuit with the relay, there's nothing to do here
        if relay.has_circuit() {
            return false;
        }

        // Check if we've got a confirmed address for the relay
        let Some(remote_address) = relay.remote_address.as_ref() else {
            return false;
        };

        let circuit_addr = remote_address.clone().with(Protocol::P2pCircuit);

        match self.swarm.listen_on(circuit_addr.clone()) {
            Ok(id) => {
                info!(target: LOG_TARGET, "🌍️ Peer {peer_id} is a relay. Listening (id={id:?}) for circuit connections");
                let Some(relay_mut) = self.relays.selected_relay_mut() else {
                    // unreachable
                    return false;
                };
                relay_mut.circuit_connection_id = Some(connection_id);
                true
            },
            Err(e) => {
                // failed to establish a circuit, reset to try another relay
                self.relays.clear_selected_relay();
                error!(target: LOG_TARGET, "Local node failed to listen on relay address. Error: {e}");
                false
            },
        }
    }

    fn on_substream_event(&mut self, event: substream::Event) {
        use substream::Event::*;
        match event {
            SubstreamOpen {
                peer_id,
                stream_id,
                stream,
                protocol,
            } => {
                info!(target: LOG_TARGET, "📥 substream open: peer_id={}, stream_id={}, protocol={}", peer_id, stream_id, protocol);
                let Some(reply) = self.pending_substream_requests.remove(&stream_id) else {
                    debug!(target: LOG_TARGET, "No pending requests for subtream protocol {protocol} for peer {peer_id}");
                    return;
                };
                shrink_hashmap_if_required(&mut self.pending_substream_requests);

                let _ignore = reply.send(Ok(NegotiatedSubstream::new(peer_id, protocol, stream)));
            },
            InboundSubstreamOpen { notification } => {
                debug!(target: LOG_TARGET, "📥 Inbound substream open: protocol={}", notification.protocol);
                self.substream_notifiers.notify(notification);
            },
            InboundFailure {
                peer_id,
                stream_id,
                error,
            } => {
                debug!(target: LOG_TARGET, "Inbound substream failed from peer {peer_id} with stream id {stream_id}: {error}");
                if let Some(waiting_reply) = self.pending_substream_requests.remove(&stream_id) {
                    let _ignore = waiting_reply.send(Err(NetworkingError::FailedToOpenSubstream(error)));
                }
            },
            OutboundFailure {
                error,
                stream_id,
                peer_id,
                ..
            } => {
                debug!(target: LOG_TARGET, "Outbound substream failed with peer {peer_id}, stream {stream_id}: {error}");
                if let Some(waiting_reply) = self.pending_substream_requests.remove(&stream_id) {
                    let _ignore = waiting_reply.send(Err(NetworkingError::FailedToOpenSubstream(error)));
                }
            },
        }
    }

    fn publish_event(&mut self, event: NetworkingEvent) {
        if let Ok(num) = self.tx_events.send(event) {
            debug!(target: LOG_TARGET, "📢 Published networking event to {num} subscribers");
        }
    }

    fn get_rendezvous_namespace(&self) -> Namespace {
        // Panic: namespace MUST be less than 255 characters
        Namespace::new(self.config.rendezvous_namespace.clone()).expect("configuration error")
    }

    /// Requests the peer list registered at our rendezvous servers (the seed peers). Pass `only` to query a single
    /// server, or `None` to query all of them. Discovered registrations are handled in the
    /// [`RendezvousClient(Discovered)`] event. No-op if no rendezvous servers are configured.
    ///
    /// This is best-effort: a seed peer that does not run a rendezvous server simply yields a harmless failed request.
    fn discover_rendezvous_peers(&mut self, only: Option<PeerId>) {
        if self.rendezvous_state.is_empty() {
            return;
        }
        let servers = self
            .rendezvous_state
            .servers()
            .filter(|(peer_id, ..)| only.is_none_or(|o| o == *peer_id))
            .collect::<Vec<_>>();
        let ns = self.get_rendezvous_namespace();
        for (peer_id, addr, cookie) in servers {
            // Rendezvous servers are dialed at bootstrap, but if a connection has since dropped (e.g. idle timeout)
            // re-dial it so the discovery request can be delivered.
            if !self.active_connections.contains_key(&peer_id) {
                debug!(target: LOG_TARGET, "🌐 Not connected to rendezvous server {peer_id}. Dialing");
                let opts = DialOpts::peer_id(peer_id)
                    .addresses(vec![addr])
                    .condition(PeerCondition::DisconnectedAndNotDialing)
                    .extend_addresses_through_behaviour()
                    .build();
                if let Err(err) = self.swarm.dial(opts) &&
                    !matches!(&err, DialError::DialPeerConditionFalse(_))
                {
                    warn!(target: LOG_TARGET, "🚨 Failed to dial rendezvous server {peer_id}: {err}");
                }
            }

            self.swarm.behaviour_mut().rendezvous_client.discover(
                Some(ns.clone()),
                cookie,
                self.config.rendezvous_peer_limit,
                peer_id,
            );
        }
    }
}

fn is_p2p_address(address: &Multiaddr) -> bool {
    address.iter().any(|p| matches!(p, Protocol::P2p(_)))
}

fn is_through_relay_address(address: &Multiaddr) -> bool {
    let mut found_p2p_circuit = false;
    for protocol in address {
        if !found_p2p_circuit {
            if let Protocol::P2pCircuit = protocol {
                found_p2p_circuit = true;
                continue;
            }
            continue;
        }
        // Once we found a p2p-circuit protocol, this is followed by /p2p/<peer_id>
        return matches!(protocol, Protocol::P2p(_));
    }

    false
}

fn is_dial_error_caused_by_remote(err: &DialError) -> bool {
    !matches!(
        err,
        DialError::DialPeerConditionFalse(_) | DialError::Aborted | DialError::LocalPeerId { .. }
    )
}

fn shrink_hashmap_if_required<K, V>(map: &mut HashMap<K, V>)
where K: Eq + Hash {
    const HASHMAP_EXCESS_ENTRIES_SHRINK_THRESHOLD: usize = 50;
    if map.len() + HASHMAP_EXCESS_ENTRIES_SHRINK_THRESHOLD < map.capacity() {
        map.shrink_to_fit();
    }
}
