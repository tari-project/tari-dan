use async_trait::async_trait;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{Committee, Epoch},
    services::epoch_manager::EpochManager,
};

#[derive(Clone)]
pub struct BaseLayerEpochManager {}

#[async_trait]
impl EpochManager<CommsPublicKey> for BaseLayerEpochManager {
    async fn current_epoch(&self) -> Epoch {
        todo!()
    }

    async fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        todo!()
    }

    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<CommsPublicKey>>)>, String> {
        todo!()
    }

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<CommsPublicKey>, String> {
        todo!()
    }

    async fn get_shards(
        &self,
        epoch: Epoch,
        addr: &CommsPublicKey,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String> {
        todo!()
    }
}
