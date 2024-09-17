//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::PeerId;
use tari_dan_common_types::PeerAddress;
use tari_networking::{MessageSpec, NetworkingError, NetworkingHandle, NetworkingService};
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct Gossip<TMessageSpec: MessageSpec + Send> {
    networking: NetworkingHandle<TMessageSpec>,
    rx_gossip: mpsc::UnboundedReceiver<(PeerId, TMessageSpec::GossipMessage)>,
}

impl<TMessageSpec: MessageSpec + Send> Gossip<TMessageSpec> {
    pub fn new(
        networking: NetworkingHandle<TMessageSpec>,
        rx_gossip: mpsc::UnboundedReceiver<(PeerId, TMessageSpec::GossipMessage)>,
    ) -> Self {
        Self { networking, rx_gossip }
    }
}

impl<TMessageSpec: MessageSpec + Send + 'static> Gossip<TMessageSpec> {
    pub async fn next_message<TMsg>(&mut self) -> Option<Result<(PeerAddress, TMsg), anyhow::Error>>
    where
        TMsg: TryFrom<TMessageSpec::GossipMessage>,
        TMsg::Error: Into<anyhow::Error>
    {
        let (from, msg) = self.rx_gossip.recv().await?;
        match msg.try_into() {
            Ok(msg) => Some(Ok((from.into(), msg))),
            Err(e) => Some(Err(e.into())),
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
        message: TMessageSpec::GossipMessage,
    ) -> Result<(), NetworkingError> {
        self.networking.publish_gossip(topic, message).await
    }
}
