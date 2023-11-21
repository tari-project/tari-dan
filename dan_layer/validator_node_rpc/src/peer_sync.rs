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

use log::*;
use tari_comms::{peer_manager::PeerIdentityClaim, types::CommsPublicKey, PeerConnection};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_p2p::{DanPeer, PeerProvider};
use tokio_stream::StreamExt;

use crate::{proto, rpc_service};

const LOG_TARGET: &str = "tari::validator_node::networking::peer_sync";

pub struct PeerSyncProtocol<TPeerProvider> {
    conn: PeerConnection,
    our_identity: CommsPublicKey,
    peer_provider: TPeerProvider,
}

impl<TPeerProvider: PeerProvider<Addr = CommsPublicKey>> PeerSyncProtocol<TPeerProvider> {
    pub fn new(conn: PeerConnection, our_identity: CommsPublicKey, peer_provider: TPeerProvider) -> Self {
        Self {
            conn,
            our_identity,
            peer_provider,
        }
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "🫂 Peer sync protocol starting with {}",
            self.conn.peer_node_id()
        );
        let mut client = self.conn.connect_rpc::<rpc_service::ValidatorNodeRpcClient>().await?;

        // TODO: limit peer sync to current epoch
        let mut stream = client.get_peers(proto::rpc::GetPeersRequest::default()).await?;
        let mut count = 0usize;
        while let Some(resp) = stream.next().await {
            let resp = resp?;
            let identity = CommsPublicKey::from_canonical_bytes(&resp.identity).map_err(anyhow::Error::msg)?;
            if self.our_identity == identity {
                continue;
            }

            let claims = resp
                .claims
                .into_iter()
                .map(PeerIdentityClaim::try_from)
                .collect::<Result<Vec<_>, _>>()?;

            let peer = DanPeer { identity, claims };
            debug!(target: LOG_TARGET, "Received peer: {}", peer);
            if !peer.is_valid() {
                return Err(anyhow::anyhow!(
                    "Invalid peer: peer {} has an invalid signature (synced from {})",
                    peer.identity,
                    self.conn.peer_node_id()
                ));
            }

            self.peer_provider.add_peer(peer).await?;
            count += 1;
        }

        info!(target: LOG_TARGET, "🫂 Peer sync protocol synced {} peers", count);

        Ok(())
    }
}
