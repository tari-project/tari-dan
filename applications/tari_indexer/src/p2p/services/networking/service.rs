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

use log::*;
use tari_comms::{
    connectivity::{ConnectivityEvent, ConnectivityRequester},
    peer_manager::NodeId,
    protocol::rpc::NamedProtocolService,
    NodeIdentity,
    PeerConnection,
};
use tari_dan_p2p::PeerProvider;
use tari_validator_node_rpc::peer_sync::PeerSyncProtocol;
use tokio::{sync::Semaphore, task};

use crate::p2p::services::{comms_peer_provider::CommsPeerProvider, networking::NetworkingError};

const LOG_TARGET: &str = "tari::dan::indexer::p2p::services::networking";

pub struct Networking {
    node_identity: Arc<NodeIdentity>,
    peer_provider: CommsPeerProvider,
    connectivity: ConnectivityRequester,
    peer_sync_permit: Arc<Semaphore>,
}

impl Networking {
    pub fn new(
        node_identity: Arc<NodeIdentity>,
        peer_provider: CommsPeerProvider,
        connectivity: ConnectivityRequester,
    ) -> Self {
        Self {
            node_identity,
            peer_provider,
            connectivity,
            peer_sync_permit: Arc::new(Semaphore::new(1)),
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let mut events = self.connectivity.get_event_subscription();
        if let Err(err) = self.dial_seed_peers().await {
            error!(target: LOG_TARGET, "🚨 Failed to dial seed peers: {}", err);
        }
        loop {
            tokio::select! {
                Ok(event) = events.recv() => {
                    if let Err(e) = self.handle_connectivity_event(event).await {
                        error!(target: LOG_TARGET, "Error handling connectivity event: {}", e);
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
                debug!(target: LOG_TARGET, "📡 Peer connected: {}", conn);
                if self.is_vn_protocol_supported(&conn).await? {
                    self.initiate_sync_protocol(conn.as_ref().clone());
                }
            },
            evt => {
                debug!(target: LOG_TARGET, "ℹ️  Network event: {}", evt);
            },
        }
        Ok(())
    }

    async fn is_vn_protocol_supported(&self, conn: &PeerConnection) -> Result<bool, NetworkingError> {
        let peer = self.peer_provider.get_peer_by_node_id(conn.peer_node_id()).await?;
        let is_supported = self
            .peer_provider
            .is_protocol_supported(
                &peer.identity,
                tari_validator_node_rpc::rpc_service::ValidatorNodeRpcClient::PROTOCOL_NAME,
            )
            .await?;
        Ok(is_supported)
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
