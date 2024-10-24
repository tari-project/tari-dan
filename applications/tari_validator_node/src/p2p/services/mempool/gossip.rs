//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, iter};

use libp2p::{gossipsub, PeerId};
use log::*;
use tari_dan_common_types::{Epoch, NumPreshards, PeerAddress, ShardGroup, ToSubstateAddress};
use tari_dan_p2p::{proto, DanMessage, NewTransactionMessage, TariMessagingSpec};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_networking::{NetworkingHandle, NetworkingService};
use tari_swarm::messaging::{prost::ProstCodec, Codec};
use tokio::sync::mpsc;

use crate::p2p::services::mempool::MempoolError;

const LOG_TARGET: &str = "tari::validator_node::mempool::gossip";

pub const TOPIC_PREFIX: &str = "transactions";

#[derive(Debug)]
pub struct MempoolGossipCodec {
    codec: ProstCodec<proto::network::DanMessage>,
}

impl MempoolGossipCodec {
    pub fn new() -> Self {
        Self {
            codec: ProstCodec::default(),
        }
    }

    pub async fn encode(&self, message: DanMessage) -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(1024);
        let message = proto::network::DanMessage::from(&message);
        self.codec.encode_to(&mut buf, message).await?;
        Ok(buf)
    }

    pub async fn decode(&self, message: gossipsub::Message) -> std::io::Result<(usize, DanMessage)> {
        let (length, message) = self.codec.decode_from(&mut message.data.as_slice()).await?;
        let message = DanMessage::try_from(message).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        Ok((length, message))
    }
}

#[derive(Debug)]
pub(super) struct MempoolGossip<TAddr> {
    num_preshards: NumPreshards,
    epoch_manager: EpochManagerHandle<TAddr>,
    is_subscribed: Option<ShardGroup>,
    networking: NetworkingHandle<TariMessagingSpec>,
    rx_gossip: mpsc::UnboundedReceiver<(PeerId, gossipsub::Message)>,
    codec: MempoolGossipCodec,
}

impl MempoolGossip<PeerAddress> {
    pub fn new(
        num_preshards: NumPreshards,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        networking: NetworkingHandle<TariMessagingSpec>,
        rx_gossip: mpsc::UnboundedReceiver<(PeerId, gossipsub::Message)>,
    ) -> Self {
        Self {
            num_preshards,
            epoch_manager,
            is_subscribed: None,
            networking,
            rx_gossip,
            codec: MempoolGossipCodec::new(),
        }
    }

    pub async fn next_message(&mut self) -> Option<Result<IncomingMessage, MempoolError>> {
        let (from, msg) = self.rx_gossip.recv().await?;
        // Number of transactions still to receive
        let num_pending = self.rx_gossip.len();
        match self.codec.decode(msg).await {
            Ok((msg_len, msg)) => Some(Ok(IncomingMessage {
                address: from.into(),
                message: msg,
                num_pending,
                message_size: msg_len,
            })),
            Err(e) => Some(Err(MempoolError::InvalidMessage(e.into()))),
        }
    }

    pub async fn subscribe(&mut self, epoch: Epoch) -> Result<(), MempoolError> {
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

        self.networking
            .subscribe_topic(shard_group_to_topic(committee_shard.shard_group()))
            .await?;
        self.is_subscribed = Some(committee_shard.shard_group());
        Ok(())
    }

    pub async fn unsubscribe(&mut self) -> Result<(), MempoolError> {
        if let Some(sg) = self.is_subscribed {
            self.networking.unsubscribe_topic(shard_group_to_topic(sg)).await?;
            self.is_subscribed = None;
        }
        Ok(())
    }

    pub async fn forward_to_local_replicas(&mut self, epoch: Epoch, msg: DanMessage) -> Result<(), MempoolError> {
        let committee = self.epoch_manager.get_local_committee_info(epoch).await?;

        let topic = shard_group_to_topic(committee.shard_group());
        debug!(
            target: LOG_TARGET,
            "forward_to_local_replicas: topic: {}", topic,
        );

        let msg = self
            .codec
            .encode(msg)
            .await
            .map_err(|e| MempoolError::InvalidMessage(e.into()))?;
        self.networking.publish_gossip(topic, msg).await?;

        Ok(())
    }

    pub fn get_num_incoming_messages(&self) -> usize {
        self.rx_gossip.len()
    }

    pub async fn forward_to_foreign_replicas(
        &mut self,
        epoch: Epoch,
        msg: NewTransactionMessage,
        exclude_shard_group: Option<ShardGroup>,
    ) -> Result<(), MempoolError> {
        let n = self.epoch_manager.get_num_committees(epoch).await?;
        let committee_shard = self.epoch_manager.get_local_committee_info(epoch).await?;
        let local_shard_group = committee_shard.shard_group();
        let shard_groups = msg
            .transaction
            .all_inputs_iter()
            .map(|s| {
                s.or_zero_version()
                    .to_substate_address()
                    .to_shard_group(self.num_preshards, n)
            })
            .chain(iter::once(
                msg.transaction
                    .id()
                    .to_substate_address()
                    .to_shard_group(self.num_preshards, n),
            ))
            .filter(|sg| exclude_shard_group.as_ref() != Some(sg) && sg != &local_shard_group)
            .collect::<HashSet<_>>();
        // If the only shard group involved is the excluded one.
        if shard_groups.is_empty() {
            return Ok(());
        }

        let msg = self
            .codec
            .encode(msg.into())
            .await
            .map_err(|e| MempoolError::InvalidMessage(e.into()))?;

        for sg in shard_groups {
            let topic = shard_group_to_topic(sg);
            debug!(
                target: LOG_TARGET,
                "forward_to_foreign_replicas: topic: {}", topic,
            );
            self.networking.publish_gossip(topic, msg.clone()).await?;
        }

        Ok(())
    }
}

fn shard_group_to_topic(shard_group: ShardGroup) -> String {
    format!(
        "{}-{}-{}",
        TOPIC_PREFIX,
        shard_group.start().as_u32(),
        shard_group.end().as_u32()
    )
}

pub struct IncomingMessage {
    pub address: PeerAddress,
    pub message: DanMessage,
    pub num_pending: usize,
    pub message_size: usize,
}
