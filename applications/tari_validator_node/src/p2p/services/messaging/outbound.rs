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
use tari_dan_core::{
    message::DanMessage,
    models::TariDanPayload,
    services::infrastructure_services::OutboundService,
    DigitalAssetError,
};
use tokio::sync::mpsc;

use crate::comms::Destination;

const LOG_TARGET: &str = "tari::validator_node::messages::outbound::validator_node";

// pub struct TariCommsOutboundService {
//     outbound_messaging: OutboundMessaging,
//     loopback: mpsc::Sender<(CommsPublicKey, DanMessage<TariDanPayload, CommsPublicKey>)>,
// }
//
// impl TariCommsOutboundService {
//     #[allow(dead_code)]
//     pub fn new(
//         outbound_messaging: OutboundMessaging,
//         loopback: mpsc::Sender<(CommsPublicKey, DanMessage<TariDanPayload, CommsPublicKey>)>,
//     ) -> Self {
//         Self {
//             outbound_messaging,
//             loopback,
//         }
//     }
// }
//
// #[async_trait]
// impl OutboundService for TariCommsOutboundService {
//     type Addr = CommsPublicKey;
//     type Payload = TariDanPayload;
//
//     async fn send(
//         &mut self,
//         from: CommsPublicKey,
//         to: CommsPublicKey,
//         message: DanMessage<TariDanPayload, CommsPublicKey>,
//     ) -> Result<(), DigitalAssetError> {
//         debug!(target: LOG_TARGET, "Outbound message to be sent:{} {:?}", to, message);
//
//         // Messages destined to ourselves are added to the loopback queue
//         if from == to {
//             debug!(target: LOG_TARGET, "Sending {:?} to self", message.message_type());
//             self.loopback
//                 .send((from, dan_message))
//                 .await
//                 .map_err(|_| DigitalAssetError::SendError {
//                     context: "Sending to loopback".to_string(),
//                 })?;
//             return Ok(());
//         }
//
//         self.outbound_messaging.send(to, tari_message).await?;
//         Ok(())
//     }
//
//     async fn broadcast(
//         &mut self,
//         from: CommsPublicKey,
//         committee: &[CommsPublicKey],
//         message: DanMessage<TariDanPayload, CommsPublicKey>,
//     ) -> Result<(), DigitalAssetError> {
//         for committee_member in committee {
//             self.send(from.clone(), committee_member.clone(), message.clone())
//                 .await?;
//         }
//         Ok(())
//     }
// }

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
    type Payload = TariDanPayload;

    async fn send(
        &mut self,
        _from: Self::Addr,
        to: Self::Addr,
        message: DanMessage<Self::Payload, Self::Addr>,
    ) -> Result<(), DigitalAssetError> {
        if to == self.our_node_addr {
            trace!(target: LOG_TARGET, "Sending {:?} to self", message);
            self.loopback_sender
                .send(message)
                .await
                .map_err(|_| DigitalAssetError::SendError {
                    context: "Sending to loopback".to_string(),
                })?;
            return Ok(());
        }

        self.sender
            .send((Destination::Peer(to), message))
            .await
            .map_err(|_| DigitalAssetError::SendError {
                context: "Sending to outbound messaging".to_string(),
            })?;
        Ok(())
    }

    async fn broadcast(
        &mut self,
        _from: Self::Addr,
        committee: &[Self::Addr],
        message: DanMessage<Self::Payload, Self::Addr>,
    ) -> Result<(), DigitalAssetError> {
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
                .map_err(|_| DigitalAssetError::SendError {
                    context: "Sending to loopback".to_string(),
                })?;
        }

        self.sender
            .send((Destination::Selected(theirs), message))
            .await
            .map_err(|_| DigitalAssetError::SendError {
                context: "Sending to outbound messaging".to_string(),
            })?;
        Ok(())
    }

    async fn flood(
        &mut self,
        _from: Self::Addr,
        message: DanMessage<Self::Payload, Self::Addr>,
    ) -> Result<(), DigitalAssetError> {
        self.sender
            .send((Destination::Flood, message))
            .await
            .map_err(|_| DigitalAssetError::SendError {
                context: "Sending to outbound messaging".to_string(),
            })?;
        Ok(())
    }
}
