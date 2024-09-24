//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::PeerId;
use tari_dan_common_types::PeerAddress;
use tari_dan_p2p::{proto, DanMessage, TariMessagingSpec};
use tari_networking::{MessageSpec, NetworkingError, NetworkingHandle, NetworkingService};
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct Gossip {
    networking: NetworkingHandle<TariMessagingSpec>,
    rx_gossip: mpsc::UnboundedReceiver<(PeerId, proto::network::DanMessage)>,
}

impl Gossip {
    pub fn new(
        networking: NetworkingHandle<TariMessagingSpec>,
        rx_gossip: mpsc::UnboundedReceiver<(PeerId, proto::network::DanMessage)>,
    ) -> Self {
        Self { networking, rx_gossip }
    }

    pub fn get_num_incoming_messages(&self) -> usize {
        self.rx_gossip.len()
    }

    pub async fn next_message(&mut self) -> Option<Result<(PeerAddress, DanMessage, usize), anyhow::Error>> {
        let (from, msg) = self.rx_gossip.recv().await?;
        let len = self.rx_gossip.len();
        match msg.try_into() {
            Ok(msg) => Some(Ok((from.into(), msg, len))),
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
