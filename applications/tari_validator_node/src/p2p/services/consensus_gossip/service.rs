//  Copyright 2024. The Tari Project
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

use libp2p::{PeerId, gossipsub::MessageAcceptance};
use log::*;
use tari_consensus::{hotstuff::HotstuffEvent, messages::HotstuffMessage};
use tari_epoch_manager::EpochManagerEvent;
use tari_networking::{GossipMessage, NetworkingHandle, NetworkingService};
use tari_ootle_p2p::{TariMessagingSpec, proto};
use tari_swarm::messaging::{Codec, prost::ProstCodec};
use tokio::sync::{broadcast, mpsc};

use super::ConsensusGossipError;

const LOG_TARGET: &str = "tari::validator_node::consensus_gossip::service";

/// All consensus gossip is published on a single network-wide topic. Using one topic (rather than a topic per shard
/// group) keeps the gossipsub mesh stable across epoch boundaries, since validators never need to unsubscribe and
/// resubscribe when they are shuffled into a different shard group.
pub const TOPIC_PREFIX: &str = "consensus";

#[derive(Debug)]
pub(super) struct ConsensusGossipService {
    epoch_manager_events: broadcast::Receiver<EpochManagerEvent>,
    consensus_events: broadcast::Receiver<HotstuffEvent>,
    is_subscribed: bool,
    networking: NetworkingHandle<TariMessagingSpec>,
    codec: ProstCodec<proto::consensus::HotStuffMessage>,
    rx_gossip: mpsc::Receiver<GossipMessage>,
    tx_consensus_gossip: mpsc::Sender<(PeerId, HotstuffMessage)>,
}

impl ConsensusGossipService {
    pub fn new(
        epoch_manager_events: broadcast::Receiver<EpochManagerEvent>,
        consensus_events: broadcast::Receiver<HotstuffEvent>,
        networking: NetworkingHandle<TariMessagingSpec>,
        rx_gossip: mpsc::Receiver<GossipMessage>,
        tx_consensus_gossip: mpsc::Sender<(PeerId, HotstuffMessage)>,
    ) -> Self {
        Self {
            epoch_manager_events,
            consensus_events,
            is_subscribed: false,
            networking,
            codec: ProstCodec::default(),
            rx_gossip,
            tx_consensus_gossip,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut initial_subscription_complete = false;
        loop {
            tokio::select! {
                Ok(HotstuffEvent::EpochChanged{ registered_shard_group, .. }) = self.consensus_events.recv() => {
                    if registered_shard_group.is_some() {
                        self.subscribe().await?;
                    } else {
                        self.unsubscribe().await?;
                    }
                },
                Some(msg) = self.rx_gossip.recv() => {
                    if let Err(err) = self.handle_incoming_gossip_message(msg).await {
                        warn!(target: LOG_TARGET, "Consensus gossip service error: {}", err);
                    }
                },
                Ok(EpochManagerEvent::EpochChanged{ registered_shard_group, .. }) = self.epoch_manager_events.recv() => {
                    if !initial_subscription_complete && registered_shard_group.is_some() {
                        self.subscribe().await?;
                        initial_subscription_complete = true;
                    }
                },
                else => {
                    info!(target: LOG_TARGET, "Consensus gossip service shutting down");
                    break;
                }
            }
        }

        self.unsubscribe().await?;

        Ok(())
    }

    async fn handle_incoming_gossip_message(&mut self, gossip: GossipMessage) -> Result<(), ConsensusGossipError> {
        let (message_id, propagation_source) = gossip.validation_key();
        let from = gossip.source;

        let decoded = self
            .codec
            .decode_from(&mut gossip.message.data.as_slice())
            .await
            .map_err(anyhow::Error::from)
            .and_then(|(_, msg)| decode_gossiped_message(msg));

        // gossipsub withholds the message from the mesh until a verdict is reported. A message that
        // is not a well-formed broadcast is withheld and counted against the peer that sent it;
        // anything else is accepted so it continues to propagate.
        let acceptance = if decoded.is_ok() {
            MessageAcceptance::Accept
        } else {
            MessageAcceptance::Reject
        };
        if let Err(e) = self
            .networking
            .report_gossip_validation(message_id, propagation_source, acceptance)
            .await
        {
            warn!(target: LOG_TARGET, "Failed to report gossip validation result: {e}");
        }

        let msg = decoded.map_err(ConsensusGossipError::InvalidMessage)?;

        self.tx_consensus_gossip
            .send((from, msg))
            .await
            .map_err(|e| ConsensusGossipError::InvalidMessage(e.into()))?;

        Ok(())
    }

    async fn subscribe(&mut self) -> Result<(), ConsensusGossipError> {
        if self.is_subscribed {
            return Ok(());
        }

        info!(target: LOG_TARGET, "🌬️ Consensus gossip service subscribing to {}", topic());
        self.networking.subscribe_topic(topic()).await?;
        self.is_subscribed = true;

        Ok(())
    }

    async fn unsubscribe(&mut self) -> Result<(), ConsensusGossipError> {
        if self.is_subscribed {
            self.networking.unsubscribe_topic(topic()).await?;
            self.is_subscribed = false;
        }

        Ok(())
    }
}

pub(super) fn topic() -> String {
    TOPIC_PREFIX.to_string()
}

/// Converts a message received on the consensus topic, refusing any kind consensus never broadcasts before
/// its payload is decoded. Every other kind travels point to point, where the receiver can tell who sent it.
fn decode_gossiped_message(msg: proto::consensus::HotStuffMessage) -> anyhow::Result<HotstuffMessage> {
    match &msg.message {
        Some(proto::consensus::hot_stuff_message::Message::ForeignProposalNotification(_)) => {
            HotstuffMessage::try_from(msg)
        },
        Some(_) => Err(anyhow::anyhow!(
            "Peer gossiped a consensus message that is only sent point to point"
        )),
        None => Err(anyhow::anyhow!("Peer gossiped an empty consensus message")),
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_p2p::proto::{
        consensus::{
            ForeignProposalNotification,
            HotStuffMessage,
            MissingTransactionsResponse,
            hot_stuff_message::Message,
        },
        transaction::Transaction,
    };

    use super::*;

    #[test]
    fn a_foreign_proposal_notification_is_accepted() {
        let msg = HotStuffMessage {
            message: Some(Message::ForeignProposalNotification(ForeignProposalNotification {
                block_id: vec![1; 32],
                epoch: 1,
                shard_groups: vec![],
                shard_group: 0,
            })),
        };
        let decoded = decode_gossiped_message(msg).unwrap();
        assert!(matches!(decoded, HotstuffMessage::ForeignProposalNotification(_)));
    }

    #[test]
    fn a_point_to_point_message_is_refused_before_its_payload_is_decoded() {
        // The transaction bytes do not decode, so an error about the message kind shows they were never tried.
        let msg = HotStuffMessage {
            message: Some(Message::RequestedTransaction(MissingTransactionsResponse {
                request_id: 1,
                epoch: 1,
                block_id: vec![1; 32],
                transactions: vec![Transaction {
                    bor_encoded: vec![0xff; 16],
                }],
            })),
        };
        let err = decode_gossiped_message(msg).unwrap_err();
        assert!(err.to_string().contains("point to point"), "{err}");
    }
}
