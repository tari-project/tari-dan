//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use libp2p::PeerId;
use log::*;
use tari_dan_common_types::{Epoch, NumPreshards, PeerAddress, ShardGroup, SubstateAddress};
use tari_dan_p2p::{proto, DanMessage, TariMessagingSpec};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_networking::{GossipReceiver, GossipReceiverError, NetworkingHandle, NetworkingService};
use tari_swarm::messaging::prost::ProstCodec;
use tokio::sync::mpsc;
use tari_swarm::messaging::Codec;
use async_trait::async_trait;

use crate::p2p::services::mempool::MempoolError;

const LOG_TARGET: &str = "tari::validator_node::mempool::gossip";

pub struct MempoolGossipReceiver {
    codec:  ProstCodec<proto::network::DanMessage>,
    tx_gossip: mpsc::UnboundedSender<(PeerId, proto::network::DanMessage)>,
}

impl MempoolGossipReceiver {
    pub fn new(tx_gossip: mpsc::UnboundedSender<(PeerId, proto::network::DanMessage)>) -> Self {
        Self {
            codec: ProstCodec::default(),
            tx_gossip
        }
    }

    pub async fn encode(&self, message: proto::network::DanMessage) -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(1024);
        self.codec
            .encode_to(&mut buf, message)
            .await?;
        Ok(buf)
    }
}

#[async_trait]
impl GossipReceiver for MempoolGossipReceiver {
    async fn receive_message(&self, from: PeerId, message: libp2p::gossipsub::Message) -> Result<usize, GossipReceiverError> {
        let (length, message) = self
            .codec
            .decode_from(&mut message.data.as_slice())
            .await
            .map_err(GossipReceiverError::CodecError)?;

        self.tx_gossip
            .send((from, message))
            .map_err(|e| GossipReceiverError::Other(e.to_string()))?;

        Ok(length)
    }
}

#[derive(Debug)]
pub(super) struct MempoolGossip<TAddr> {
    num_preshards: NumPreshards,
    epoch_manager: EpochManagerHandle<TAddr>,
    is_subscribed: Option<ShardGroup>,
    networking: NetworkingHandle<TariMessagingSpec>,
    rx_gossip: mpsc::UnboundedReceiver<(PeerId, proto::network::DanMessage)>,
}

impl MempoolGossip<PeerAddress> {
    pub fn new(
        num_preshards: NumPreshards,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        networking: NetworkingHandle<TariMessagingSpec>,
        rx_gossip: mpsc::UnboundedReceiver<(PeerId, proto::network::DanMessage)>,
    ) -> Self {
        Self {
            num_preshards,
            epoch_manager,
            is_subscribed: None,
            networking,
            rx_gossip,
        }
    }

    pub async fn next_message(&mut self) -> Option<Result<(PeerAddress, DanMessage), MempoolError>> {
        let (from, msg) = self.rx_gossip.recv().await?;
        match msg.try_into() {
            Ok(msg) => Some(Ok((from.into(), msg))),
            Err(e) => Some(Err(MempoolError::InvalidMessage(e))),
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

        let msg = proto::network::DanMessage::from(&msg);
        self.networking.publish_transaction_gossip(topic, msg).await?;

        Ok(())
    }

    pub async fn forward_to_foreign_replicas<T: Into<DanMessage>>(
        &mut self,
        epoch: Epoch,
        substate_addresses: HashSet<SubstateAddress>,
        msg: T,
        exclude_shard_group: Option<ShardGroup>,
    ) -> Result<(), MempoolError> {
        let n = self.epoch_manager.get_num_committees(epoch).await?;
        let committee_shard = self.epoch_manager.get_local_committee_info(epoch).await?;
        let local_shard_group = committee_shard.shard_group();
        let shard_groups = substate_addresses
            .into_iter()
            .map(|s| s.to_shard_group(self.num_preshards, n))
            .filter(|sg| exclude_shard_group.as_ref() != Some(sg) && sg != &local_shard_group)
            .collect::<HashSet<_>>();

        let msg = proto::network::DanMessage::from(&msg.into());
        for sg in shard_groups {
            let topic = shard_group_to_topic(sg);
            debug!(
                target: LOG_TARGET,
                "forward_to_foreign_replicas: topic: {}", topic,
            );

            self.networking.publish_transaction_gossip(topic, msg.clone()).await?;
        }

        Ok(())
    }

    pub async fn gossip_to_foreign_replicas<T: Into<DanMessage>>(
        &mut self,
        epoch: Epoch,
        addresses: HashSet<SubstateAddress>,
        msg: T,
    ) -> Result<(), MempoolError> {
        self.forward_to_foreign_replicas(epoch, addresses, msg, None).await?;
        Ok(())
    }
}

fn shard_group_to_topic(shard_group: ShardGroup) -> String {
    format!(
        "transactions-{}-{}",
        shard_group.start().as_u32(),
        shard_group.end().as_u32()
    )
}
