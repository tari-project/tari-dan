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

const LOG_TARGET: &str = "tari::validator_node::p2p::services::mempool_outbound_service";

pub struct TariCommsMempoolOutboundService {
    outbound_message_requester: Option<OutboundMessageRequester>,
}

impl TariCommsMempoolOutboundService {
    pub fn new() -> Self {
        Self {
            outbound_message_requester: None,
        }
    }
}

#[async_trait]
impl MempoolOutboundService for TariCommsMempoolOutboundService {
    async fn propagate_instruction(&mut self, instruction: Instruction) -> Result<(), DigitalAssetError> {
        let outbound_message_requester = match &mut self.outbound_message_requester {
            Some(requester) => requester,
            None => return Ok(()),
        };

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

        let result = outbound_message_requester
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
