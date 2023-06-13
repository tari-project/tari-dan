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

use std::ops::RangeInclusive;

use async_trait::async_trait;
use tari_core::ValidatorNodeBMT;
use tari_dan_common_types::{committee::Committee, Epoch, NodeAddressable, ShardId};

use crate::validator_node::ValidatorNode;

#[derive(Debug)]
pub struct ShardCommitteeAllocation<TAddr> {
    pub shard_id: ShardId,
    pub committee: Committee<TAddr>,
}

// TODO: Rename to reflect that it's a read only interface (e.g. EpochReader, EpochQuery)
#[async_trait]
pub trait EpochManager<TAddr: NodeAddressable>: Clone {
    type Error;

    async fn current_epoch(&self) -> Result<Epoch, Self::Error>;
    async fn current_block_height(&self) -> Result<u64, Self::Error>;
    async fn get_validator_node(&self, epoch: Epoch, addr: TAddr) -> Result<ValidatorNode<TAddr>, Self::Error>;
    async fn is_epoch_valid(&self, epoch: Epoch) -> Result<bool, Self::Error>;
    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<ShardCommitteeAllocation<TAddr>>, Self::Error>;

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, Self::Error>;
    async fn get_committee_for_shard_range(
        &self,
        epoch: Epoch,
        shard_range: RangeInclusive<ShardId>,
    ) -> Result<Committee<TAddr>, Self::Error>;
    async fn is_validator_in_committee_for_current_epoch(
        &self,
        shard: ShardId,
        identity: TAddr,
    ) -> Result<bool, Self::Error>;
    /// Filters out from the available_shards, returning the ShardIds for committees for each available_shard that
    /// `for_addr` is part of.
    async fn filter_to_local_shards(
        &self,
        epoch: Epoch,
        for_addr: &TAddr,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, Self::Error>;

    async fn get_validator_nodes_per_epoch(&self, epoch: Epoch) -> Result<Vec<ValidatorNode<TAddr>>, Self::Error>;
    async fn get_validator_node_bmt(&self, epoch: Epoch) -> Result<ValidatorNodeBMT, Self::Error>;
    async fn get_validator_node_merkle_root(&self, epoch: Epoch) -> Result<Vec<u8>, Self::Error>;

    async fn get_local_shard_range(&self, epoch: Epoch, addr: TAddr) -> Result<RangeInclusive<ShardId>, Self::Error>;

    // TODO: Should be part of VN state machine
    async fn notify_scanning_complete(&self) -> Result<(), Self::Error>;
}
