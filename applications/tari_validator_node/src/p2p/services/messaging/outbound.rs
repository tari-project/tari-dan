//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

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
use tari_consensus::{messages::HotstuffMessage, traits::OutboundMessagingError};
use tari_dan_common_types::PeerAddress;
use tari_dan_p2p::{proto, TariMessagingSpec};
use tari_networking::{NetworkingHandle, NetworkingService};
use tokio::sync::mpsc;

use crate::p2p::logging::MessageLogger;

const _LOG_TARGET: &str = "tari::dan::messages::outbound::validator_node";

#[derive(Debug, Clone)]
pub struct ConsensusOutboundMessaging<TMsgLogger> {
    our_node_addr: PeerAddress,
    loopback_sender: mpsc::UnboundedSender<HotstuffMessage>,
    networking: NetworkingHandle<TariMessagingSpec>,
    msg_logger: TMsgLogger,
}

impl<TMsgLogger: MessageLogger> ConsensusOutboundMessaging<TMsgLogger> {
    pub fn new(
        loopback_sender: mpsc::UnboundedSender<HotstuffMessage>,
        networking: NetworkingHandle<TariMessagingSpec>,
        msg_logger: TMsgLogger,
    ) -> Self {
        Self {
            our_node_addr: (*networking.local_peer_id()).into(),
            loopback_sender,
            networking,
            msg_logger,
        }
    }
}

#[async_trait]
impl<TMsgLogger: MessageLogger + Send> tari_consensus::traits::OutboundMessaging
    for ConsensusOutboundMessaging<TMsgLogger>
{
    type Addr = PeerAddress;

    async fn send_self<T: Into<HotstuffMessage> + Send>(&mut self, message: T) -> Result<(), OutboundMessagingError> {
        let message = message.into();
        self.msg_logger.log_outbound_message(
            "self",
            &self.our_node_addr.as_peer_id().to_string(),
            message.as_type_str(),
            "",
            &message,
        );
        self.loopback_sender
            .send(message)
            .map_err(|_| OutboundMessagingError::FailedToEnqueueMessage {
                reason: "loopback sender closed".to_string(),
            })?;
        return Ok(());
    }

    async fn send<T: Into<HotstuffMessage> + Send>(
        &mut self,
        to: Self::Addr,
        message: T,
    ) -> Result<(), OutboundMessagingError> {
        if to == self.our_node_addr {
            return self.send_self(message).await;
        }

        let msg = message.into();

        self.msg_logger
            .log_outbound_message("send", &to.to_string(), msg.as_type_str(), "", &msg);
        self.networking
            .send_message(to.as_peer_id(), proto::consensus::HotStuffMessage::from(&msg))
            .await
            .map_err(OutboundMessagingError::from_error)?;

        Ok(())
    }

    async fn multicast<'a, I, T>(&mut self, committee: I, message: T) -> Result<(), OutboundMessagingError>
    where
        Self::Addr: 'a,
        I: IntoIterator<Item = &'a Self::Addr> + Send,
        T: Into<HotstuffMessage> + Send,
    {
        let message = message.into();

        let (ours, theirs) = committee
            .into_iter()
            .partition::<Vec<&Self::Addr>, _>(|x| **x == self.our_node_addr);

        if ours.is_empty() && theirs.is_empty() {
            return Ok(());
        }

        // send it once to ourselves
        if !ours.is_empty() {
            self.send_self(message.clone()).await?;
        }

        for to in &theirs {
            self.msg_logger
                .log_outbound_message("broadcast", &to.to_string(), message.as_type_str(), "", &message);
        }

        self.networking
            .send_multicast(
                theirs.into_iter().map(|a| a.as_peer_id()).collect::<Vec<_>>(),
                proto::consensus::HotStuffMessage::from(&message),
            )
            .await
            .map_err(OutboundMessagingError::from_error)?;

        Ok(())
    }
}
