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
use tari_comms::types::CommsPublicKey;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_p2p::{DanMessage, Message, OutboundService};
use tokio::sync::mpsc;

use crate::{comms::Destination, p2p::services::messaging::MessagingError};

const _LOG_TARGET: &str = "tari::dan::messages::outbound::validator_node";

#[derive(Debug, Clone)]
pub struct OutboundMessaging {
    our_node_addr: CommsPublicKey,
    msg_sender: mpsc::Sender<(Destination<CommsPublicKey>, DanMessage<CommsPublicKey>)>,
    consensus_sender: mpsc::Sender<(Destination<CommsPublicKey>, HotstuffMessage<CommsPublicKey>)>,
    loopback_sender: mpsc::Sender<Message<CommsPublicKey>>,
}

impl OutboundMessaging {
    pub fn new(
        our_node_addr: CommsPublicKey,
        msg_sender: mpsc::Sender<(Destination<CommsPublicKey>, DanMessage<CommsPublicKey>)>,
        consensus_sender: mpsc::Sender<(Destination<CommsPublicKey>, HotstuffMessage<CommsPublicKey>)>,
        loopback_sender: mpsc::Sender<Message<CommsPublicKey>>,
    ) -> Self {
        Self {
            our_node_addr,
            msg_sender,
            consensus_sender,
            loopback_sender,
        }
    }

    async fn send_message(
        &self,
        dest: Destination<CommsPublicKey>,
        message: Message<CommsPublicKey>,
    ) -> Result<(), MessagingError> {
        match message {
            Message::Consensus(msg) => {
                self.consensus_sender
                    .send((dest, msg))
                    .await
                    .map_err(|_| MessagingError::MessageSendFailed)?;
            },
            Message::Dan(msg) => {
                self.msg_sender
                    .send((dest, msg))
                    .await
                    .map_err(|_| MessagingError::MessageSendFailed)?;
            },
        }

        Ok(())
    }
}

#[async_trait]
impl OutboundService for OutboundMessaging {
    type Addr = CommsPublicKey;
    type Error = MessagingError;

    async fn send_self<T: Into<Message<Self::Addr>> + Send>(&mut self, message: T) -> Result<(), MessagingError> {
        self.loopback_sender
            .send(message.into())
            .await
            .map_err(|_| MessagingError::LoopbackSendFailed)?;
        return Ok(());
    }

    async fn send<T: Into<Message<Self::Addr>> + Send>(
        &mut self,
        to: Self::Addr,
        message: T,
    ) -> Result<(), MessagingError> {
        if to == self.our_node_addr {
            return self.send_self(message).await;
        }

        self.send_message(Destination::Peer(to), message.into()).await?;

        Ok(())
    }

    async fn broadcast<T: Into<Message<Self::Addr>> + Send>(
        &mut self,
        committee: &[Self::Addr],
        message: T,
    ) -> Result<(), MessagingError> {
        let message = message.into();
        let (ours, theirs) = committee
            .iter()
            .cloned()
            .partition::<Vec<_>, _>(|x| *x == self.our_node_addr);

        // send it more than once to ourselves??
        for _ in ours {
            self.loopback_sender
                .send(message.clone())
                .await
                .map_err(|_| MessagingError::LoopbackSendFailed)?;
        }

        self.send_message(Destination::Selected(theirs), message).await?;

        Ok(())
    }

    async fn flood<T: Into<Message<Self::Addr>> + Send>(&mut self, message: T) -> Result<(), MessagingError> {
        self.send_message(Destination::Flood, message.into()).await
    }
}
