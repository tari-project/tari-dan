//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::{Epoch, PeerAddress, ShardGroup, SubstateAddress};
use tari_dan_p2p::{proto::{self, consensus::HotStuffMessage, network::Message}, DanMessage};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerError, EpochManagerReader};
use tari_networking::{MessageSpec, NetworkingError};

use crate::p2p::services::{mempool::MempoolError, messaging::Gossip};

const LOG_TARGET: &str = "tari::validator_node::consensus::gossip";

#[derive(Debug, Clone, Copy)]
pub struct ConsensusMessagingSpec;

impl MessageSpec for ConsensusMessagingSpec {
    type GossipMessage = proto::consensus::HotStuffMessage;
    type Message = proto::consensus::HotStuffMessage;
}

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
pub(super) struct ConsensusGossip<TAddr> {
    epoch_manager: EpochManagerHandle<TAddr>,
    gossip: Gossip<ConsensusMessagingSpec>,
}

impl ConsensusGossip<PeerAddress> {
    pub fn new(epoch_manager: EpochManagerHandle<PeerAddress>, outbound: Gossip<ConsensusMessagingSpec>) -> Self {
        Self {
            epoch_manager,
            gossip: outbound,
        }
    }

    pub async fn next_message(&mut self) -> Option<Result<(PeerAddress, HotStuffMessage), ConsensusGossipError>> {
        self.gossip
            .next_message()
            .await
            .map(|result| result.map_err(ConsensusGossipError::InvalidMessage))
    }

    pub async fn subscribe(&mut self, epoch: Epoch) -> Result<(), ConsensusGossipError> {
        /*
        let committee_shard = self.epoch_manager.get_local_committee_info(epoch).await?;
        match self.is_subscribed {
            Some(b) if b == committee_shard.shard_group() => {
                return Ok(());
            },
            Some(_) => {
                self.unsubscribe().await?;
            },
            None => {},
        }

        self.gossip
            .subscribe_topic(shard_group_to_topic(committee_shard.shard_group()))
            .await?;
        self.is_subscribed = Some(committee_shard.shard_group());
         */
        Ok(())
    }

    pub async fn unsubscribe(&mut self) -> Result<(), ConsensusGossipError> {
        /*
        if let Some(sg) = self.is_subscribed {
            self.gossip.unsubscribe_topic(shard_group_to_topic(sg)).await?;
            self.is_subscribed = None;
        }
         */
        Ok(())
    }

    pub async fn forward_to_local_replicas(&mut self, epoch: Epoch, msg: DanMessage) -> Result<(), ConsensusGossipError> {
        /* 
        let committee = self.epoch_manager.get_local_committee_info(epoch).await?;

        let topic = shard_group_to_topic(committee.shard_group());
        debug!(
            target: LOG_TARGET,
            "forward_to_local_replicas: topic: {}", topic,
        );

        let msg = proto::network::DanMessage::from(&msg);
        self.gossip.publish_message(topic, msg).await?;
        */
        Ok(())
    }

    pub async fn multicast(&mut self, shard_group: ShardGroup, msg: HotstuffMessage) -> Result<(), ConsensusGossipError>
    {
        let topic = shard_group_to_topic(shard_group);

        debug!(
            target: LOG_TARGET,
            "multicast: topic: {}", topic,
        );

        let msg = proto::consensus::HotStuffMessage::from(&msg);

        self.gossip.publish_message(topic, msg.into()).await?;

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
