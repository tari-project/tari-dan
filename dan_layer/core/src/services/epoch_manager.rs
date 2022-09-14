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
use tari_dan_common_types::ShardId;

use crate::{
    models::{Committee, Epoch},
    services::infrastructure_services::NodeAddressable,
};

#[async_trait]
pub trait EpochManager<TAddr: NodeAddressable>: Clone {
    async fn current_epoch(&mut self) -> Epoch;
    async fn is_epoch_valid(&mut self, epoch: Epoch) -> bool;
    async fn get_committees(
        &mut self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<TAddr>>)>, String>;
    async fn get_committee(&mut self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, String>;
    async fn get_shards(
        &mut self,
        epoch: Epoch,
        addr: &TAddr,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String>;
}

#[derive(Debug, Clone)]
pub struct RangeEpochManager<TAddr: NodeAddressable> {
    current_epoch: Epoch,
    #[allow(clippy::type_complexity)]
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
    async fn current_epoch(&mut self) -> Epoch {
        self.current_epoch
    }

    async fn is_epoch_valid(&mut self, epoch: Epoch) -> bool {
        self.current_epoch == epoch
    }

    async fn get_committees(
        &mut self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<TAddr>>)>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch")?;
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

    async fn get_committee(&mut self, epoch: Epoch, shard: ShardId) -> Result<Committee<TAddr>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch")?;
        for (range, committee) in epoch {
            if range.contains(&shard) {
                return Ok(committee.clone());
            }
        }
        Err("Could not find a committee for that shard".to_string())
    }

    async fn get_shards(
        &mut self,
        epoch: Epoch,
        addr: &TAddr,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String> {
        let epoch = self.epochs.get(&epoch).ok_or("No value for that epoch")?;
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
}
