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
use log::*;
use tari_comms::types::CommsPublicKey;
use tari_dan_core::{message::DanMessage, models::TariDanPayload, services::infrastructure_services::OutboundService};
use tokio::sync::mpsc;

use crate::{comms::Destination, p2p::services::messaging::MessagingError};

const LOG_TARGET: &str = "tari::validator_node::messages::outbound::validator_node";

#[derive(Debug, Clone)]
pub struct OutboundMessaging {
    our_node_addr: CommsPublicKey,
    sender: mpsc::Sender<(Destination<CommsPublicKey>, DanMessage<TariDanPayload, CommsPublicKey>)>,
    loopback_sender: mpsc::Sender<DanMessage<TariDanPayload, CommsPublicKey>>,
}

impl OutboundMessaging {
    pub fn new(
        our_node_addr: CommsPublicKey,
        sender: mpsc::Sender<(Destination<CommsPublicKey>, DanMessage<TariDanPayload, CommsPublicKey>)>,
        loopback_sender: mpsc::Sender<DanMessage<TariDanPayload, CommsPublicKey>>,
    ) -> Self {
        Self {
            our_node_addr,
            sender,
            loopback_sender,
        }
    }
}

#[async_trait]
impl OutboundService for OutboundMessaging {
    type Addr = CommsPublicKey;
    type Error = MessagingError;
    type Payload = TariDanPayload;

    async fn send(
        &mut self,
        _from: Self::Addr,
        to: Self::Addr,
        message: DanMessage<Self::Payload, Self::Addr>,
    ) -> Result<(), MessagingError> {
        // Comment this in to slow down messages for debugging
        info!(
            target: LOG_TARGET,
            "------------------------------------------------------\n",
        );
        // tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        if to == self.our_node_addr {
            debug!(target: LOG_TARGET, "Sending {:?} to self", message);
            self.loopback_sender
                .send(message)
                .await
                .map_err(|_| MessagingError::LoopbackSendFailed)?;
            return Ok(());
        }

        self.sender
            .send((Destination::Peer(to), message))
            .await
            .map_err(|_| MessagingError::MessageSendFailed)?;
        Ok(())
    }

    async fn broadcast(
        &mut self,
        _from: Self::Addr,
        committee: &[Self::Addr],
        message: DanMessage<Self::Payload, Self::Addr>,
    ) -> Result<(), MessagingError> {
        let (ours, theirs) = committee
            .iter()
            .cloned()
            .partition::<Vec<_>, _>(|x| *x == self.our_node_addr);

        // send it more than once to ourselves??
        for _ in ours {
            trace!(target: LOG_TARGET, "Sending {:?} to self", message);
            self.loopback_sender
                .send(message.clone())
                .await
                .map_err(|_| MessagingError::LoopbackSendFailed)?;
        }

        self.sender
            .send((Destination::Selected(theirs), message))
            .await
            .map_err(|_| MessagingError::MessageSendFailed)?;
        Ok(())
    }

    async fn flood(
        &mut self,
        _from: Self::Addr,
        message: DanMessage<Self::Payload, Self::Addr>,
    ) -> Result<(), MessagingError> {
        self.sender
            .send((Destination::Flood, message))
            .await
            .map_err(|_| MessagingError::MessageSendFailed)?;
        Ok(())
    }
}
