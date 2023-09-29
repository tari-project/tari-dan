//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::{shard_bucket::ShardBucket, Epoch, ShardId};
use tari_dan_p2p::{DanMessage, OutboundService};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};

use crate::p2p::services::{mempool::MempoolError, messaging::OutboundMessaging};

const LOG_TARGET: &str = "tari::validator_node::mempool::gossip";

#[derive(Debug)]
pub(super) struct Gossip {
    epoch_manager: EpochManagerHandle,
    outbound: OutboundMessaging,
    validator_public_key: CommsPublicKey,
}

impl Gossip {
    pub fn new(
        epoch_manager: EpochManagerHandle,
        outbound: OutboundMessaging,
        validator_public_key: CommsPublicKey,
    ) -> Self {
        Self {
            epoch_manager,
            outbound,
            validator_public_key,
        }
    }

    pub async fn forward_to_local_replicas(
        &mut self,
        epoch: Epoch,
        msg: DanMessage<CommsPublicKey>,
    ) -> Result<(), MempoolError> {
        let committee = self.epoch_manager.get_local_committee(epoch).await?;

        let Some(our_index) = committee
            .members()
            .iter()
            .position(|addr| addr == &self.validator_public_key)
        else {
            error!(target: LOG_TARGET, "BUG: forward_to_local_replicas: get_local_committee returned committee that this node is not part of");
            return Ok(());
        };

        let f = committee.max_failures();

        debug!(
            target: LOG_TARGET,
            "forward_to_local_replicas: {} member(s) selected",
            f + 1,
        );

        let selected_members = committee.select_n_starting_from(f + 1, our_index + 1);
        self.outbound.broadcast(selected_members, msg).await?;

        Ok(())
    }

    pub async fn forward_to_foreign_replicas(
        &mut self,
        epoch: Epoch,
        shards: HashSet<ShardId>,
        msg: DanMessage<CommsPublicKey>,
        exclude_bucket: Option<ShardBucket>,
    ) -> Result<(), MempoolError> {
        let committees = self.epoch_manager.get_committees_by_shards(epoch, shards).await?;
        let local_shard = self.epoch_manager.get_local_committee_shard(epoch).await?;
        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;

        if local_committee.is_empty() {
            error!(target: LOG_TARGET, "BUG: forward_to_foreign_replicas: get_local_committee returned empty committee");
            return Ok(());
        }

        let Some(our_index) = local_committee
            .members()
            .iter()
            .position(|addr| addr == &self.validator_public_key)
        else {
            error!(target: LOG_TARGET, "BUG: forward_to_foreign_replicas: get_local_committee returned committee that this node is not part of");
            return Ok(());
        };

        let mut selected_members = vec![];
        for (bucket, committee) in committees {
            // Dont forward locally
            if bucket == local_shard.bucket() {
                continue;
            }
            if exclude_bucket.map(|b| b == bucket).unwrap_or(false) {
                continue;
            }
            if committee.is_empty() {
                error!(
                    target: LOG_TARGET,
                    "BUG: forward_to_foreign_replicas: get_committees_by_shards returned empty committee"
                );
                continue;
            }
            let n = if local_committee.len() > committee.len() {
                // Our local committee is bigger, so we send to a single node
                1
            } else {
                // Our local committee is smaller, so we send to a portion of their nodes
                committee.len() / local_committee.len()
            };

            selected_members.extend(committee.select_n_starting_from(n, our_index).cloned());
        }

        debug!(
            target: LOG_TARGET,
            "forward_to_foreign_replicas: {} member(s) selected",
            selected_members.len(),
        );

        if selected_members.is_empty() {
            return Ok(());
        }

        self.outbound.broadcast(selected_members.iter(), msg).await?;

        Ok(())
    }

    pub async fn gossip_to_foreign_replicas(
        &mut self,
        epoch: Epoch,
        shards: HashSet<ShardId>,
        msg: DanMessage<CommsPublicKey>,
    ) -> Result<(), MempoolError> {
        let committees = self.epoch_manager.get_committees_by_shards(epoch, shards).await?;
        let local_shard = self.epoch_manager.get_local_committee_shard(epoch).await?;

        let mut selected_members = vec![];
        for (bucket, committee) in committees {
            // Dont forward locally
            if bucket == local_shard.bucket() {
                continue;
            }
            if committee.is_empty() {
                error!(
                    target: LOG_TARGET,
                    "BUG: gossip_to_foreign_replicas: get_committees_by_shards returned empty committee"
                );
                continue;
            }
            let f = committee.max_failures();

            selected_members.extend(committee.select_n_random(f + 1).cloned());
        }

        debug!(
            target: LOG_TARGET,
            "gossip_to_foreign_replicas: {} member(s) selected",
            selected_members.len(),
        );

        if selected_members.is_empty() {
            return Ok(());
        }

        self.outbound.broadcast(selected_members.iter(), msg).await?;

        Ok(())
    }
}
