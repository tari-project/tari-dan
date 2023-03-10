//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::sync::Arc;

use anyhow::anyhow;
use log::*;
use tari_comms::{
    connectivity::{ConnectivityEvent, ConnectivityRequester},
    peer_manager::NodeId,
    types::CommsPublicKey,
    NodeIdentity,
    PeerConnection,
};
use tari_dan_common_types::optional::Optional;
use tari_dan_core::{
    message::{DanMessage, NetworkAnnounce},
    services::{infrastructure_services::OutboundService, DanPeer, PeerProvider},
};
use tokio::{
    sync::{mpsc, Semaphore},
    task,
};

use crate::p2p::services::{
    comms_peer_provider::CommsPeerProvider,
    messaging::OutboundMessaging,
    networking::{handle::NetworkingRequest, peer_sync::PeerSyncProtocol, NetworkingError},
};

const LOG_TARGET: &str = "tari::validator_node::p2p::services::networking";

pub struct Networking {
    rx_network_announce: mpsc::Receiver<(CommsPublicKey, NetworkAnnounce<CommsPublicKey>)>,
    rx_request: mpsc::Receiver<NetworkingRequest>,
    node_identity: Arc<NodeIdentity>,
    outbound: OutboundMessaging,
    peer_provider: CommsPeerProvider,
    connectivity: ConnectivityRequester,
    peer_sync_permit: Arc<Semaphore>,
}

impl Networking {
    pub fn new(
        rx_network_announce: mpsc::Receiver<(CommsPublicKey, NetworkAnnounce<CommsPublicKey>)>,
        rx_request: mpsc::Receiver<NetworkingRequest>,
        node_identity: Arc<NodeIdentity>,
        outbound: OutboundMessaging,
        peer_provider: CommsPeerProvider,
        connectivity: ConnectivityRequester,
    ) -> Self {
        Self {
            rx_network_announce,
            rx_request,
            node_identity,
            outbound,
            peer_provider,
            connectivity,
            peer_sync_permit: Arc::new(Semaphore::new(1)),
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut events = self.connectivity.get_event_subscription();
        if let Err(err) = self.dial_seed_peers().await {
            error!(target: LOG_TARGET, "🚨 Failed to dial seed peers: {}", err);
        }
        loop {
            tokio::select! {
                Some((_sender, announce)) = self.rx_network_announce.recv() => {
                    if let Err(e) = self.handle_announce(announce).await {
                        error!(target: LOG_TARGET, "Error handling network announce: {}", e);
                    }
                },

                Ok(event) = events.recv() => {
                    if let Err(e) = self.handle_connectivity_event(event).await {
                        error!(target: LOG_TARGET, "Error handling connectivity event: {}", e);
                    }
                },

                Some(request) = self.rx_request.recv() => {
                    if let Err(e) = self.handle_request(request).await {
                        error!(target: LOG_TARGET, "Error handling networking request: {}", e);
                    }
                },

                else => break
            }
        }
        Ok(())
    }

    async fn dial_seed_peers(&self) -> Result<(), NetworkingError> {
        let seed_peers = self.peer_provider.get_seed_peers().await?;
        info!(target: LOG_TARGET, "☎️ Dialing {} seed peers", seed_peers.len());

        self.connectivity
            .request_many_dials(seed_peers.into_iter().map(|p| NodeId::from_public_key(&p.identity)))
            .await?;
        Ok(())
    }

    async fn handle_connectivity_event(&self, event: ConnectivityEvent) -> Result<(), NetworkingError> {
        match event {
            ConnectivityEvent::PeerConnected(conn) => {
                info!(target: LOG_TARGET, "📡 Peer connected: {}", conn);
                self.initiate_sync_protocol(conn);
            },
            evt => {
                info!(target: LOG_TARGET, "ℹ️  Network event: {}", evt);
            },
        }
        Ok(())
    }

    async fn handle_request(&mut self, request: NetworkingRequest) -> Result<(), NetworkingError> {
        match request {
            NetworkingRequest::Announce(reply) => {
                info!(target: LOG_TARGET, "📢 Announcing presence to network");
                // TODO: Identity signature can never be empty, but handle the case properly anyway
                let identity_signature = self.node_identity.identity_signature_read().clone().unwrap();

                let res = self
                    .outbound
                    .flood(
                        Default::default(),
                        DanMessage::NetworkAnnounce(Box::new(NetworkAnnounce {
                            identity: self.node_identity.public_key().clone(),
                            addresses: vec![self.node_identity.public_address().clone()],
                            identity_signature,
                        })),
                    )
                    .await;
                let _ignore = reply.send(res.map_err(Into::into));
            },
        }

        Ok(())
    }

    async fn handle_announce(&mut self, announce: NetworkAnnounce<CommsPublicKey>) -> Result<(), NetworkingError> {
        debug!("Received network announce from {}", announce.identity);
        if self.node_identity.public_key() == &announce.identity {
            debug!("Ignoring network announce from self");
            return Ok(());
        }

        info!(target: LOG_TARGET, "👋 Received announce from {}", announce.identity);

        let peer = DanPeer {
            identity: announce.identity.clone(),
            addresses: announce.addresses.clone(),
            // identity_signature: Some(announce.identity_signature.clone()),
        };

        if !peer.is_valid() {
            return Err(anyhow::anyhow!(
                "Invalid announce: peer {} has an invalid signature",
                peer.identity,
            ));
        }

        match self.peer_provider.get_peer(&announce.identity).await.optional()? {
            Some(existing_peer) => {
                let candidate = peer
                    .identity_signature
                    .as_ref()
                    .ok_or_else(|| anyhow!("Identity signature for announce message is empty"))?;
                if existing_peer
                    .identity_signature
                    .as_ref()
                    .map(|id| candidate.updated_at() > id.updated_at())
                    .unwrap_or(true)
                {
                    self.peer_provider.update_peer(peer).await?;

                    // TODO: should not forward announce to sending peer
                    self.outbound
                        .flood(Default::default(), DanMessage::NetworkAnnounce(Box::new(announce)))
                        .await?;
                }
            },
            None => {
                self.peer_provider.add_peer(peer).await?;
                self.outbound
                    .flood(Default::default(), DanMessage::NetworkAnnounce(Box::new(announce)))
                    .await?;
            },
        }

        Ok(())
    }

    fn initiate_sync_protocol(&self, conn: PeerConnection) {
        let permit = self.peer_sync_permit.clone();
        let peer_provider = self.peer_provider.clone();
        let our_identity = self.node_identity.public_key().clone();
        task::spawn(async move {
            let _permit = match permit.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    debug!(
                        target: LOG_TARGET,
                        "Networking has shut down while waiting for a peer sync permit. Aborting sync."
                    );
                    return;
                },
            };
            let protocol = PeerSyncProtocol::new(conn, our_identity, peer_provider);
            if let Err(err) = protocol.run().await {
                error!(target: LOG_TARGET, "🫂 Peer sync protocol failed: {}", err);
            }
        });
    }
}
