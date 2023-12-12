//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    hash::Hash,
    time::{Duration, Instant},
};

use libp2p::{
    autonat,
    autonat::NatStatus,
    core::ConnectedPoint,
    dcutr,
    futures::StreamExt,
    gossipsub,
    identify,
    kad,
    kad::{QueryResult, RoutingUpdate},
    mdns,
    multiaddr::Protocol,
    ping,
    relay,
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        ConnectionId,
        DialError,
        SwarmEvent,
    },
    Multiaddr,
    PeerId,
    StreamProtocol,
};
use log::*;
use tari_rpc_framework::Substream;
use tari_shutdown::ShutdownSignal;
use tari_swarm::{
    messaging,
    messaging::Codec,
    substream,
    substream::{NegotiatedSubstream, ProtocolNotification, StreamId},
    ProtocolVersion,
    TariNodeBehaviourEvent,
    TariSwarm,
};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time,
};

use crate::{
    connection::Connection,
    event::NetworkingEvent,
    global_ip::GlobalIp,
    handle::NetworkingRequest,
    notify::Notifiers,
    relay_state::RelayState,
    NetworkingError,
};

const LOG_TARGET: &str = "tari::dan::networking::service::worker";

type ReplyTx<T> = oneshot::Sender<Result<T, NetworkingError>>;

pub struct NetworkingWorker<TCodec>
where TCodec: Codec + Send + Clone + 'static
{
    rx_request: mpsc::Receiver<NetworkingRequest<TCodec::Message>>,
    tx_events: broadcast::Sender<NetworkingEvent>,
    tx_messages: mpsc::Sender<(PeerId, TCodec::Message)>,
    active_connections: HashMap<PeerId, Vec<Connection>>,
    pending_substream_requests: HashMap<StreamId, ReplyTx<NegotiatedSubstream<Substream>>>,
    pending_dial_requests: HashMap<PeerId, Vec<ReplyTx<()>>>,
    codec: TCodec,
    substream_notifiers: Notifiers<Substream>,
    swarm: TariSwarm<TCodec>,
    config: crate::Config,
    relays: RelayState,
    is_initial_bootstrap_complete: bool,
    shutdown_signal: ShutdownSignal,
}

impl<TCodec> NetworkingWorker<TCodec>
where
    TCodec: Codec + Send + Clone + 'static,
    TCodec::Message: Clone,
{
    pub(crate) fn new(
        rx_request: mpsc::Receiver<NetworkingRequest<TCodec::Message>>,
        tx_events: broadcast::Sender<NetworkingEvent>,
        tx_messages: mpsc::Sender<(PeerId, TCodec::Message)>,
        swarm: TariSwarm<TCodec>,
        config: crate::Config,
        known_relay_nodes: Vec<(PeerId, Multiaddr)>,
        shutdown_signal: ShutdownSignal,
    ) -> Self {
        Self {
            rx_request,
            tx_events,
            tx_messages,
            substream_notifiers: Notifiers::new(),
            active_connections: HashMap::new(),
            pending_substream_requests: HashMap::new(),
            pending_dial_requests: HashMap::new(),
            codec: TCodec::default(),
            relays: RelayState::new(known_relay_nodes),
            swarm,
            config,
            is_initial_bootstrap_complete: false,
            shutdown_signal,
        }
    }

    pub fn add_protocol_notifier(
        &mut self,
        protocol: StreamProtocol,
        sender: mpsc::UnboundedSender<ProtocolNotification<Substream>>,
    ) {
        self.substream_notifiers.add(protocol, sender);
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        debug!(target: LOG_TARGET, "🌐 Starting networking service {:?}", self.config);
        // Listen on all interfaces TODO: Configure
        self.swarm.listen_on(
            format!("/ip4/0.0.0.0/tcp/{}", self.config.listener_port)
                .parse()
                .unwrap(),
        )?;
        self.swarm.listen_on(
            format!("/ip4/0.0.0.0/udp/{}/quic-v1", self.config.listener_port)
                .parse()
                .unwrap(),
        )?;

        if self.config.reachability_mode.is_private() {
            self.attempt_relay_reservation();
        }

        let mut bootstrap_interval = time::interval(Duration::from_secs(3600));

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
                _ =  bootstrap_interval.tick() => {
                    if let Err(err) = self.bootstrap_kad() {
                        error!(target: LOG_TARGET, "🚨 Failed to bootstrap Kademlia: {}", err);
                    }
                },

                _ = self.shutdown_signal.wait() => {
                    break;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_request(&mut self, request: NetworkingRequest<TCodec::Message>) -> Result<(), NetworkingError> {
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
                        if let Some(peer_id) = maybe_peer_id {
                            if matches!(err, DialError::NoAddresses) {
                                self.swarm.behaviour_mut().kad.get_closest_peers(peer_id);
                            }
                        }
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
                    .enqueue_message(peer, message)
                    .await
                {
                    Ok(_) => {
                        debug!(target: LOG_TARGET, "📢 Queued message to peer {}", peer);
                        let _ignore = reply_tx.send(Ok(()));
                    },
                    Err(err) => {
                        debug!(target: LOG_TARGET, "🚨 Failed to queue message to peer {}: {}", peer, err);
                        let _ignore = reply_tx.send(Err(err.into()));
                    },
                }
            },
            NetworkingRequest::SendMulticast {
                destination,
                message,
                reply_tx,
            } => {
                let len = destination.len();
                let messaging_mut = &mut self.swarm.behaviour_mut().messaging;

                for peer in destination {
                    match messaging_mut.enqueue_message(peer, message.clone()).await {
                        Ok(_) => {},
                        Err(err) => {
                            debug!(target: LOG_TARGET, "🚨 Failed to queue message to peer {}: {}", peer, err);
                        },
                    }
                }
                debug!(target: LOG_TARGET, "📢 Queued message to {} peers", len);
                let _ignore = reply_tx.send(Ok(()));
            },
            NetworkingRequest::PublishGossip {
                topic,
                message,
                reply_tx,
            } => {
                let mut buf = Vec::with_capacity(1024);
                self.codec
                    .encode_to(&mut buf, message)
                    .await
                    .map_err(NetworkingError::CodecError)?;
                match self.swarm.behaviour_mut().gossipsub.publish(topic, buf) {
                    Ok(msg_id) => {
                        debug!(target: LOG_TARGET, "📢 Published gossipsub message: {}", msg_id);
                        let _ignore = reply_tx.send(Ok(()));
                    },
                    Err(err) => {
                        debug!(target: LOG_TARGET, "🚨 Failed to publish gossipsub message: {}", err);
                        let _ignore = reply_tx.send(Err(err.into()));
                    },
                }
            },
            NetworkingRequest::SubscribeTopic { topic, reply_tx } => {
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
                match self.swarm.behaviour_mut().gossipsub.unsubscribe(&topic) {
                    Ok(_) => {
                        debug!(target: LOG_TARGET, "📢 Unsubscribed from gossipsub topic: {}", topic);
                        let _ignore = reply_tx.send(Ok(()));
                    },
                    Err(err) => {
                        error!(target: LOG_TARGET, "🚨 Failed to unsubscribe from gossipsub topic: {}", err);
                        let _ignore = reply_tx.send(Err(err.into()));
                    },
                }
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
        }

        Ok(())
    }

    fn bootstrap_kad(&mut self) -> Result<(), NetworkingError> {
        if self.is_initial_bootstrap_complete {
            info!(target: LOG_TARGET, "🥾 Bootstrapping kad");
            // If there are no seed peers this will error
            let _ignore = self.swarm.behaviour_mut().kad.bootstrap();
        } else {
            let mut has_seed_peers = false;
            info!(target: LOG_TARGET, "🥾 Bootstrapping kad with {} known relay peers", self.relays.num_possible_relays());

            let local_peer_id = *self.swarm.local_peer_id();
            for (peer_id, addresses) in self.relays.possible_relays() {
                if *peer_id == local_peer_id {
                    continue;
                }
                has_seed_peers = true;

                for address in addresses {
                    self.swarm.behaviour_mut().kad.add_address(peer_id, address.clone());
                }
            }
            if has_seed_peers {
                self.swarm.behaviour_mut().kad.bootstrap()?;
            }
        }

        Ok(())
    }

    async fn on_swarm_event(
        &mut self,
        event: SwarmEvent<TariNodeBehaviourEvent<TCodec>>,
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
                ..
            } => {
                info!(target: LOG_TARGET, "🔌 Connection closed: peer_id={}, endpoint={:?}, cause={:?}", peer_id, endpoint, cause);
                match self.active_connections.entry(peer_id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().retain(|c| c.endpoint != endpoint);
                        if entry.get().is_empty() {
                            entry.remove_entry();
                        }
                    },
                    Entry::Vacant(_) => {
                        debug!(target: LOG_TARGET, "Connection closed for peer {peer_id} but this connection is not in the active connections list");
                    },
                }
                check_and_shrink_hashmap(&mut self.active_connections);
            },
            SwarmEvent::OutgoingConnectionError {
                peer_id: Some(peer_id),
                error,
                ..
            } => {
                let Some(waiters) = self.pending_dial_requests.remove(&peer_id) else {
                    debug!(target: LOG_TARGET, "No pending dial requests initiated by this service for peer {}", peer_id);
                    return Ok(());
                };
                check_and_shrink_hashmap(&mut self.pending_dial_requests);

                for waiter in waiters {
                    let _ignore = waiter.send(Err(NetworkingError::OutgoingConnectionError(error.to_string())));
                }
            },
            SwarmEvent::ExternalAddrConfirmed { address } => {
                info!(target: LOG_TARGET, "🌍️ External address confirmed: {}", address);
            },
            SwarmEvent::Dialing { peer_id, connection_id } => {
                if let Some(peer_id) = peer_id {
                    info!(target: LOG_TARGET, "🤝 Dialing peer {peer_id} for connection({connection_id})");
                    // TODO: This helps us discover peers, however it seems like a better strategy should be found
                    if !self.active_connections.contains_key(&peer_id) {
                        self.swarm.behaviour_mut().kad.get_closest_peers(peer_id);
                    }
                }
            },
            e => {
                debug!(target: LOG_TARGET, "🌎️ Swarm event: {:?}", e);
            },
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn on_behaviour_event(&mut self, event: TariNodeBehaviourEvent<TCodec>) -> Result<(), NetworkingError> {
        use TariNodeBehaviourEvent::*;
        match event {
            Ping(ping::Event {
                peer,
                connection,
                result,
            }) => match result {
                Ok(t) => {
                    if let Some(c) = self
                        .active_connections
                        .get_mut(&peer)
                        .and_then(|c| c.iter_mut().find(|c| c.connection_id == connection))
                    {
                        c.ping_latency = Some(t);
                    }
                    debug!(target: LOG_TARGET, "🏓 Ping: peer={}, connection={}, t={:.2?}", peer, connection, t);
                },
                Err(err) => {
                    warn!(target: LOG_TARGET, "🏓 Ping failed: peer={}, connection={}, error={}", peer, connection, err);
                },
            },
            Dcutr(dcutr::Event { remote_peer_id, result }) => match result {
                Ok(_) => {
                    info!(target: LOG_TARGET, "📡 Dcutr successful: peer={}", remote_peer_id);
                },
                Err(err) => {
                    info!(target: LOG_TARGET, "📡 Dcutr failed: peer={}, error={}", remote_peer_id, err);
                },
            },
            Identify(identify::Event::Received { peer_id, info }) => {
                info!(target: LOG_TARGET, "👋 Received identify from {} with {} addresses", peer_id, info.listen_addrs.len());
                self.on_peer_identified(peer_id, info)?;
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
            }) => match message.source {
                Some(source) => {
                    info!(target: LOG_TARGET, "📢 Gossipsub message: [{topic}] {message_id} ({bytes} bytes) from {source}", topic = message.topic, bytes = message.data.len());
                    let msg = self
                        .codec
                        .decode_from(&mut message.data.as_slice())
                        .await
                        .map_err(NetworkingError::CodecError)?;
                    let _ignore = self.tx_messages.send((source, msg)).await;
                },
                None => {
                    warn!(target: LOG_TARGET, "📢 Discarding Gossipsub message [{topic}] ({bytes} bytes) with no source propagated by {propagation_source}", topic=message.topic, bytes=message.data.len());
                },
            },
            Gossipsub(event) => {
                info!(target: LOG_TARGET, "ℹ️ Gossipsub event: {:?}", event);
            },
            Messaging(messaging::Event::ReceivedMessage { peer_id, message }) => {
                info!(target: LOG_TARGET, "📧 Messaging: received message from peer {peer_id}");
                let _ignore = self.tx_messages.send((peer_id, message)).await;
            },
            Messaging(event) => {
                debug!(target: LOG_TARGET, "ℹ️ Messaging event: {:?}", event);
            },
            Kad(kad::Event::OutboundQueryProgressed {
                id,
                result,
                stats,
                step,
            }) => {
                debug!(target: LOG_TARGET, "🧭 Kad outbound query progressed: id={}, result={:?}, stats={:?}, step={:?}", id, result, stats, step);
                match result {
                    QueryResult::Bootstrap(b) => match b {
                        Ok(ok) => {
                            if ok.num_remaining == 0 {
                                info!(target: LOG_TARGET, "🧭 Kad bootstrap complete");
                                self.is_initial_bootstrap_complete = true;
                            }
                        },
                        Err(err) => {
                            info!(target: LOG_TARGET, "🧭 Kad bootstrap failed: {}", err);
                        },
                    },
                    QueryResult::GetClosestPeers(Ok(ok)) => {
                        info!(target: LOG_TARGET, "🧭 Kad get closest peers: {:?}", ok);
                    },
                    QueryResult::GetClosestPeers(Err(err)) => {
                        info!(target: LOG_TARGET, "🧭 Kad get closest peers failed: {}", err);
                    },
                    q => {
                        info!(target: LOG_TARGET, "🧭 Kad result: {q:?}");
                    },
                }
            },
            Kad(event) => {
                debug!(target: LOG_TARGET, "🧭 Kad event: {:?}", event);
            },

            Substream(event) => {
                info!(target: LOG_TARGET, "ℹ️ Substream event: {:?}", event);
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
        }

        Ok(())
    }

    fn on_mdns_event(&mut self, event: mdns::Event) -> Result<(), NetworkingError> {
        match event {
            mdns::Event::Discovered(peers_and_addrs) => {
                let mut unique_discovered_peers = HashSet::new();
                for (peer, addr) in peers_and_addrs {
                    info!(target: LOG_TARGET, "📡 mDNS discovered peer {} at {}", peer, addr);
                    let route_update = self.swarm.behaviour_mut().kad.add_address(&peer, addr);
                    debug!(target: LOG_TARGET, "🧭 kad route update: {:?}", route_update);
                    unique_discovered_peers.insert(peer);
                }
                for peer in unique_discovered_peers {
                    self.swarm.dial(peer).or_else(|err| {
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

                    // Ensure that the peer was previously added by mDNS
                    if self
                        .swarm
                        .behaviour()
                        .mdns
                        .as_ref()
                        .expect("mdns is enabled")
                        .discovered_nodes()
                        .any(|&p| p == peer_id)
                    {
                        self.swarm.behaviour_mut().kad.remove_address(&peer_id, &multiaddr);
                    }
                }
            },
        }
        Ok(())
    }

    fn on_autonat_event(&mut self, event: autonat::Event) -> Result<(), NetworkingError> {
        use autonat::Event::*;
        match event {
            StatusChanged { old, new } => {
                if let Some(public_address) = self.swarm.behaviour().autonat.public_address() {
                    info!(target: LOG_TARGET, "🌍️ Autonat: Our public address is {public_address}");
                }

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
                info!(target: LOG_TARGET, "🌍️ Autonat event: {:?}", event);
            },
        }

        Ok(())
    }

    fn attempt_relay_reservation(&mut self) {
        self.relays.select_random_relay();
        if let Some(relay) = self.relays.selected_relay() {
            if let Err(err) = self.swarm.dial(
                DialOpts::peer_id(relay.peer_id)
                    .addresses(relay.addresses.clone())
                    .condition(PeerCondition::NotDialing)
                    .build(),
            ) {
                if is_dial_error_caused_by_remote(&err) {
                    self.relays.clear_selected_relay();
                }
                warn!(target: LOG_TARGET, "🚨 Failed to dial relay: {}", err);
            }
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

        if let Some(relay) = self.relays.selected_relay_mut() {
            if endpoint.is_dialer() && relay.peer_id == peer_id {
                relay.dialled_address = Some(endpoint.get_remote_address().clone());
            }
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

    fn on_peer_identified(&mut self, peer_id: PeerId, info: identify::Info) -> Result<(), NetworkingError> {
        if !self
            .config
            .swarm
            .protocol_version
            .is_compatible(&ProtocolVersion::try_from(info.protocol_version.as_str())?)
        {
            info!(target: LOG_TARGET, "🚨 Peer {} is using an incompatible protocol version: {}. Our version {}", peer_id, info.protocol_version, self.config.swarm.protocol_version);
            // Errors just indicate that there was no connection to the peer.
            let _ignore = self.swarm.disconnect_peer_id(peer_id);
            self.swarm.behaviour_mut().kad.remove_peer(&peer_id);
            return Ok(());
        }

        // Not sure if this can happen but just in case
        if *self.swarm.local_peer_id() == peer_id {
            warn!(target: LOG_TARGET, "Dialled ourselves");
            return Ok(());
        }

        let is_relay = info.protocols.iter().any(|p| *p == relay::HOP_PROTOCOL_NAME);

        for address in info.listen_addrs {
            if is_p2p_address(&address) {
                if address.is_global_ip() {
                    // If the peer has a p2p-circuit address, immediately upgrade to a direct connection (DCUtR /
                    // hole-punching)
                    if is_p2p_circuit(&address) {
                        info!(target: LOG_TARGET, "📡 Peer {} has a p2p-circuit address. Upgrading to DCUtR", peer_id);
                        // Ignore as connection failures are logged in events, or an error here is because the peer is
                        // already connected/being dialled
                        let _ignore = self
                            .swarm
                            .dial(DialOpts::peer_id(peer_id).addresses(vec![address.clone()]).build());
                    } else if is_relay {
                        // Otherwise, if the peer advertises as a relay we'll add them
                        info!(target: LOG_TARGET, "📡 Adding peer {peer_id} {address} as a relay");
                        self.relays.add_possible_relay(peer_id, address.clone());
                    } else {
                        // Nothing to do
                    }
                }

                match self.swarm.behaviour_mut().kad.add_address(&peer_id, address) {
                    RoutingUpdate::Success | RoutingUpdate::Pending => {},
                    RoutingUpdate::Failed => {
                        info!(target: LOG_TARGET, "🚨 Failed to add address to Kademlia for peer {}", peer_id);
                    },
                }
            }
        }

        // If this peer is the selected relay that was dialled previously, listen on the circuit address
        // Note we only select a relay if autonat says we are not publicly accessible.
        if is_relay {
            self.establish_relay_circuit_on_connect(&peer_id);
        }

        self.publish_event(NetworkingEvent::NewIdentifiedPeer {
            peer_id,
            public_key: info.public_key,
            supported_protocols: info.protocols,
        });
        Ok(())
    }

    /// Establishes a relay circuit for the given peer if the it is the selected relay peer. Returns true if the circuit
    /// was established from this call.
    fn establish_relay_circuit_on_connect(&mut self, peer_id: &PeerId) -> bool {
        let Some(relay) = self.relays.selected_relay() else {
            return false;
        };

        // If the peer we've connected with is the selected relay that we previously dialled, then continue
        if relay.peer_id != *peer_id {
            return false;
        }

        // If we've already established a circuit with the relay, there's nothing to do here
        if relay.is_circuit_established {
            return false;
        }

        // Check if we've got a confirmed address for the relay
        let Some(dialled_address) = relay.dialled_address.as_ref() else {
            return false;
        };

        match self.swarm.listen_on(dialled_address.clone().with(Protocol::P2pCircuit)) {
            Ok(id) => {
                info!(target: LOG_TARGET, "🌍️ Peer {peer_id} is a relay. Listening (id={id:?}) for circuit connections");
                let Some(relay_mut) = self.relays.selected_relay_mut() else {
                    // unreachable
                    return false;
                };
                relay_mut.is_circuit_established = true;
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
                let Some(reply) = self.pending_substream_requests.remove(&stream_id) else {
                    debug!(target: LOG_TARGET, "No pending requests for subtream protocol {protocol} for peer {peer_id}");
                    return;
                };
                check_and_shrink_hashmap(&mut self.pending_substream_requests);

                let _ignore = reply.send(Ok(NegotiatedSubstream::new(peer_id, protocol, stream)));
            },
            InboundSubstreamOpen { notification } => {
                self.substream_notifiers.notify(notification);
            },
            InboundFailure {
                peer_id,
                stream_id,
                error,
            } => {
                debug!(target: LOG_TARGET, "Inbound substream failed from peer {peer_id} with stream id {stream_id}: {error}");
            },
            OutboundFailure {
                error,
                stream_id,
                peer_id,
                ..
            } => {
                if matches!(&error, substream::Error::NoAddressesForPeer) {
                    self.swarm.behaviour_mut().kad.get_closest_peers(peer_id);
                }
                if let Some(waiting_reply) = self.pending_substream_requests.remove(&stream_id) {
                    let _ignore = waiting_reply.send(Err(NetworkingError::FailedToOpenSubstream(error)));
                }
            },
            Error(_) => {},
        }
    }

    fn publish_event(&mut self, event: NetworkingEvent) {
        if let Ok(num) = self.tx_events.send(event) {
            debug!(target: LOG_TARGET, "📢 Published networking event to {num} subscribers");
        }
    }
}

fn is_p2p_address(address: &Multiaddr) -> bool {
    address.iter().any(|p| matches!(p, Protocol::P2p(_)))
}

fn is_p2p_circuit(address: &Multiaddr) -> bool {
    address.iter().any(|p| matches!(p, Protocol::P2pCircuit))
}

fn is_dial_error_caused_by_remote(err: &DialError) -> bool {
    !matches!(
        err,
        DialError::DialPeerConditionFalse(_) | DialError::Aborted | DialError::LocalPeerId { .. }
    )
}

fn check_and_shrink_hashmap<K, V>(map: &mut HashMap<K, V>)
where K: Eq + Hash {
    const HASHMAP_EXCESS_ENTRIES_SHRINK_THRESHOLD: usize = 50;
    if map.len() + HASHMAP_EXCESS_ENTRIES_SHRINK_THRESHOLD < map.capacity() {
        map.shrink_to_fit();
    }
}
