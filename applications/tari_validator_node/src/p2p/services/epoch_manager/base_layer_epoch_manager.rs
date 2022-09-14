use std::convert::TryInto;

use async_trait::async_trait;
use tari_comms::types::CommsPublicKey;
use tari_crypto::tari_utilities::ByteArray;
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
    async fn current_epoch(&mut self) -> Epoch {
        let tip = &self
            .base_node_client
            .get_tip_info()
            .await
            .unwrap()
            .height_of_longest_chain;
        Epoch(tip - 100)
    }

    async fn is_epoch_valid(&mut self, epoch: Epoch) -> bool {
        let current_epoch = self.current_epoch().await;
        current_epoch.0 - 10 <= epoch.0 && epoch.0 <= current_epoch.0 + 10
    }

    async fn get_committees(
        &mut self,
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

    async fn get_committee(&mut self, epoch: Epoch, shard: ShardId) -> Result<Committee<CommsPublicKey>, String> {
        let validator_nodes = self
            .base_node_client
            .get_committee(epoch.0, shard.to_le_bytes().try_into().unwrap())
            .await
            .map_err(|s| format!("{:?}", s))?;
        Ok(Committee::new(validator_nodes))
    }

    async fn get_shards(
        &mut self,
        epoch: Epoch,
        addr: &CommsPublicKey,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String> {
        // If the committee size is bigger than vns.len() then this function is broken.
        let half_committee_size = 5;
        let &shard_key = self
            .base_node_client
            .get_shard_key(epoch.0, addr.as_bytes().try_into().unwrap())
            .await
            .map_err(|s| format!("{:?}", s))?;
        let mut vns = self
            .base_node_client
            .get_validator_nodes(epoch.0)
            .await
            .map_err(|s| format!("{:?}", s))?;
        vns.sort_by(|a, b| a.shard_key.partial_cmp(&b.shard_key).unwrap());
        let p = vns.iter().position(|x| x.shard_key == shard_key).unwrap();
        let begin = &vns[(vns.len() + p - half_committee_size) % vns.len()]
            .shard_key
            .to_vec();
        let end = &vns[(p + half_committee_size) % vns.len()].shard_key.to_vec();
        if p >= half_committee_size || p + half_committee_size >= vns.len() {
            // This means the committee is wrapped around
            Ok(available_shards
                .iter()
                .filter(|&a| &a.0.to_vec() <= begin || &a.0.to_vec() >= end)
                .map(|a| a.clone())
                .collect())
        } else {
            Ok(available_shards
                .iter()
                .filter(|&a| &a.0.to_vec() >= begin || &a.0.to_vec() <= end)
                .map(|a| a.clone())
                .collect())
        }
    }
}
