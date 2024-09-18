//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::{Epoch, PeerAddress, ShardGroup, SubstateAddress};
use tari_dan_p2p::{proto::{self, network::Message}, DanMessage};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerError, EpochManagerEvent, EpochManagerReader};
use tari_networking::NetworkingError;

use crate::p2p::services::{mempool::MempoolError, messaging::Gossip};


const LOG_TARGET: &str = "tari::validator_node::consensus::gossip";

#[derive(thiserror::Error, Debug)]
pub enum ConsensusGossipError {
    #[error("Invalid message: {0}")]
    InvalidMessage(#[from] anyhow::Error),
    #[error("Epoch Manager Error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Internal service request cancelled")]
    RequestCancelled,
    #[error("Consensus channel closed")]
    ConsensusChannelClosed,
    #[error("Network error: {0}")]
    NetworkingError(#[from] NetworkingError),
}

#[derive(Debug)]
pub(super) struct ConsensusGossipService<TAddr> {
    epoch_manager: EpochManagerHandle<TAddr>,
    gossip: Gossip,
    is_subscribed: Option<ShardGroup>,
}

impl ConsensusGossipService<PeerAddress> {
    pub fn new(epoch_manager: EpochManagerHandle<PeerAddress>, outbound: Gossip) -> Self {
        Self {
            epoch_manager,
            gossip: outbound,
            is_subscribed: None,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut events = self.epoch_manager.subscribe().await?;

        loop {
            tokio::select! {
                Some(result) = self.next_message() => {
                    if let Err(e) = self.handle_new_message(result).await {
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

    async fn next_message(&mut self) -> Option<Result<(PeerAddress, HotstuffMessage), ConsensusGossipError>> {
        self.gossip
            .next_message()
            .await
            .map(|result| result.map_err(ConsensusGossipError::InvalidMessage))
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
        result: Result<(PeerAddress, HotstuffMessage), ConsensusGossipError>,
    ) -> Result<(), ConsensusGossipError> {
        let (from, msg) = result?;

        debug!(
            target: LOG_TARGET,
            "Received NEW consensus gossip message from {}: {:?}",
            from,
            msg
        );

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
