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

use std::{
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
};

use async_trait::async_trait;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    hashing::MergedValidatorNodeMerkleProof,
    shard_bucket::ShardBucket,
    Epoch,
    NodeAddressable,
    ShardId,
};
use tari_dan_storage::global::models::ValidatorNode;
use tokio::sync::broadcast;

use crate::{EpochManagerError, EpochManagerEvent};

#[async_trait]
pub trait EpochManagerReader: Send + Sync {
    type Addr: NodeAddressable;

    async fn subscribe(&self) -> Result<broadcast::Receiver<EpochManagerEvent>, EpochManagerError>;

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<Self::Addr>, EpochManagerError>;
    async fn get_committee_within_shard_range(
        &self,
        epoch: Epoch,
        range: RangeInclusive<ShardId>,
    ) -> Result<Committee<Self::Addr>, EpochManagerError>;
    async fn get_validator_node(
        &self,
        epoch: Epoch,
        addr: &Self::Addr,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError>;

    /// Returns a list of validator nodes with the given epoch and public key. If any validator node is not found, an
    /// error is returned.
    async fn get_many_validator_nodes(
        &self,
        query: Vec<(Epoch, Self::Addr)>,
    ) -> Result<HashMap<(Epoch, Self::Addr), ValidatorNode<Self::Addr>>, EpochManagerError> {
        let mut results = HashMap::with_capacity(query.len());
        for (epoch, addr) in query {
            let vn = self.get_validator_node(epoch, &addr).await?;
            results.insert((epoch, addr.clone()), vn);
        }
        Ok(results)
    }

    async fn get_validator_set_merged_merkle_proof(
        &self,
        epoch: Epoch,
        validators: Vec<Self::Addr>,
    ) -> Result<MergedValidatorNodeMerkleProof, EpochManagerError>;

    async fn get_our_validator_node(&self, epoch: Epoch) -> Result<ValidatorNode<Self::Addr>, EpochManagerError>;
    async fn get_local_committee_shard(&self, epoch: Epoch) -> Result<CommitteeShard, EpochManagerError>;
    async fn get_committee_shard(&self, epoch: Epoch, shard: ShardId) -> Result<CommitteeShard, EpochManagerError>;

    async fn get_committee_shard_by_validator_address(
        &self,
        epoch: Epoch,
        addr: &Self::Addr,
    ) -> Result<CommitteeShard, EpochManagerError> {
        let validator = self.get_validator_node(epoch, addr).await?;
        self.get_committee_shard(epoch, validator.shard_key).await
    }

    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError>;
    async fn is_epoch_active(&self, epoch: Epoch) -> Result<bool, EpochManagerError>;

    async fn get_num_committees(&self, epoch: Epoch) -> Result<u32, EpochManagerError>;
    async fn get_vns(&self, epoch: Epoch) -> Result<Vec<ShardId>, EpochManagerError>;
    async fn get_committee_size(&self) -> Result<u32, EpochManagerError>;

    async fn get_committees_by_buckets(
        &self,
        epoch: Epoch,
        buckets: HashSet<ShardBucket>,
    ) -> Result<HashMap<ShardBucket, Committee<Self::Addr>>, EpochManagerError>;

    async fn get_local_committee(&self, epoch: Epoch) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let validator = self.get_our_validator_node(epoch).await?;
        let committee = self.get_committee(epoch, validator.shard_key).await?;
        Ok(committee)
    }

    async fn get_committee_by_validator_address(
        &self,
        epoch: Epoch,
        addr: &Self::Addr,
    ) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let validator = self.get_validator_node(epoch, addr).await?;
        let committee = self.get_committee(epoch, validator.shard_key).await?;
        Ok(committee)
    }

    /// Returns true if the validator is in the local committee for the given epoch.
    /// It is recommended that implementations override this method if they can provide a more efficient implementation.
    async fn is_validator_in_local_committee(
        &self,
        validator_addr: &Self::Addr,
        epoch: Epoch,
    ) -> Result<bool, EpochManagerError> {
        let committee = self.get_local_committee(epoch).await?;
        Ok(committee.contains(validator_addr))
    }

    async fn get_current_epoch_committee(&self, shard: ShardId) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let current_epoch = self.current_epoch().await?;
        self.get_committee(current_epoch, shard).await
    }

    async fn get_local_threshold_for_epoch(&self, epoch: Epoch) -> Result<usize, EpochManagerError> {
        let committee = self.get_local_committee_shard(epoch).await?;
        Ok(committee.quorum_threshold() as usize)
    }

    async fn is_this_validator_registered_for_epoch(&self, epoch: Epoch) -> Result<bool, EpochManagerError> {
        if !self.is_epoch_active(epoch).await? {
            return Ok(false);
        }

        // TODO: might want to improve this
        self.get_local_committee_shard(epoch)
            .await
            .map(|_| true)
            .or_else(|err| {
                if err.is_not_registered_error() {
                    Ok(false)
                } else {
                    Err(err)
                }
            })
    }
}
