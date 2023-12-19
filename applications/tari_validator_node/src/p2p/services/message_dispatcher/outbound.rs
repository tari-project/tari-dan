//  Copyright 2021. The Tari Project
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

use async_trait::async_trait;
use libp2p::PeerId;
use tari_dan_common_types::PeerAddress;
use tari_dan_p2p::{Message, OutboundService};
use tari_networking::{NetworkingHandle, NetworkingService};
use tari_validator_node_rpc::proto;
use tokio::sync::mpsc;

use crate::p2p::{logging::MessageLogger, services::message_dispatcher::MessagingError};

const _LOG_TARGET: &str = "tari::dan::messages::outbound::validator_node";

#[derive(Debug, Clone)]
pub struct OutboundMessaging<TAddr, TMsgLogger> {
    our_node_addr: TAddr,
    loopback_sender: mpsc::Sender<Message>,
    networking: NetworkingHandle<proto::network::Message>,
    msg_logger: TMsgLogger,
}

impl<TAddr: From<PeerId> + Send, TMsgLogger: MessageLogger> OutboundMessaging<TAddr, TMsgLogger> {
    pub fn new(
        loopback_sender: mpsc::Sender<Message>,
        networking: NetworkingHandle<proto::network::Message>,
        msg_logger: TMsgLogger,
    ) -> Self {
        Self {
            our_node_addr: (*networking.local_peer_id()).into(),
            loopback_sender,
            networking,
            msg_logger,
        }
    }

    pub fn networking_mut(&mut self) -> &mut NetworkingHandle<proto::network::Message> {
        &mut self.networking
    }
}

#[async_trait]
impl<TMsgLogger: MessageLogger + Send + 'static> OutboundService for OutboundMessaging<PeerAddress, TMsgLogger> {
    type Addr = PeerAddress;
    type Error = MessagingError;

    async fn send_self<T: Into<Message> + Send>(&mut self, message: T) -> Result<(), MessagingError> {
        let message = message.into();
        self.msg_logger.log_outbound_message(
            "self",
            &self.our_node_addr.as_peer_id().to_string(),
            &message.to_type_str(),
            &message.get_message_tag(),
            &message,
        );
        self.loopback_sender
            .send(message)
            .await
            .map_err(|_| MessagingError::LoopbackSendFailed)?;
        return Ok(());
    }

    async fn send<T: Into<Message> + Send>(&mut self, to: Self::Addr, message: T) -> Result<(), MessagingError> {
        if to == self.our_node_addr {
            return self.send_self(message).await;
        }

        let msg = message.into();

        self.msg_logger.log_outbound_message(
            "send",
            &to.to_string(),
            &msg.to_type_str(),
            &msg.get_message_tag(),
            &msg,
        );
        self.networking
            .send_message(to.as_peer_id(), proto::network::Message::from(&msg))
            .await?;

        Ok(())
    }

    async fn broadcast<'a, I, T>(&mut self, committee: I, message: T) -> Result<(), MessagingError>
    where
        I: IntoIterator<Item = &'a Self::Addr> + Send,
        T: Into<Message> + Send,
        Self::Addr: 'a,
    {
        let message = message.into();

        let (ours, theirs) = committee
            .into_iter()
            .partition::<Vec<_>, _>(|x| **x == self.our_node_addr);

        if ours.is_empty() && theirs.is_empty() {
            return Ok(());
        }

        // send it more than once to ourselves??
        for _ in ours {
            self.msg_logger.log_outbound_message(
                "broadcast",
                &self.our_node_addr.to_string(),
                &message.to_type_str(),
                &message.get_message_tag(),
                &message,
            );
            self.loopback_sender
                .send(message.clone())
                .await
                .map_err(|_| MessagingError::LoopbackSendFailed)?;
        }

        for to in &theirs {
            self.msg_logger.log_outbound_message(
                "broadcast",
                &to.to_string(),
                &message.to_type_str(),
                &message.get_message_tag(),
                &message,
            );
        }

        self.networking
            .send_multicast(
                theirs.into_iter().map(|a| a.as_peer_id()).collect::<Vec<_>>(),
                (&message).into(),
            )
            .await?;

        Ok(())
    }

    async fn publish_gossip<TTopic: Into<String> + Send, TMsg: Into<Message> + Send>(
        &mut self,
        topic: TTopic,
        message: TMsg,
    ) -> Result<(), Self::Error> {
        let msg = message.into();
        let topic = topic.into();
        self.msg_logger.log_outbound_message(
            "gossip",
            &topic.to_string(),
            &msg.to_type_str(),
            &msg.get_message_tag(),
            &msg,
        );
        self.networking.gossip(topic, (&msg).into()).await?;
        Ok(())
    }
}
