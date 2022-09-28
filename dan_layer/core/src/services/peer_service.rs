//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use async_trait::async_trait;
use tari_comms::{
    multiaddr::Multiaddr,
    peer_manager::{IdentitySignature, Peer},
    types::CommsPublicKey,
};

use crate::services::infrastructure_services::NodeAddressable;

#[async_trait]
pub trait PeerProvider {
    type Addr: NodeAddressable + Send;
    type Error: std::error::Error + Send + Sync + 'static;

    async fn get_seed_peers(&self) -> Result<Vec<DanPeer<Self::Addr>>, Self::Error>;
    async fn get_peer(&self, addr: &Self::Addr) -> Result<DanPeer<Self::Addr>, Self::Error>;
    async fn add_peer(&self, peer: DanPeer<Self::Addr>) -> Result<(), Self::Error>;
    async fn update_peer(&self, peer: DanPeer<Self::Addr>) -> Result<(), Self::Error>;
    async fn peers_for_current_epoch_iter(
        &self,
    ) -> Box<dyn Iterator<Item = Result<DanPeer<Self::Addr>, Self::Error>> + Send>;
}

pub struct DanPeer<TAddr> {
    pub identity: TAddr,
    pub addresses: Vec<Multiaddr>,
    pub identity_signature: Option<IdentitySignature>,
}

impl From<Peer> for DanPeer<CommsPublicKey> {
    fn from(peer: Peer) -> Self {
        Self {
            identity: peer.public_key,
            addresses: peer.addresses.into_vec(),
            identity_signature: peer.identity_signature,
        }
    }
}
