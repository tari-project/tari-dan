//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::PeerId;
use tari_dan_common_types::PeerAddress;
use tari_dan_p2p::{proto, Message, TariMessagingSpec};
use tari_networking::{MessageSpec, NetworkingError, NetworkingHandle, NetworkingService};
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct Gossip {
    networking: NetworkingHandle<TariMessagingSpec>,
    rx_gossip: mpsc::UnboundedReceiver<(PeerId, proto::network::Message)>,
}

impl Gossip {
    pub fn new(
        networking: NetworkingHandle<TariMessagingSpec>,
        rx_gossip: mpsc::UnboundedReceiver<(PeerId, proto::network::Message)>,
    ) -> Self {
        Self { networking, rx_gossip }
    }
}

impl Gossip {
    pub async fn next_message<TMsg>(&mut self) -> Option<Result<(PeerAddress, TMsg), anyhow::Error>>
    where
        TMsg: TryFrom<Message>,
        TMsg::Error: Into<anyhow::Error>
    {
        let (peer_id, msg) = self.rx_gossip.recv().await?;
        match TryInto::<Message>::try_into(msg) {
            Ok(msg) => {
                match msg.try_into() {
                    Ok(msg) => Some(Ok((peer_id.into(), msg))),
                    Err(e) => Some(Err(e.into())),
                }
            },
            Err(e) => Some(Err(e)),
        }
    }

    pub async fn subscribe_topic<T: Into<String> + Send>(&mut self, topic: T) -> Result<(), NetworkingError> {
        self.networking.subscribe_topic(topic).await
    }

    pub async fn unsubscribe_topic<T: Into<String> + Send>(&mut self, topic: T) -> Result<(), NetworkingError> {
        self.networking.unsubscribe_topic(topic).await
    }

    pub async fn publish_message<T: Into<String> + Send>(
        &mut self,
        topic: T,
        message: <TariMessagingSpec as MessageSpec>::GossipMessage,
    ) -> Result<(), NetworkingError> {
        self.networking.publish_gossip(topic, message).await
    }
}
