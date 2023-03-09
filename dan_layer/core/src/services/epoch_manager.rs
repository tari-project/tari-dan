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

use std::{collections::HashMap, ops::Range};

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
    async fn get_validator_node_mmr(&self, epoch: Epoch) -> Result<ValidatorNodeBMT, EpochManagerError>;
    async fn get_validator_node_merkle_root(&self, epoch: Epoch) -> Result<Vec<u8>, EpochManagerError>;

    // TODO: Should be part of VN state machine
    async fn notify_scanning_complete(&self) -> Result<(), EpochManagerError>;
}

#[derive(Debug, Clone)]
pub struct RangeEpochManager<TAddr> {
    current_epoch: Epoch,
    #[allow(clippy::type_complexity)]
    epochs: HashMap<Epoch, Vec<(Range<ShardId>, Committee<TAddr>)>>,
    registered_vns: HashMap<Epoch, Vec<ValidatorNode<TAddr>>>,
}

impl<TAddr: NodeAddressable> RangeEpochManager<TAddr> {
    pub fn new(vn_keys: Vec<TAddr>, current: Range<ShardId>, committee: Vec<TAddr>) -> Self {
        let mut epochs = HashMap::new();
        epochs.insert(Epoch(0), vec![(current, Committee::new(committee))]);

        let mut registered_vns = HashMap::new();
        let vns: Vec<_> = vn_keys
            .into_iter()
            .map(|k| ValidatorNode {
                shard_key: ShardId::zero(),
                public_key: k,
            })
            .collect();
        registered_vns.insert(Epoch(0), vns);

        Self {
            current_epoch: Epoch(0),
            epochs,
            registered_vns,
        }
    }

    pub fn new_with_multiple(vn_keys: Vec<TAddr>, ranges: &[(Range<ShardId>, Vec<TAddr>)]) -> Self {
        let mut epochs = HashMap::new();
        epochs.insert(
            Epoch(0),
            ranges
                .iter()
                .cloned()
                .map(|(range, members)| (range, Committee::new(members)))
                .collect(),
        );
        let mut registered_vns = HashMap::new();
        let vns: Vec<_> = vn_keys
            .into_iter()
            .map(|k| ValidatorNode {
                shard_key: ShardId::zero(),
                public_key: k,
            })
            .collect();
        registered_vns.insert(Epoch(0), vns);
        Self {
            current_epoch: Epoch(0),
            epochs,
            registered_vns,
        }
    }
}

#[async_trait]
impl<TAddr: NodeAddressable> EpochManager<TAddr> for RangeEpochManager<TAddr> {
    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
        Ok(self.current_epoch)
    }

    async fn current_block_height(&self) -> Result<u64, EpochManagerError> {
        // We never change this or the epoch anyway
        Ok(0)
    }

    async fn get_validator_shard_key(&self, epoch: Epoch, addr: TAddr) -> Result<ShardId, EpochManagerError> {
        self.registered_vns
            .iter()
            .find_map(|(e, vns)| {
                if *e == epoch {
                    vns.iter().find_map(|vn| {
                        if vn.public_key == addr {
                            Some(vn.shard_key)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .ok_or(EpochManagerError::ValidatorNodeNotRegistered)
    }

    async fn is_epoch_valid(&self, epoch: Epoch) -> Result<bool, EpochManagerError> {
        Ok(self.current_epoch == epoch)
    }

    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<ShardCommitteeAllocation<TAddr>>, EpochManagerError> {
        let epoch = self.epochs.get(&epoch).ok_or(EpochManagerError::NoEpochFound(epoch))?;
        let mut result = vec![];
        for shard in shards {
            let mut found_committee = None;
            for (range, committee) in epoch {
                if range.contains(shard) {
                    found_committee = Some(committee.clone());
                    break;
                }
            }
            result.push(ShardCommitteeAllocation {
                shard_id: *shard,
                committee: found_committee.unwrap_or_else(Committee::empty),
            });
        }

        Ok(result)
    }

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, EpochManagerError> {
        let epoch = self.epochs.get(&epoch).ok_or(EpochManagerError::NoEpochFound(epoch))?;
        for (range, committee) in epoch {
            if range.contains(&shard) {
                return Ok(committee.clone());
            }
        }
        Err(EpochManagerError::NoCommitteeFound(shard))
    }

    async fn is_validator_in_committee_for_current_epoch(
        &self,
        shard: ShardId,
        identity: TAddr,
    ) -> Result<bool, EpochManagerError> {
        let epoch = self
            .epochs
            .get(&self.current_epoch)
            .ok_or(EpochManagerError::NoEpochFound(self.current_epoch))?;
        for (range, committee) in epoch {
            if range.contains(&shard) && committee.members.contains(&identity) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn filter_to_local_shards(
        &self,
        epoch: Epoch,
        addr: &TAddr,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, EpochManagerError> {
        let epoch = self.epochs.get(&epoch).ok_or(EpochManagerError::NoEpochFound(epoch))?;
        let mut result = vec![];
        for (range, committee) in epoch {
            for shard in available_shards {
                if range.contains(shard) && committee.contains(addr) {
                    result.push(*shard);
                }
            }
        }

        Ok(result)
    }

    async fn get_validator_nodes_per_epoch(
        &self,
        epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<TAddr>>, EpochManagerError> {
        let vns = self.registered_vns.get(&epoch).unwrap();
        Ok(vns.clone())
    }

    async fn get_validator_node_mmr(&self, epoch: Epoch) -> Result<ValidatorNodeBMT, EpochManagerError> {
        let vns = self
            .registered_vns
            .get(&epoch)
            .ok_or(EpochManagerError::NoEpochFound(epoch))?;
        let mut vn_bmt = Vec::with_capacity(vns.len());
        for vn in vns {
            vn_bmt
                .push(vn.node_hash().to_vec())
                .expect("Could not build the merkle mountain range of the VN set");
        }
        let vn_bmt = ValidatorNodeBMT::create(vn_bmt);
        Ok(vn_bmt)
    }

    async fn get_validator_node_merkle_root(&self, epoch: Epoch) -> Result<Vec<u8>, EpochManagerError> {
        let vn_mmr = self.get_validator_node_mmr(epoch).await?;
        Ok(vn_mmr.get_merkle_root().unwrap())
    }

    async fn notify_scanning_complete(&self) -> Result<(), EpochManagerError> {
        Ok(())
    }
}
