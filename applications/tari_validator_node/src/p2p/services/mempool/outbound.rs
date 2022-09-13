//  Copyright 2022. The Tari Project
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
use log::error;
use tari_comms::connectivity::ConnectivityRequester;
use tari_dan_core::{message::DanMessage, services::mempool::outbound::MempoolOutboundService, DigitalAssetError};
use tari_dan_engine::instruction::Transaction;
use tari_p2p::tari_message::TariMessageType;
use tari_validator_node_grpc::rpc::SubmitTransactionRequest;

use crate::p2p::services::outbound::OutboundMessaging;

const LOG_TARGET: &str = "tari::validator_node::p2p::services::mempool::outbound";

pub struct TariCommsMempoolOutboundService {
    connectivity: ConnectivityRequester,
    outbound_messaging: OutboundMessaging,
}

impl TariCommsMempoolOutboundService {
    pub fn new(connectivity: ConnectivityRequester, outbound_messaging: OutboundMessaging) -> Self {
        Self {
            connectivity,
            outbound_messaging,
        }
    }
}

#[async_trait]
impl MempoolOutboundService for TariCommsMempoolOutboundService {
    async fn propagate_transaction(&mut self, transaction: Transaction) -> Result<(), DigitalAssetError> {
        let conns = self.connectivity.get_active_connections().await?;

        let msg = DanMessage::new_transaction(transaction);
        self.outbound_messaging.broadcast(dest, msg).await?;
        let destination = NodeDestination::Unknown;
        let encryption = OutboundEncryption::ClearText;
        let exclude_peers = vec![];

        let request: SubmitTransactionRequest = SubmitTransactionRequest {
            transaction: Some(transaction.into()),
        };
        let message = OutboundDomainMessage::new(&TariMessageType::DanConsensusMessage, request);

        let result = self
            .outbound_messaging
            .flood(destination, encryption, exclude_peers, message)
            .await;

        if let Err(e) = result {
            return match e {
                DhtOutboundError::NoMessagesQueued => Ok(()),
                _ => {
                    error!(target: LOG_TARGET, "propagate_transaction failure. {:?}", e);
                    Err(DigitalAssetError::DhtOutboundError(e))
                },
            };
        }

        Ok(())
    }
}
