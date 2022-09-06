use std::{collections::HashMap, ops::Range};

use async_trait::async_trait;
use tari_dan_common_types::ShardId;

use crate::{
    models::{Committee, Epoch},
    services::infrastructure_services::NodeAddressable,
};

#[async_trait]
pub trait EpochManager<TAddr: NodeAddressable>: Clone {
    async fn current_epoch(&self) -> Epoch;
    async fn is_epoch_valid(&self, epoch: Epoch) -> bool;
    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<TAddr>>)>, String>;
    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, String>;
    async fn get_shards(
        &self,
        epoch: Epoch,
        addr: &TAddr,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String>;
}

#[derive(Debug, Clone)]
pub struct RangeEpochManager<TAddr: NodeAddressable> {
    current_epoch: Epoch,
    epochs: HashMap<Epoch, Vec<(Range<ShardId>, Committee<TAddr>)>>,
}

impl<TAddr: NodeAddressable> RangeEpochManager<TAddr> {
    pub fn new(current: Range<ShardId>, committee: Vec<TAddr>) -> Self {
        let mut epochs = HashMap::new();
        epochs.insert(Epoch(0), vec![(current, Committee::new(committee))]);
        Self {
            current_epoch: Epoch(0),
            epochs,
        }
    }

    pub fn new_with_multiple(ranges: &[(Range<ShardId>, Vec<TAddr>)]) -> Self {
        let mut epochs = HashMap::new();
        epochs.insert(
            Epoch(0),
            ranges
                .iter()
                .map(|r| (r.0.clone(), Committee::new(r.1.clone())))
                .collect(),
        );
        Self {
            current_epoch: Epoch(0),
            epochs,
        }
    }
}

#[async_trait]
impl<TAddr: NodeAddressable> EpochManager<TAddr> for RangeEpochManager<TAddr> {
    async fn current_epoch(&self) -> Epoch {
        self.current_epoch
    }

    async fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        self.current_epoch == epoch
    }

    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<TAddr>>)>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch".to_string())?;
        let mut result = vec![];
        for shard in shards {
            let mut found_committee = None;
            for (range, committee) in epoch {
                if range.contains(shard) {
                    found_committee = Some(committee.clone());
                    break;
                }
            }
            result.push((*shard, found_committee.clone()));
        }

        Ok(result)
    }

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch".to_string())?;
        for (range, committee) in epoch {
            if range.contains(&shard) {
                return Ok(committee.clone());
            }
        }
        Err("Could not find a committee for that shard".to_string())
    }

    async fn get_shards(
        &self,
        epoch: Epoch,
        addr: &TAddr,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch".to_string())?;
        let mut result = vec![];
        for (range, committee) in epoch {
            for shard in available_shards {
                if range.contains(shard) {
                    if committee.contains(addr) {
                        result.push(*shard);
                    }
                }
            }
        }

        Ok(result)
    }
}
