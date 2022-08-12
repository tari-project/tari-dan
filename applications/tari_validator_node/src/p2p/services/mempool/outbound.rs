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
use tari_comms_dht::{
    domain_message::OutboundDomainMessage,
    envelope::NodeDestination,
    outbound::{DhtOutboundError, OutboundEncryption, OutboundMessageRequester},
};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_core::{services::mempool::outbound::MempoolOutboundService, DigitalAssetError};
use tari_dan_engine::instructions::Instruction;
use tari_p2p::tari_message::TariMessageType;

use crate::p2p::proto::validator_node::InvokeMethodRequest;

const LOG_TARGET: &str = "tari::validator_node::p2p::services::mempool::outbound";

pub struct TariCommsMempoolOutboundService {
    outbound_message_requester: OutboundMessageRequester,
}

impl TariCommsMempoolOutboundService {
    pub fn new(outbound_message_requester: OutboundMessageRequester) -> Self {
        Self {
            outbound_message_requester,
        }
    }
}

#[async_trait]
impl MempoolOutboundService for TariCommsMempoolOutboundService {
    async fn propagate_instruction(&mut self, instruction: Instruction) -> Result<(), DigitalAssetError> {
        let destination = NodeDestination::Unknown;
        let encryption = OutboundEncryption::ClearText;
        let exclude_peers = vec![];

        let req = InvokeMethodRequest {
            // TODO: contract id ?
            contract_id: vec![],
            template_id: instruction.template_id() as u32,
            method: instruction.method().to_string(),
            args: instruction.args().to_vec(),
            sender: instruction.sender().to_vec(),
        };

        let message = OutboundDomainMessage::new(&TariMessageType::DanConsensusMessage, req);

        let result = self
            .outbound_message_requester
            .flood(destination, encryption, exclude_peers, message)
            .await;

        if let Err(e) = result {
            return match e {
                DhtOutboundError::NoMessagesQueued => Ok(()),
                _ => {
                    error!(target: LOG_TARGET, "propagate_instruction failure. {:?}", e);
                    Err(DigitalAssetError::DhtOutboundError(e))
                },
            };
        }

        Ok(())
    }
}
