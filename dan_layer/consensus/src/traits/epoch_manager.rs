//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use async_trait::async_trait;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    Epoch, NodeAddressable, ShardId,
};

#[async_trait]
pub trait EpochManager: Send + Sync {
    type Addr: NodeAddressable;
    type Error;

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<Self::Addr>, Self::Error>;
    async fn get_our_validator_shard(&self, epoch: Epoch) -> Result<ShardId, Self::Error>;
    async fn get_our_validator_addr(&self, epoch: Epoch) -> Result<Self::Addr, Self::Error>;
    async fn get_local_committee_shard(&self, epoch: Epoch) -> Result<CommitteeShard, Self::Error>;
    async fn current_epoch(&self) -> Result<Epoch, Self::Error>;
    async fn is_epoch_active(&self, epoch: Epoch) -> Result<bool, Self::Error>;
    async fn get_num_committees(&self, epoch: Epoch) -> Result<u64, Self::Error>;
    async fn get_committees_by_buckets(
        &self,
        epoch: Epoch,
        buckets: HashSet<u64>,
    ) -> Result<Committee<Self::Addr>, Self::Error>;

    async fn get_local_committee(&self, epoch: Epoch) -> Result<Committee<Self::Addr>, Self::Error> {
        let validator_shard_id = self.get_our_validator_shard(epoch).await?;
        let committee = self.get_committee(epoch, validator_shard_id).await?;
        Ok(committee)
    }

    /// Returns true if the validator is in the local committee for the given epoch.
    /// It is recommended that implementations override this method if they can provide a more efficient implementation.
    async fn is_validator_in_local_committee(
        &self,
        validator_addr: &Self::Addr,
        epoch: Epoch,
    ) -> Result<bool, Self::Error> {
        let committee = self.get_local_committee(epoch).await?;
        Ok(committee.contains(validator_addr))
    }

    async fn is_current_epoch_active(&self) -> Result<bool, Self::Error> {
        let current_epoch = self.current_epoch().await?;
        self.is_epoch_active(current_epoch).await
    }

    async fn get_current_epoch_committee(&self, shard: ShardId) -> Result<Committee<Self::Addr>, Self::Error> {
        let current_epoch = self.current_epoch().await?;
        self.get_committee(current_epoch, shard).await
    }
}
