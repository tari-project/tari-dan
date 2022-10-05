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

use async_trait::async_trait;
use tari_comms::{
    connectivity::ConnectivityRequester,
    peer_manager::PeerFeatures,
    types::CommsPublicKey,
    NodeIdentity,
};
use tari_dan_core::message::NetworkAnnounce;
use tokio::sync::mpsc;

use crate::p2p::services::{comms_peer_provider::CommsPeerProvider, messaging::OutboundMessaging};

mod service;
use service::Networking;

mod error;
pub use error::NetworkingError;

mod handle;
mod peer_sync;

pub use handle::NetworkingHandle;

pub const DAN_PEER_FEATURES: PeerFeatures = PeerFeatures::COMMUNICATION_NODE;

pub fn spawn(
    rx_network_announce: mpsc::Receiver<(CommsPublicKey, NetworkAnnounce<CommsPublicKey>)>,
    node_identity: Arc<NodeIdentity>,
    outbound: OutboundMessaging,
    peer_provider: CommsPeerProvider,
    connectivity: ConnectivityRequester,
) -> NetworkingHandle {
    let (tx, rx) = mpsc::channel(1);
    tokio::spawn(
        Networking::new(
            rx_network_announce,
            rx,
            node_identity,
            outbound,
            peer_provider,
            connectivity,
        )
        .run(),
    );
    NetworkingHandle::new(tx)
}

#[async_trait]
pub trait NetworkingService {
    async fn announce(&mut self) -> Result<(), NetworkingError>;
}
