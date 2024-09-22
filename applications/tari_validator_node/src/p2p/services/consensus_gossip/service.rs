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

use std::fmt::Display;

use log::*;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::{Epoch, PeerAddress, ShardGroup};
use tari_dan_p2p::{proto, TariMessagingSpec};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerEvent, EpochManagerReader};
use tari_networking::{NetworkingHandle, NetworkingService};
use tokio::sync::{mpsc, oneshot};

use super::{ConsensusGossipError, ConsensusGossipRequest};


const LOG_TARGET: &str = "tari::validator_node::consensus_gossip::service";

#[derive(Debug)]
pub(super) struct ConsensusGossipService<TAddr> {
    requests: mpsc::Receiver<ConsensusGossipRequest>,
    epoch_manager: EpochManagerHandle<TAddr>,
    is_subscribed: Option<ShardGroup>,
    networking: NetworkingHandle<TariMessagingSpec>,
}

impl ConsensusGossipService<PeerAddress> {
    pub fn new(
        requests: mpsc::Receiver<ConsensusGossipRequest>,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        networking: NetworkingHandle<TariMessagingSpec>,
    ) -> Self {
        Self {
            requests,
            epoch_manager,
            is_subscribed: None,
            networking,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut events = self.epoch_manager.subscribe().await?;

        loop {
            tokio::select! {
                Some(req) = self.requests.recv() => self.handle_request(req).await,
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
        self.networking
            .subscribe_topic(topic)
            .await?;
        self.is_subscribed = Some(committee_shard.shard_group());

        Ok(())
    }

    async fn unsubscribe(&mut self) -> Result<(), ConsensusGossipError> {
        if let Some(sg) = self.is_subscribed {
            let topic = shard_group_to_topic(sg);
            self.networking.unsubscribe_topic(topic).await?;
            self.is_subscribed = None;
        }

        Ok(())
    }

    pub async fn multicast(&mut self, shard_group: ShardGroup, message: HotstuffMessage) -> Result<(), ConsensusGossipError>
    {
        let topic = shard_group_to_topic(shard_group);

        debug!(
            target: LOG_TARGET,
            "multicast: topic: {}", topic,
        );

        let message = proto::consensus::HotStuffMessage::from(&message);

        self.networking.publish_consensus_gossip(topic, message).await?;

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
