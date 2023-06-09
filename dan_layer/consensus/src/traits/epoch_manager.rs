//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use async_trait::async_trait;
use tari_dan_common_types::{committee::Committee, Epoch, ShardId};
use tari_dan_storage::consensus_models::ValidatorId;

#[async_trait]
pub trait EpochManager {
    type Error;

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<ValidatorId>, Self::Error>;
    async fn get_validator_id(&self, epoch: Epoch) -> Result<ValidatorId, Self::Error>;
    async fn current_epoch(&self) -> Result<Epoch, Self::Error>;
    async fn is_epoch_active(&self, epoch: Epoch) -> Result<bool, Self::Error>;
    async fn is_validator_in_local_committee(
        &self,
        validator_id: ValidatorId,
        epoch: Epoch,
    ) -> Result<bool, Self::Error>;
    async fn get_num_committees(&self, epoch: Epoch) -> Result<u64, Self::Error>;
    async fn get_committees_by_buckets(
        &self,
        epoch: Epoch,
        buckets: HashSet<u64>,
    ) -> Result<Committee<ValidatorId>, Self::Error>;

    async fn get_local_committee(&self, epoch: Epoch) -> Result<(ValidatorId, Committee<ValidatorId>), Self::Error> {
        let validator_id = self.get_validator_id(epoch).await?;
        let committee = self.get_committee(epoch, validator_id.shard_id()).await?;
        Ok((validator_id, committee))
    }

    async fn is_current_epoch_active(&self) -> Result<bool, Self::Error> {
        let current_epoch = self.current_epoch().await?;
        self.is_epoch_active(current_epoch).await
    }

    async fn get_current_epoch_committee(&self, shard: ShardId) -> Result<Committee<ValidatorId>, Self::Error> {
        let current_epoch = self.current_epoch().await?;
        self.get_committee(current_epoch, shard).await
    }
}
