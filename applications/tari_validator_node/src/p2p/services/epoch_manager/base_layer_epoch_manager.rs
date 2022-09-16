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

use std::convert::TryInto;

use async_trait::async_trait;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{Committee, Epoch},
    services::{epoch_manager::EpochManager, BaseNodeClient},
};

use crate::grpc::services::base_node_client::GrpcBaseNodeClient;

#[derive(Clone)]
pub struct BaseLayerEpochManager {
    pub base_node_client: GrpcBaseNodeClient,
}

#[async_trait]
impl EpochManager<CommsPublicKey> for BaseLayerEpochManager {
    async fn current_epoch(&self) -> Epoch {
        let tip = self
            .base_node_client
            .clone()
            .get_tip_info()
            .await
            .unwrap()
            .height_of_longest_chain;
        Epoch(tip - 100)
    }

    async fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        let current_epoch = self.current_epoch().await;
        current_epoch.0 - 10 <= epoch.0 && epoch.0 <= current_epoch.0 + 10
    }

    fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<CommsPublicKey>>)>, String> {
        let mut result = vec![];
        for &shard in shards {
            let committee = self.get_committee(epoch, shard).await.ok();
            result.push((shard, committee));
        }
        Ok(result)
    }

    fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<CommsPublicKey>, String> {
        let validator_nodes = self
            .base_node_client
            .clone()
            .get_committee(epoch.0, shard.to_le_bytes().try_into().unwrap())
            .await
            .map_err(|s| format!("{:?}", s))?;
        Ok(Committee::new(validator_nodes))
    }

    fn get_shards(
        &self,
        epoch: Epoch,
        addr: &CommsPublicKey,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String> {
        // If the committee size is bigger than vns.len() then this function is broken.
        let half_committee_size = 5;
        let mut base_node_client = self.base_node_client.clone();
        let shard_key = base_node_client
            .clone()
            .get_shard_key(epoch.0, addr)
            .await
            .map_err(|s| format!("{:?}", s))?;
        let mut vns = base_node_client
            .get_validator_nodes(epoch.0)
            .await
            .map_err(|s| format!("{:?}", s))?;
        vns.sort_by(|a, b| a.shard_key.partial_cmp(&b.shard_key).unwrap());
        let p = vns.iter().position(|x| x.shard_key == shard_key).unwrap();
        let begin = &vns[(vns.len() + p - half_committee_size) % vns.len()].shard_key;
        let end = &vns[(p + half_committee_size) % vns.len()].shard_key;
        if p >= half_committee_size || p + half_committee_size >= vns.len() {
            // This means the committee is wrapped around
            Ok(available_shards
                .iter()
                .filter(|&a| a <= begin || a >= end)
                .map(|a| a.clone())
                .collect())
        } else {
            Ok(available_shards
                .iter()
                .filter(|&a| a >= begin || a <= end)
                .map(|a| a.clone())
                .collect())
        }
    }
}
