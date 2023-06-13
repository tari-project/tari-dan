//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    ops::{Range, RangeInclusive},
};

use tari_comms::async_trait;
use tari_core::ValidatorNodeBMT;
use tari_dan_common_types::{committee::Committee, Epoch, NodeAddressable, ShardId};
use tari_epoch_manager::{base_layer::EpochManagerError, EpochManager, ShardCommitteeAllocation, ValidatorNode};

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
    type Error = EpochManagerError;

    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
        Ok(self.current_epoch)
    }

    async fn current_block_height(&self) -> Result<u64, EpochManagerError> {
        // We never change this or the epoch anyway
        Ok(0)
    }

    async fn get_validator_node(&self, epoch: Epoch, addr: TAddr) -> Result<ValidatorNode<TAddr>, EpochManagerError> {
        self.registered_vns
            .iter()
            .find_map(|(e, vns)| {
                if *e == epoch {
                    vns.iter().find(|vn| vn.public_key == addr).cloned()
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

    async fn get_committee_for_shard_range(
        &self,
        _epoch: Epoch,
        _shard_range: RangeInclusive<ShardId>,
    ) -> Result<Committee<TAddr>, Self::Error> {
        todo!()
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

    async fn get_validator_node_bmt(&self, epoch: Epoch) -> Result<ValidatorNodeBMT, EpochManagerError> {
        let vns = self
            .registered_vns
            .get(&epoch)
            .ok_or(EpochManagerError::NoEpochFound(epoch))?;
        let mut vn_bmt = Vec::with_capacity(vns.len());
        for vn in vns {
            vn_bmt.push(vn.node_hash().to_vec());
        }
        let vn_bmt = ValidatorNodeBMT::create(vn_bmt);
        Ok(vn_bmt)
    }

    async fn get_validator_node_merkle_root(&self, epoch: Epoch) -> Result<Vec<u8>, EpochManagerError> {
        let vn_mmr = self.get_validator_node_bmt(epoch).await?;
        Ok(vn_mmr.get_merkle_root())
    }

    async fn get_local_shard_range(&self, _epoch: Epoch, _addr: TAddr) -> Result<RangeInclusive<ShardId>, Self::Error> {
        todo!()
    }

    async fn notify_scanning_complete(&self) -> Result<(), EpochManagerError> {
        Ok(())
    }
}
