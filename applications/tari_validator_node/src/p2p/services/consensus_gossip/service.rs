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

use std::{collections::HashSet, fmt::Display};

use log::*;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::{Epoch, PeerAddress, ShardGroup, SubstateAddress};
use tari_dan_p2p::proto::{self, network::Message};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerError, EpochManagerEvent, EpochManagerReader};
use tari_networking::NetworkingError;
use tokio::sync::{mpsc, oneshot};
use crate::p2p::services::messaging::Gossip;

use super::{ConsensusGossipError, ConsensusGossipRequest};


const LOG_TARGET: &str = "tari::validator_node::consensus_gossip::service";

#[derive(Debug)]
pub(super) struct ConsensusGossipService<TAddr> {
    requests: mpsc::Receiver<ConsensusGossipRequest>,
    epoch_manager: EpochManagerHandle<TAddr>,
    gossip: Gossip,
    is_subscribed: Option<ShardGroup>,
}

impl ConsensusGossipService<PeerAddress> {
    pub fn new(
        requests: mpsc::Receiver<ConsensusGossipRequest>,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        outbound: Gossip
    ) -> Self {
        Self {
            requests,
            epoch_manager,
            gossip: outbound,
            is_subscribed: None,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut events = self.epoch_manager.subscribe().await?;

        loop {
            tokio::select! {
                Some(req) = self.requests.recv() => self.handle_request(req).await,
                Some(msg) = self.gossip.next_message() => {
                    if let Err(e) = self.handle_new_message(msg).await {
                        warn!(target: LOG_TARGET, "Consensus gossip service rejected message: {}", e);
                    }
                }
                Ok(event) = events.recv() => {
                    if let EpochManagerEvent::EpochChanged(epoch) = event {
                        if self.epoch_manager.is_this_validator_registered_for_epoch(epoch).await?{
                            info!(target: LOG_TARGET, "Consensus gossip service subscribing messages for epoch {}", epoch);
                            self.subscribe(epoch).await?;

                            // TODO: unsubscribe older epoch shards?
                        }
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

    async fn handle_request(&mut self, request: ConsensusGossipRequest) {
        match request {
            ConsensusGossipRequest::Multicast { shard_group, message, reply } => {
                handle(reply, self.multicast(shard_group, message).await);
            },
        }
    }

    async fn subscribe(&mut self, epoch: Epoch) -> Result<(), ConsensusGossipError> {
        let committee_shard = self.epoch_manager.get_local_committee_info(epoch).await?;
        let shard_group = committee_shard.shard_group();

        match self.is_subscribed {
            Some(sg) if sg == shard_group => {
                return Ok(());
            },
            Some(_) => {
                self.unsubscribe().await?;
            },
            None => {},
        }

        let topic = shard_group_to_topic(shard_group);
        self.gossip
            .subscribe_topic(topic)
            .await?;
        self.is_subscribed = Some(committee_shard.shard_group());

        Ok(())
    }

    async fn unsubscribe(&mut self) -> Result<(), ConsensusGossipError> {
        if let Some(sg) = self.is_subscribed {
            let topic = shard_group_to_topic(sg);
            self.gossip.unsubscribe_topic(topic).await?;
            self.is_subscribed = None;
        }

        Ok(())
    }

    async fn handle_new_message(
        &mut self,
        result: Result<(PeerAddress, HotstuffMessage), anyhow::Error>,
    ) -> Result<(), ConsensusGossipError> {
        let (from, msg) = result?;

        debug!(
            target: LOG_TARGET,
            "Received NEW consensus gossip message from {}: {:?}",
            from,
            msg
        );

        // TODO
        match msg {
            HotstuffMessage::NewView(_msg) => {},
            HotstuffMessage::Proposal(_msg) => {},
            HotstuffMessage::ForeignProposal(msg) => {},
            HotstuffMessage::Vote(_msg) => {},
            HotstuffMessage::MissingTransactionsRequest(_msg) => {},
            HotstuffMessage::MissingTransactionsResponse(_msg) => {},
            HotstuffMessage::CatchUpSyncRequest(_msg) => {},
            HotstuffMessage::SyncResponse(_msg) => {},
        }

        Ok(())
    }

    pub async fn multicast<T>(&mut self, shard_group: ShardGroup, message: T) -> Result<(), ConsensusGossipError>
    where
        T: Into<HotstuffMessage> + Send
    {
        let topic = shard_group_to_topic(shard_group);

        debug!(
            target: LOG_TARGET,
            "multicast: topic: {}", topic,
        );

        let message = HotstuffMessage::from(message.into());
        let message = proto::network::Message::from(&message.into());

        self.gossip.publish_message(topic, message).await?;

        Ok(())
    }
}

fn shard_group_to_topic(shard_group: ShardGroup) -> String {
    format!(
        "consensus-{}-{}",
        shard_group.start().as_u32(),
        shard_group.end().as_u32()
    )
}


fn handle<T, E: Display>(reply: oneshot::Sender<Result<T, E>>, result: Result<T, E>) {
    if let Err(ref e) = result {
        error!(target: LOG_TARGET, "Request failed with error: {}", e);
    }
    if reply.send(result).is_err() {
        error!(target: LOG_TARGET, "Requester abandoned request");
    }
}
