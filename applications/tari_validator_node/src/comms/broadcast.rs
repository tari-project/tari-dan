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
    future,
    task::{Context, Poll},
};

use futures::{stream, StreamExt};
use tari_comms::{
    connectivity::ConnectivityRequester,
    message::OutboundMessage,
    peer_manager::NodeId,
    types::CommsPublicKey,
    Bytes,
};
use tari_dan_core::{message::DanMessage, models::TariDanPayload};
use tonic::codegen::futures_core::future::BoxFuture;
use tower::{Service, ServiceExt};

use crate::{comms::destination::Destination, p2p::proto};

const LOG_TARGET: &str = "tari::validator_node::comms::messaging";

#[derive(Debug, Clone)]
pub struct DanBroadcast {
    connectivity: ConnectivityRequester,
}

impl DanBroadcast {
    pub fn new(connectivity: ConnectivityRequester) -> Self {
        Self { connectivity }
    }
}

impl<S> tower_layer::Layer<S> for DanBroadcast
where
    S: Service<OutboundMessage, Response = (), Error = anyhow::Error> + Sync + Send + Clone + 'static,
    S::Future: Send + 'static,
{
    type Service = BroadcastService<S>;

    fn layer(&self, next_service: S) -> Self::Service {
        BroadcastService {
            next_service,
            connectivity: self.connectivity.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BroadcastService<S> {
    connectivity: ConnectivityRequester,
    next_service: S,
}

impl<S> Service<(Destination<CommsPublicKey>, DanMessage<TariDanPayload, CommsPublicKey>)> for BroadcastService<S>
where
    S: Service<OutboundMessage, Response = (), Error = anyhow::Error> + Sync + Send + Clone + 'static,
    S::Future: Send + 'static,
{
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = ();

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(
        &mut self,
        (dest, msg): (Destination<CommsPublicKey>, DanMessage<TariDanPayload, CommsPublicKey>),
    ) -> Self::Future {
        let mut next_service = self.next_service.clone();
        let mut connectivity = self.connectivity.clone();

        Box::pin(async move {
            let type_str = msg.as_type_str();
            let bytes = encode_message(&proto::validator_node::DanMessage::from(msg));

            log::info!(
                target: LOG_TARGET,
                "📨 Tx: {} ({} bytes) to {}",
                type_str,
                bytes.len(),
                dest
            );
            let svc = next_service.ready().await?;
            match dest {
                Destination::Peer(pk) => {
                    svc.call(OutboundMessage::new(NodeId::from_public_key(&pk), bytes))
                        .await?;
                },
                Destination::Selected(pks) => {
                    let iter = pks
                        .iter()
                        .map(NodeId::from_public_key)
                        .map(|n| OutboundMessage::new(n, bytes.clone()));
                    svc.call_all(stream::iter(iter))
                        .unordered()
                        .filter_map(|result| future::ready(result.err()))
                        .for_each(|err| {
                            // TODO: this should return the error back to the service
                            log::warn!("Error when sending broadcast messages: {}", err);
                            future::ready(())
                        })
                        .await;
                },
                Destination::Flood => {
                    let conns = connectivity.get_active_connections().await?;
                    let iter = conns
                        .into_iter()
                        .map(|c| c.peer_node_id().clone())
                        .map(|n| OutboundMessage::new(n, bytes.clone()));
                    svc.call_all(stream::iter(iter))
                        .unordered()
                        .filter_map(|result| future::ready(result.err()))
                        .for_each(|err| {
                            // TODO: this should return the error back to the service
                            log::warn!("Error when sending broadcast messages: {}", err);
                            future::ready(())
                        })
                        .await;
                },
            }

            Ok(())
        })
    }
}

fn encode_message<T: prost::Message>(msg: &T) -> Bytes {
    let mut buf = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut buf).expect(
        "prost::Message::encode documentation says it is infallible unless the buffer has insufficient capacity. This \
         buffer's capacity was set with encoded_len",
    );
    buf.into()
}
