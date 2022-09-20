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

use std::collections::HashMap;

use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{Committee, Epoch, ValidatorNode},
    services::{
        epoch_manager::{EpochManagerError, ShardCommitteeAllocation},
        BaseNodeClient,
    },
};

use crate::grpc::services::base_node_client::GrpcBaseNodeClient;

#[derive(Clone)]
pub struct BaseLayerEpochManager {
    pub base_node_client: GrpcBaseNodeClient,
    current_epoch: Epoch,
    id: CommsPublicKey,
    validators_per_epoch: HashMap<u64, Vec<ValidatorNode>>,
}
impl BaseLayerEpochManager {
    pub fn new(base_node_client: GrpcBaseNodeClient, id: CommsPublicKey) -> Self {
        Self {
            base_node_client,
            current_epoch: Epoch(0),
            id,
            validators_per_epoch: HashMap::new(),
        }
    }

    pub async fn update_epoch(&mut self, tip: u64) -> Result<(), EpochManagerError> {
        let epoch = Epoch(tip / 100);
        if self.current_epoch.0 < epoch.0 {
            self.current_epoch = epoch;
        }

        // If the committee size is bigger than vns.len() then this function is broken.
        let half_committee_size = 5;
        let mut base_node_client = self.base_node_client.clone();
        let mut vns = base_node_client.get_validator_nodes(epoch.0).await?;
        let shard_key = base_node_client.clone().get_shard_key(epoch.0, &self.id).await?;

        vns.sort_by(|a, b| a.shard_key.partial_cmp(&b.shard_key).unwrap());
        let p = vns.iter().position(|x| x.shard_key == shard_key).unwrap();
        let begin = &vns[(vns.len() + p - half_committee_size) % vns.len()].shard_key;
        let end = &vns[(p + half_committee_size) % vns.len()].shard_key;
        let vns: Vec<ValidatorNode> = if p >= half_committee_size || p + half_committee_size >= vns.len() {
            //     This means the committee is wrapped around
            vns.iter()
                .filter(|&a| &a.shard_key <= begin || &a.shard_key >= end)
                .map(|a| a.clone())
                .collect()
        } else {
            vns.iter()
                .filter(|&a| &a.shard_key >= begin || &a.shard_key <= end)
                .map(|a| a.clone())
                .collect()
        };
        *self.validators_per_epoch.entry(epoch.0).or_insert(vns.clone()) = vns.clone();
        Ok(())
    }

    pub fn current_epoch(&self) -> Epoch {
        // let tip = self
        //     .base_node_client
        //     .clone()
        //     .get_tip_info()
        //     .await
        //     .unwrap()
        //     .height_of_longest_chain;
        // Epoch(tip - 100)
        self.current_epoch
    }

    pub fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        let current_epoch = self.current_epoch();
        current_epoch.0 - 10 <= epoch.0 && epoch.0 <= current_epoch.0 + 10
    }

    pub fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<ShardCommitteeAllocation<CommsPublicKey>>, String> {
        let mut result = vec![];
        for &shard in shards {
            let committee = self.get_committee(epoch, shard).ok();
            result.push(ShardCommitteeAllocation {
                shard_id: shard,
                committee,
            });
        }
        Ok(result)
    }

    pub fn get_committee(&self, _epoch: Epoch, _shard: ShardId) -> Result<Committee<CommsPublicKey>, String> {
        todo!()
        // let validator_nodes = self
        //     .base_node_client
        //     .clone()
        //     .get_committee(epoch.0, shard.to_le_bytes().try_into().unwrap())
        //     .map_err(|s| format!("{:?}", s))?;
        // Ok(Committee::new(validator_nodes))
    }

    pub fn get_shards(
        &self,
        _epoch: Epoch,
        _addr: &CommsPublicKey,
        _available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String> {
        // If the committee size is bigger than vns.len() then this function is broken.
        // let half_committee_size = 5;
        // let mut base_node_client = self.base_node_client.clone();
        // let shard_key = base_node_client
        //     .clone()
        //     .get_shard_key(epoch.0, addr)
        //     .await
        //     .map_err(|s| format!("{:?}", s))?;
        // let mut vns = base_node_client
        //     .get_validator_nodes(epoch.0)
        //     .await
        //     .map_err(|s| format!("{:?}", s))?;
        // vns.sort_by(|a, b| a.shard_key.partial_cmp(&b.shard_key).unwrap());
        // let p = vns.iter().position(|x| x.shard_key == shard_key).unwrap();
        // let begin = &vns[(vns.len() + p - half_committee_size) % vns.len()].shard_key;
        // let end = &vns[(p + half_committee_size) % vns.len()].shard_key;
        // if p >= half_committee_size || p + half_committee_size >= vns.len() {
        //     // This means the committee is wrapped around
        //     Ok(available_shards
        //         .iter()
        //         .filter(|&a| a <= begin || a >= end)
        //         .map(|a| a.clone())
        //         .collect())
        // } else {
        //     Ok(available_shards
        //         .iter()
        //         .filter(|&a| a >= begin || a <= end)
        //         .map(|a| a.clone())
        //         .collect())
        // }
        todo!()
    }
}
