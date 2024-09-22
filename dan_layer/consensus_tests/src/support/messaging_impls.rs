//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_consensus::{
    messages::HotstuffMessage,
    traits::{InboundMessaging, InboundMessagingError, OutboundMessaging, OutboundMessagingError},
};
use tari_dan_common_types::ShardGroup;
use tokio::sync::mpsc;

use crate::support::TestAddress;

#[derive(Debug, Clone)]
pub struct TestOutboundMessaging {
    tx_leader: mpsc::Sender<(TestAddress, HotstuffMessage)>,
    _tx_broadcast: mpsc::Sender<(Vec<TestAddress>, HotstuffMessage)>,
    loopback_sender: mpsc::Sender<HotstuffMessage>,
}

impl TestOutboundMessaging {
    pub fn create(
        tx_leader: mpsc::Sender<(TestAddress, HotstuffMessage)>,
        tx_broadcast: mpsc::Sender<(Vec<TestAddress>, HotstuffMessage)>,
    ) -> (Self, mpsc::Receiver<HotstuffMessage>) {
        let (loopback_sender, loopback_receiver) = mpsc::channel(100);
        (
            Self {
                tx_leader,
                _tx_broadcast: tx_broadcast,
                loopback_sender,
            },
            loopback_receiver,
        )
    }
}

#[async_trait]
impl OutboundMessaging for TestOutboundMessaging {
    type Addr = TestAddress;

    async fn send_self<T: Into<HotstuffMessage> + Send>(&mut self, message: T) -> Result<(), OutboundMessagingError> {
        self.loopback_sender
            .send(message.into())
            .await
            .map_err(|_| OutboundMessagingError::FailedToEnqueueMessage {
                reason: "loopback channel closed".to_string(),
            })
    }

    async fn send<T: Into<HotstuffMessage> + Send>(
        &mut self,
        to: Self::Addr,
        message: T,
    ) -> Result<(), OutboundMessagingError> {
        self.tx_leader
            .send((to, message.into()))
            .await
            .map_err(|_| OutboundMessagingError::FailedToEnqueueMessage {
                reason: "leader channel closed".to_string(),
            })
    }

    async fn multicast<'a, T>(&mut self, _shard_group: ShardGroup, _message: T) -> Result<(), OutboundMessagingError>
    where
        Self::Addr: 'a,
        T: Into<HotstuffMessage> + Send,
    {
        Ok(())
    }
}

pub struct TestInboundMessaging {
    local_address: TestAddress,
    receiver: mpsc::Receiver<(TestAddress, HotstuffMessage)>,
    loopback_receiver: mpsc::Receiver<HotstuffMessage>,
}

impl TestInboundMessaging {
    pub fn new(
        local_address: TestAddress,
        receiver: mpsc::Receiver<(TestAddress, HotstuffMessage)>,
        loopback_receiver: mpsc::Receiver<HotstuffMessage>,
    ) -> Self {
        Self {
            local_address,
            receiver,
            loopback_receiver,
        }
    }
}

#[async_trait]
impl InboundMessaging for TestInboundMessaging {
    type Addr = TestAddress;

    async fn next_message(&mut self) -> Option<Result<(Self::Addr, HotstuffMessage), InboundMessagingError>> {
        tokio::select! {
            msg = self.receiver.recv() => msg.map(Ok),
            msg = self.loopback_receiver.recv() => msg.map(|msg| Ok((self.local_address.clone(), msg))),
        }
    }
}
