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
    net_address::{MultiaddrWithStats, MultiaddressesWithStats, PeerAddressSource},
    peer_manager::{NodeId, Peer, PeerFeatures, PeerFlags, PeerManagerError, PeerQuery},
    types::CommsPublicKey,
    PeerManager,
};
use tari_dan_common_types::optional::IsNotFoundError;
use tari_dan_p2p::{DanPeer, PeerProvider};

#[derive(Debug, Clone)]
pub struct CommsPeerProvider {
    peer_manager: Arc<PeerManager>,
}

impl CommsPeerProvider {
    pub fn new(peer_manager: Arc<PeerManager>) -> Self {
        Self { peer_manager }
    }
}

#[async_trait]
impl PeerProvider for CommsPeerProvider {
    type Addr = CommsPublicKey;
    type Error = CommsPeerProviderError;
    type NodeId = NodeId;

    async fn get_peer(&self, addr: &Self::Addr) -> Result<DanPeer<Self::Addr>, Self::Error> {
        match self.peer_manager.find_by_public_key(addr).await? {
            Some(peer) => Ok(DanPeer {
                identity: peer.public_key,
                claims: peer
                    .addresses
                    .addresses()
                    .iter()
                    .filter_map(|a| a.source().peer_identity_claim().cloned())
                    .collect(),
            }),
            None => Err(CommsPeerProviderError::PeerNotFound),
        }
    }

    async fn peers_for_current_epoch_iter(
        &self,
    ) -> Box<dyn Iterator<Item = Result<DanPeer<Self::Addr>, Self::Error>> + Send> {
        // TODO: this is heavy, we need a way for peer manager to iterate over peers without loading all of them at once
        Box::new(self.peer_manager.all().await.unwrap().into_iter().map(|p| {
            Ok(DanPeer {
                identity: p.public_key,
                claims: p
                    .addresses
                    .addresses()
                    .iter()
                    .filter_map(|a| a.source().peer_identity_claim().cloned())
                    .collect(),
            })
        }))
    }

    async fn add_peer(&self, peer: DanPeer<Self::Addr>) -> Result<(), Self::Error> {
        let node_id = NodeId::from_public_key(&peer.identity);
        let addresses = peer
            .claims
            .iter()
            .flat_map(|claim| {
                claim.addresses.iter().map(|addr| {
                    MultiaddrWithStats::new(addr.clone(), PeerAddressSource::FromAnotherPeer {
                        peer_identity_claim: claim.clone(),
                        source_peer: peer.identity.clone(),
                    })
                })
            })
            .collect();
        self.peer_manager
            .add_peer(Peer::new(
                peer.identity.clone(),
                node_id,
                MultiaddressesWithStats::new(addresses),
                PeerFlags::NONE,
                PeerFeatures::NONE,
                vec![],
                String::new(),
            ))
            .await?;
        Ok(())
    }

    async fn update_peer(&self, peer: DanPeer<Self::Addr>) -> Result<(), Self::Error> {
        let node_id = NodeId::from_public_key(&peer.identity);
        if !self.peer_manager.exists(&peer.identity).await {
            return Err(CommsPeerProviderError::PeerNotFound);
        }
        let peer = Peer::new(
            peer.identity.clone(),
            node_id,
            MultiaddressesWithStats::new(
                peer.claims
                    .iter()
                    .flat_map(|claim| {
                        claim.addresses.iter().map(|addr| {
                            MultiaddrWithStats::new(addr.clone(), PeerAddressSource::FromAnotherPeer {
                                peer_identity_claim: claim.clone(),
                                source_peer: peer.identity.clone(),
                            })
                        })
                    })
                    .collect(),
            ),
            PeerFlags::NONE,
            PeerFeatures::COMMUNICATION_NODE,
            vec![],
            String::new(),
        );

        self.peer_manager.add_peer(peer).await?;

        Ok(())
    }

    async fn get_seed_peers(&self) -> Result<Vec<DanPeer<Self::Addr>>, Self::Error> {
        let query = PeerQuery::new().select_where(|p| p.is_seed());
        let peers = self.peer_manager.perform_query(query).await?;
        Ok(peers.into_iter().map(Into::into).collect())
    }

    async fn get_peer_by_node_id(&self, node_id: &Self::NodeId) -> Result<DanPeer<Self::Addr>, Self::Error> {
        match self.peer_manager.find_by_node_id(node_id).await? {
            Some(peer) => Ok(DanPeer {
                identity: peer.public_key,
                claims: peer
                    .addresses
                    .addresses()
                    .iter()
                    .filter_map(|a| a.source().peer_identity_claim().cloned())
                    .collect(),
            }),
            None => Err(CommsPeerProviderError::PeerNotFound),
        }
    }

    async fn is_protocol_supported(&self, addr: &Self::Addr, protocol: &[u8]) -> Result<bool, Self::Error> {
        match self.peer_manager.find_by_public_key(addr).await? {
            Some(peer) => Ok(peer.supported_protocols().iter().any(|p| p == protocol)),
            None => Err(CommsPeerProviderError::PeerNotFound),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommsPeerProviderError {
    #[error(transparent)]
    PeerManagerError(#[from] PeerManagerError),
    #[error("Peer not found")]
    PeerNotFound,
}

impl IsNotFoundError for CommsPeerProviderError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::PeerNotFound)
    }
}
