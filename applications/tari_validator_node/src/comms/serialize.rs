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

use futures::future::BoxFuture;
use tari_comms::{message::OutboundMessage, peer_manager::NodeId, types::CommsPublicKey, Bytes};
use tari_dan_core::{message::DanMessage, models::TariDanPayload};
use tower::{service_fn, util::ServiceFn, Service, ServiceExt};

use crate::p2p::proto;

#[derive(Debug, Clone, Copy, Default)]
pub struct DanSerialize;

impl DanSerialize {
    pub fn new() -> Self {
        Self
    }
}

impl<S> tower_layer::Layer<S> for DanSerialize
where S: Service<OutboundMessage> + Clone + 'static
{
    type Service = ServiceFn<
        fn((NodeId, DanMessage<TariDanPayload, CommsPublicKey>)) -> BoxFuture<'static, Result<(), anyhow::Error>>,
    >;

    fn layer(&self, next_service: S) -> Self::Service {
        service_fn(|(node_id, msg)| {
            let next_service = next_service.clone();
            Box::pin(async move {
                let bytes = encode_message(&proto::validator_node::DanMessage::from(msg.body()));
                let msg = OutboundMessage::new(node_id, bytes).await?;
                let mut svc = next_service.ready_oneshot().await?;
                svc.call(msg).await?;
                Ok(())
            })
        })
    }
}
