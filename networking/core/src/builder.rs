//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use anyhow::anyhow;
use libp2p::{Multiaddr, PeerId, identity::Keypair};
use tari_shutdown::ShutdownSignal;
use tari_swarm::{is_supported_multiaddr, messaging, messaging::BytesCodec};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::{MessagingMode, NetworkingHandle, message::MessageSpec, worker::NetworkingWorker};

pub struct Builder<'a, TMsg: MessageSpec> {
    identity: Keypair,
    messaging_mode: MessagingMode,
    config: crate::Config,
    seed_peers: Vec<(Option<PeerId>, Multiaddr)>,
    #[cfg(feature = "metrics")]
    registry: Option<&'a mut libp2p::metrics::Registry>,
    #[cfg(not(feature = "metrics"))]
    _marker: std::marker::PhantomData<&'a ()>,
    _spec: std::marker::PhantomData<TMsg>,
}

impl<'a, TMsg: MessageSpec> Builder<'a, TMsg>
where
    TMsg: MessageSpec + 'static,
    TMsg::Message: messaging::prost::Message + Default + Clone + 'static,
    TMsg::TransactionGossipMessage: messaging::prost::Message + Default + Clone + 'static,
    TMsg::ConsensusGossipMessage: messaging::prost::Message + Default + Clone + 'static,
    TMsg: MessageSpec,
{
    pub fn new(identity: Keypair) -> Self {
        Self {
            identity,
            messaging_mode: MessagingMode::Disabled,
            config: crate::Config::default(),
            seed_peers: Vec::new(),
            #[cfg(feature = "metrics")]
            registry: None,
            #[cfg(not(feature = "metrics"))]
            _marker: std::marker::PhantomData,
            _spec: std::marker::PhantomData,
        }
    }

    pub fn with_messaging_mode(mut self, mode: MessagingMode) -> Self {
        self.messaging_mode = mode;
        self
    }

    pub fn then(self, f: impl FnOnce(Self) -> Self) -> Self {
        f(self)
    }

    pub fn with_config(mut self, config: crate::Config) -> Self {
        self.config = config;
        self
    }

    #[cfg(feature = "metrics")]
    pub fn with_metrics(mut self, registry: &'a mut libp2p::metrics::Registry) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn with_seed_peers(mut self, seed_peers: Vec<(Option<PeerId>, Multiaddr)>) -> Self {
        self.seed_peers = seed_peers;
        self
    }

    pub fn spawn(
        self,
        shutdown_signal: ShutdownSignal,
    ) -> anyhow::Result<(NetworkingHandle<TMsg>, JoinHandle<anyhow::Result<()>>)> {
        let Self {
            identity,
            messaging_mode,
            mut config,
            seed_peers,
            #[cfg(feature = "metrics")]
            registry,
            #[cfg(not(feature = "metrics"))]
                _marker: _,
            _spec: _,
        } = self;

        for (_, addr) in &seed_peers {
            if !is_supported_multiaddr(addr) {
                return Err(anyhow!("Unsupported seed peer multi-address: {}", addr));
            }
        }

        config.swarm.enable_messaging = messaging_mode.is_enabled();
        let swarm = tari_swarm::create_swarm::<BytesCodec>(identity.clone(), HashSet::new(), config.swarm.clone())?;
        let local_peer_id = *swarm.local_peer_id();
        let (tx, rx) = mpsc::channel(1);
        let (tx_events, _) = broadcast::channel(100);

        #[allow(unused_mut)]
        let mut worker = NetworkingWorker::<TMsg>::new(
            identity,
            rx,
            tx_events.clone(),
            messaging_mode,
            swarm,
            config,
            seed_peers
                .iter()
                .filter_map(|(peer_id, addr)| Some(((*peer_id)?, addr.clone())))
                .collect(),
            seed_peers,
            shutdown_signal,
        );
        #[cfg(feature = "metrics")]
        if let Some(registry_mut) = registry {
            worker = worker.with_metrics(registry_mut);
        }
        let handle = tokio::spawn(worker.run());
        Ok((NetworkingHandle::new(local_peer_id, tx, tx_events.downgrade()), handle))
    }
}
