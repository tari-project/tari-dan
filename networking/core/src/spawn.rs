//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use anyhow::anyhow;
use libp2p::{identity::Keypair, Multiaddr, PeerId};
use tari_shutdown::ShutdownSignal;
use tari_swarm::{is_supported_multiaddr, messaging, messaging::prost::ProstCodec};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::{worker::NetworkingWorker, NetworkingHandle};

pub fn spawn<TMsg>(
    identity: Keypair,
    tx_messages: mpsc::Sender<(PeerId, TMsg)>,
    mut config: crate::Config,
    seed_peers: Vec<(PeerId, Multiaddr)>,
    shutdown_signal: ShutdownSignal,
) -> anyhow::Result<(NetworkingHandle<TMsg>, JoinHandle<anyhow::Result<()>>)>
where
    TMsg: messaging::prost::Message + Default + Clone + 'static,
{
    for (_, addr) in &seed_peers {
        if !is_supported_multiaddr(addr) {
            return Err(anyhow!("Unsupported seed peer multi-address: {}", addr));
        }
    }

    config.swarm.enable_relay = config.swarm.enable_relay || !config.reachability_mode.is_private();
    let swarm = tari_swarm::create_swarm::<ProstCodec<TMsg>>(identity.clone(), HashSet::new(), config.swarm.clone())?;
    let local_peer_id = *swarm.local_peer_id();
    let (tx, rx) = mpsc::channel(1);
    let (tx_events, _) = broadcast::channel(100);
    let handle = tokio::spawn(
        NetworkingWorker::<ProstCodec<TMsg>>::new(
            identity,
            rx,
            tx_events.clone(),
            tx_messages,
            swarm,
            config,
            seed_peers,
            shutdown_signal,
        )
        .run(),
    );
    Ok((NetworkingHandle::new(local_peer_id, tx, tx_events), handle))
}
