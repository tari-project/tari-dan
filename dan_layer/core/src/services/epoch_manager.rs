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

use async_trait::async_trait;
use tari_common_types::types::PublicKey;
use tari_comms::protocol::rpc::{RpcError, RpcStatus};
use tari_core::ValidatorNodeBMT;
use tari_dan_common_types::{Epoch, NodeAddressable, ShardId};
use thiserror::Error;

use crate::{
    models::{Committee, ValidatorNode},
    services::{base_node_error::BaseNodeError, ValidatorNodeClientError},
    storage::StorageError,
};

#[derive(Debug)]
pub struct ShardCommitteeAllocation<TAddr: NodeAddressable> {
    pub shard_id: ShardId,
    pub committee: Committee<TAddr>,
}

#[derive(Error, Debug)]
pub enum EpochManagerError {
    #[error("Could not receive from channel")]
    ReceiveError,
    #[error("Could not send to channel")]
    SendError,
    #[error("Base node errored: {0}")]
    BaseNodeError(#[from] BaseNodeError),
    #[error("No epoch found {0:?}")]
    NoEpochFound(Epoch),
    #[error("No committee found for shard {0:?}")]
    NoCommitteeFound(ShardId),
    #[error("Unexpected request")]
    UnexpectedRequest,
    #[error("Unexpected response")]
    UnexpectedResponse,
    #[error("Storage error: {0}")]
    StorageError(StorageError),
    #[error("No validator nodes found for current shard key")]
    ValidatorNodesNotFound,
    #[error("Validator node client error: {0}")]
    ValidatorNodeClientError(#[from] ValidatorNodeClientError),
    #[error("Rpc error: {0}")]
    RpcError(#[from] RpcError),
    #[error("Rpc status error: {0}")]
    RpcStatus(#[from] RpcStatus),
    #[error("No committee VNs found for shard {shard_id} and epoch {epoch}")]
    NoCommitteeVns { shard_id: ShardId, epoch: Epoch },
    #[error("This validator node is not registered")]
    ValidatorNodeNotRegistered,
    #[error("Base layer consensus constants not set")]
    BaseLayerConsensusConstantsNotSet,
    #[error("Base layer could not return shard key for {public_key} at height {block_height}")]
    ShardKeyNotFound { public_key: PublicKey, block_height: u64 },
    #[error("Received invalid state sync data from peer:{0}")]
    InvalidStateSyncData(#[from] anyhow::Error),
}

impl<T: Into<StorageError>> From<T> for EpochManagerError {
    fn from(e: T) -> Self {
        Self::StorageError(e.into())
    }
}

#[async_trait]
// TODO: Rename to reflect that it's a read only interface (e.g. EpochReader, EpochQuery)
pub trait EpochManager<TAddr: NodeAddressable>: Clone {
    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError>;
    async fn current_block_height(&self) -> Result<u64, EpochManagerError>;
    async fn get_validator_shard_key(&self, epoch: Epoch, addr: TAddr) -> Result<ShardId, EpochManagerError>;
    async fn is_epoch_valid(&self, epoch: Epoch) -> Result<bool, EpochManagerError>;
    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<ShardCommitteeAllocation<TAddr>>, EpochManagerError>;

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, EpochManagerError>;
    async fn is_validator_in_committee_for_current_epoch(
        &self,
        shard: ShardId,
        identity: TAddr,
    ) -> Result<bool, EpochManagerError>;
    /// Filters out from the available_shards, returning the ShardIds for committees for each available_shard that
    /// `for_addr` is part of.
    async fn filter_to_local_shards(
        &self,
        epoch: Epoch,
        for_addr: &TAddr,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, EpochManagerError>;

    async fn get_validator_nodes_per_epoch(&self, epoch: Epoch)
        -> Result<Vec<ValidatorNode<TAddr>>, EpochManagerError>;
    async fn get_validator_node_bmt(&self, epoch: Epoch) -> Result<ValidatorNodeBMT, EpochManagerError>;
    async fn get_validator_node_merkle_root(&self, epoch: Epoch) -> Result<Vec<u8>, EpochManagerError>;

    // TODO: Should be part of VN state machine
    async fn notify_scanning_complete(&self) -> Result<(), EpochManagerError>;
}
