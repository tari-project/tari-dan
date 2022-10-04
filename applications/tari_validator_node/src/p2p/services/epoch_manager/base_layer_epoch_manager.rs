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

use std::convert::TryInto;

use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{Committee, Epoch, ValidatorNode},
    services::{
        epoch_manager::{EpochManagerError, ShardCommitteeAllocation},
        BaseNodeClient,
    },
    storage::global::GlobalDb,
};
use tari_dan_storage_sqlite::global::{models::validator_node::NewValidatorNode, SqliteGlobalDbBackendAdapter};

use crate::grpc::services::base_node_client::GrpcBaseNodeClient;

// const LOG_TARGET: &str = "tari_validator_node::epoch_manager::base_layer_epoch_manager";

#[derive(Clone)]
pub struct BaseLayerEpochManager {
    pub global_db: GlobalDb<SqliteGlobalDbBackendAdapter>,
    pub base_node_client: GrpcBaseNodeClient,
    current_epoch: Epoch,
}
impl BaseLayerEpochManager {
    pub fn new(
        global_db: GlobalDb<SqliteGlobalDbBackendAdapter>,
        base_node_client: GrpcBaseNodeClient,
        _id: CommsPublicKey,
    ) -> Self {
        Self {
            global_db,
            base_node_client,
            current_epoch: Epoch(0),
        }
    }

    pub async fn update_epoch(&mut self, tip: u64) -> Result<(), EpochManagerError> {
        let epoch = Epoch(tip / 10);
        if self.current_epoch.0 < epoch.0 {
            self.current_epoch = epoch;
        }

        // If the committee size is bigger than vns.len() then this function is broken.
        let mut base_node_client = self.base_node_client.clone();
        let mut vns = base_node_client.get_validator_nodes(epoch.0 * 10).await?;
        vns.sort_by(|a, b| a.shard_key.partial_cmp(&b.shard_key).unwrap());

        // insert the new VNs for this epoch in database
        let epoch_height = epoch.0;
        let new_vns = vns
            .into_iter()
            .map(|v| NewValidatorNode::new(epoch_height, v))
            .collect();
        self.global_db.insert_validator_nodes(new_vns)?;
        // let shard_key;
        // match base_node_client.clone().get_shard_key(epoch.0 * 10, &self.id).await {
        //     Ok(Some(key)) => shard_key = key,
        //     Ok(None) => {
        //         warn!(target: LOG_TARGET, "Validator node not found in the current epoch");
        //         return Ok(());
        //     },
        //     Err(e) => {
        //         warn!(target: LOG_TARGET, "This VN is not registered: {}", e);
        //         return Ok(());
        //     },
        // };
        //
        //
        // let p = vns.iter().position(|x| x.shard_key == shard_key).unwrap();
        // let begin = &vns[((vns.len() + p).saturating_sub(half_committee_size)) % vns.len()].shard_key;
        // let end = &vns[(p + half_committee_size) % vns.len()].shard_key;
        // let vns: Vec<ValidatorNode> = if p >= half_committee_size || p + half_committee_size >= vns.len() {
        //     //     This means the committee is wrapped around
        //     vns.iter()
        //         .filter(|&a| &a.shard_key <= begin || &a.shard_key >= end)
        //         .cloned()
        //         .collect()
        // } else {
        //     vns.iter()
        //         .filter(|&a| &a.shard_key >= begin || &a.shard_key <= end)
        //         .cloned()
        //         .collect()
        // };
        // *self.neighbours.entry(epoch.0).or_insert(vns.clone()) = vns.clone();
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

    #[allow(dead_code)]
    pub fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        let current_epoch = self.current_epoch();
        current_epoch.0 - 10 <= epoch.0 && epoch.0 <= current_epoch.0 + 10
    }

    #[allow(dead_code)]
    pub fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<ShardCommitteeAllocation<CommsPublicKey>>, EpochManagerError> {
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

    pub fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<CommsPublicKey>, EpochManagerError> {
        // retrieve the validator nodes for this epoch from database
        let db_vns = self.global_db.get_validator_nodes_per_epoch(epoch.0)?;
        if db_vns.is_empty() {
            return Err(EpochManagerError::NoEpochFound(epoch));
        }
        let vns: Vec<ValidatorNode> = db_vns
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        let half_committee_size = 4; // total committee = 7
        if vns.len() < half_committee_size * 2 {
            return Ok(Committee::new(vns.iter().map(|v| v.public_key.clone()).collect()));
        }

        let mid_point = vns.iter().filter(|x| x.shard_key <= shard).count();

        let begin = ((mid_point as i64 - half_committee_size as i64) % vns.len() as i64) as usize;
        let end = ((mid_point as i64 + half_committee_size as i64) % vns.len() as i64) as usize;
        let mut result = Vec::with_capacity(half_committee_size * 2);
        if mid_point < half_committee_size {
            result.extend_from_slice(&vns[0..mid_point as usize]);
            result.extend_from_slice(&vns[begin..]);
        } else {
            result.extend_from_slice(&vns[begin..mid_point as usize]);
        }

        if mid_point + half_committee_size >= vns.len() {
            result.extend_from_slice(&vns[mid_point as usize..]);
            result.extend_from_slice(&vns[0..end]);
        } else {
            result.extend_from_slice(&vns[mid_point as usize..end]);
        }

        Ok(Committee::new(result.into_iter().map(|v| v.public_key).collect()))
    }

    pub fn filter_to_local_shards(
        &self,
        epoch: Epoch,
        for_addr: &CommsPublicKey,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, EpochManagerError> {
        let mut result = vec![];
        for shard in available_shards {
            let committee = self.get_committee(epoch, *shard)?;
            if committee.contains(for_addr) {
                result.push(*shard);
            }
        }
        Ok(result)
    }
}
