//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use libp2p::PeerId;
use tari_consensus::{messages::HotstuffMessage, traits::InboundMessagingError};
use tari_dan_common_types::PeerAddress;
use tari_dan_p2p::proto;
use tokio::sync::mpsc;

use crate::p2p::logging::MessageLogger;

pub struct ConsensusInboundMessaging<TMsgLogger> {
    local_address: PeerAddress,
    rx_inbound_msg: mpsc::UnboundedReceiver<(PeerId, proto::consensus::HotStuffMessage)>,
    rx_gossip: mpsc::UnboundedReceiver<(PeerId, proto::consensus::HotStuffMessage)>,
    rx_loopback: mpsc::UnboundedReceiver<HotstuffMessage>,
    msg_logger: TMsgLogger,
}

impl<TMsgLogger: MessageLogger> ConsensusInboundMessaging<TMsgLogger> {
    pub fn new(
        local_address: PeerAddress,
        rx_inbound_msg: mpsc::UnboundedReceiver<(PeerId, proto::consensus::HotStuffMessage)>,
        rx_gossip: mpsc::UnboundedReceiver<(PeerId, proto::consensus::HotStuffMessage)>,
        rx_loopback: mpsc::UnboundedReceiver<HotstuffMessage>,
        msg_logger: TMsgLogger,
    ) -> Self {
        Self {
            local_address,
            rx_inbound_msg,
            rx_gossip,
            rx_loopback,
            msg_logger,
        }
    }

    fn handle_message(&self, from: PeerId, msg: proto::consensus::HotStuffMessage) -> Option<Result<(PeerAddress, HotstuffMessage), InboundMessagingError>>  {
                match HotstuffMessage::try_from(msg) {
                    Ok(msg) => {
                        self.msg_logger.log_inbound_message(
                           &from.to_string(),
                           msg.as_type_str(),
                           "",
                           &msg,
                        );
                       Some(Ok((from.into(), msg)))
                    }
                    Err(err) => return Some(Err(InboundMessagingError::InvalidMessage{ reason: err.to_string() } )),
                }
    }
}

#[async_trait]
impl<TMsgLogger: MessageLogger + Send> tari_consensus::traits::InboundMessaging
    for ConsensusInboundMessaging<TMsgLogger>
{
    type Addr = PeerAddress;

    async fn next_message(&mut self) -> Option<Result<(Self::Addr, HotstuffMessage), InboundMessagingError>> {
        tokio::select! {
            // BIASED: messaging priority is loopback, then other
            biased;
            maybe_msg = self.rx_loopback.recv() => maybe_msg.map(|msg| {
                self.msg_logger.log_inbound_message(
                   &self.local_address.to_string(),
                   msg.as_type_str(),
                   "",
                   &msg,
                );
                Ok((self.local_address, msg))
            }),
            maybe_msg = self.rx_inbound_msg.recv() => {
                let (from, msg) = maybe_msg?;
                self.handle_message(from, msg)
            },
            maybe_msg = self.rx_gossip.recv() => {
                let (from, msg) = maybe_msg?;
                self.handle_message(from, msg)
            },
        }
    }
}
