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

use std::{
    convert::TryFrom,
    sync::Arc,
    task::{Context, Poll},
};

use futures::future::BoxFuture;
use prost::Message;
use tari_comms::{message::InboundMessage, types::CommsPublicKey, PeerManager};
use tari_dan_core::{message::DanMessage, models::TariDanPayload};
use tower::{Service, ServiceExt};

use crate::p2p::proto;

#[derive(Debug, Clone)]
pub struct DanDeserialize {
    peer_manager: Arc<PeerManager>,
}

impl DanDeserialize {
    pub fn new(peer_manager: Arc<PeerManager>) -> Self {
        Self { peer_manager }
    }
}

impl<S> tower_layer::Layer<S> for DanDeserialize
where
    S: Service<(CommsPublicKey, DanMessage<TariDanPayload, CommsPublicKey>), Response = (), Error = anyhow::Error>
        + Send
        + Clone
        + 'static,
    S::Future: Send + 'static,
{
    type Service = DanDeserializeService<S>;

    fn layer(&self, next_service: S) -> Self::Service {
        DanDeserializeService {
            next_service,
            peer_manager: self.peer_manager.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DanDeserializeService<S> {
    next_service: S,
    peer_manager: Arc<PeerManager>,
}

impl<S> Service<InboundMessage> for DanDeserializeService<S>
where
    S: Service<(CommsPublicKey, DanMessage<TariDanPayload, CommsPublicKey>), Response = (), Error = anyhow::Error>
        + Send
        + Clone
        + 'static,
    S::Future: Send + 'static,
{
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = ();

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, msg: InboundMessage) -> Self::Future {
        let InboundMessage {
            source_peer, mut body, ..
        } = msg;
        let next_service = self.next_service.clone();
        let peer_manager = self.peer_manager.clone();
        Box::pin(async move {
            let decoded_msg = proto::validator_node::DanMessage::decode(&mut body)?;
            let msg = DanMessage::try_from(decoded_msg)?;
            let peer = peer_manager
                .find_by_node_id(&source_peer)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Could not find peer with node id {}", source_peer))?;
            let mut svc = next_service.ready_oneshot().await?;
            svc.call((peer.public_key, msg)).await?;
            Ok(())
        })
    }
}
