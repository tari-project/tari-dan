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

use std::{convert::TryInto, sync::Arc};

use tari_comms::{types::CommsPublicKey, NodeIdentity};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_core::{
    consensus_constants::ConsensusConstants,
    models::{Committee, ValidatorNode},
    services::{
        epoch_manager::{EpochManagerError, ShardCommitteeAllocation},
        BaseNodeClient,
    },
    storage::DbFactory,
};
use tari_dan_storage::global::{DbValidatorNode, MetadataKey};
use tari_dan_storage_sqlite::SqliteDbFactory;
use tokio::sync::broadcast;

use super::{get_committee_shard_range, sync_peers::PeerSyncManagerService};
use crate::{
    grpc::services::base_node_client::GrpcBaseNodeClient,
    p2p::services::{
        epoch_manager::epoch_manager_service::EpochManagerEvent,
        rpc_client::TariCommsValidatorNodeClientFactory,
    },
    ValidatorNodeConfig,
};

// const LOG_TARGET: &str = "tari_validator_node::epoch_manager::base_layer_epoch_manager";

#[derive(Clone)]
pub struct BaseLayerEpochManager {
    db_factory: SqliteDbFactory,
    pub base_node_client: GrpcBaseNodeClient,
    consensus_constants: ConsensusConstants,
    current_epoch: Epoch,
    tx_events: broadcast::Sender<EpochManagerEvent>,
    node_identity: Arc<NodeIdentity>,
    validator_node_config: ValidatorNodeConfig,
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
}

impl BaseLayerEpochManager {
    pub fn new(
        db_factory: SqliteDbFactory,
        base_node_client: GrpcBaseNodeClient,
        consensus_constants: ConsensusConstants,
        _id: CommsPublicKey,
        tx_events: broadcast::Sender<EpochManagerEvent>,
        node_identity: Arc<NodeIdentity>,
        validator_node_config: ValidatorNodeConfig,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    ) -> Self {
        Self {
            db_factory,
            base_node_client,
            consensus_constants,
            current_epoch: Epoch(0),
            tx_events,
            node_identity,
            validator_node_config,
            validator_node_client_factory,
        }
    }

    pub async fn load_initial_state(&mut self) -> Result<(), EpochManagerError> {
        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db
            .create_transaction()
            .map_err(|e| EpochManagerError::StorageError(e.into()))?;
        let metadata = db.metadata(&tx);
        let current_epoch = metadata
            .get_metadata(MetadataKey::CurrentEpoch)
            .map_err(|e| EpochManagerError::StorageError(e.into()))?
            .map(|v| {
                let v2 = [v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]];
                Epoch(u64::from_le_bytes(v2))
            })
            .unwrap_or_else(|| Epoch(0));
        self.current_epoch = current_epoch;

        Ok(())
    }

    pub async fn update_epoch(&mut self, tip: u64) -> Result<(), EpochManagerError> {
        let epoch = Epoch::from_block_height(tip);
        if self.current_epoch >= epoch {
            // no need to update the epoch
            return Ok(());
        }

        // If the committee size is bigger than vns.len() then this function is broken.
        let height = epoch.to_height();
        let mut base_node_client = self.base_node_client.clone();
        let mut vns = base_node_client.get_validator_nodes(height).await?;
        vns.sort_by(|a, b| a.shard_key.partial_cmp(&b.shard_key).unwrap());

        // insert the new VNs for this epoch in the database
        self.insert_validator_nodes(epoch, vns.clone())?;
        {
            // after the end of the scope both db and tx are cleaned up
            let db = self.db_factory.get_or_create_global_db()?;
            let tx = db
                .create_transaction()
                .map_err(|e| EpochManagerError::StorageError(e.into()))?;
            let metadata = db.metadata(&tx);
            metadata
                .set_metadata(MetadataKey::CurrentEpoch, &epoch.0.to_le_bytes())
                .map_err(|e| EpochManagerError::StorageError(e.into()))?;
            db.commit(tx).map_err(|e| EpochManagerError::StorageError(e.into()))?;
        }
        self.current_epoch = epoch;
        self.tx_events
            .send(EpochManagerEvent::EpochChanged(epoch))
            .map_err(|_| EpochManagerError::SendError)?;

        let vn_shard_key = vns
            .iter()
            .find(|v| v.public_key == *self.node_identity.public_key())
            .ok_or(EpochManagerError::UnexpectedResponse)?
            .shard_key;

        // from current_shard_key we can get the corresponding vns committee
        let committee_size = self.consensus_constants.committee_size as usize;
        let committee_vns = self.get_committee_vns_from_shard_key(epoch, vn_shard_key)?;
        let (start_shard_id, end_shard_id) = get_committee_shard_range(committee_size, &committee_vns).into_inner();

        let peer_sync_service_manager = PeerSyncManagerService::new(
            self.validator_node_config.clone(),
            self.validator_node_client_factory.clone(),
        );

        // synchronize state with committee validator nodes
        peer_sync_service_manager
            .sync_peers_state(committee_vns, start_shard_id, end_shard_id, vn_shard_key)
            .await?;

        Ok(())
    }

    fn insert_validator_nodes(&self, epoch: Epoch, vns: Vec<ValidatorNode>) -> Result<(), EpochManagerError> {
        let epoch_height = epoch.0;
        let new_vns = vns
            .into_iter()
            .map(|v| DbValidatorNode {
                public_key: v.public_key.to_vec(),
                shard_key: v.shard_key.as_bytes().to_vec(),
                epoch: epoch_height,
            })
            .collect();
        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db
            .create_transaction()
            .map_err(|e| EpochManagerError::StorageError(e.into()))?;
        db.validator_nodes(&tx)
            .insert_validator_nodes(new_vns)
            .map_err(|e| EpochManagerError::StorageError(e.into()))?;
        db.commit(tx).map_err(|e| EpochManagerError::StorageError(e.into()))?;
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

    pub async fn last_registration_epoch(&self) -> Result<Option<Epoch>, EpochManagerError> {
        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db
            .create_transaction()
            .map_err(|e| EpochManagerError::StorageError(e.into()))?;
        let metadata = db.metadata(&tx);
        Ok(metadata
            .get_metadata(MetadataKey::LastEpochRegistration)
            .map_err(|e| EpochManagerError::StorageError(e.into()))?
            .map(|v| {
                let v2 = [v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]];
                Epoch(u64::from_le_bytes(v2))
            }))
    }

    pub async fn update_last_registration_epoch(&self, epoch: Epoch) -> Result<(), EpochManagerError> {
        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db
            .create_transaction()
            .map_err(|e| EpochManagerError::StorageError(e.into()))?;
        let metadata = db.metadata(&tx);
        metadata
            .set_metadata(MetadataKey::LastEpochRegistration, &epoch.to_le_bytes())
            .map_err(|e| EpochManagerError::StorageError(e.into()))?;
        db.commit(tx).map_err(|e| EpochManagerError::StorageError(e.into()))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        let current_epoch = self.current_epoch();
        current_epoch.0 <= epoch.0 + 10 && epoch.0 <= current_epoch.0 + 10
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

    pub fn get_committee_vns_from_shard_key(
        &self,
        epoch: Epoch,
        shard: ShardId,
    ) -> Result<Vec<ValidatorNode>, EpochManagerError> {
        // retrieve the validator nodes for this epoch from database
        let vns = self.get_validator_nodes_per_epoch(epoch)?;

        let half_committee_size = {
            let committee_size = self.consensus_constants.committee_size as usize;
            let v = committee_size / 2;
            if committee_size % 2 > 0 {
                v + 1
            } else {
                v
            }
        };
        if vns.len() < half_committee_size * 2 {
            return Ok(vns);
        }

        let mid_point = vns.iter().filter(|x| x.shard_key < shard).count();
        let begin =
            ((vns.len() as i64 + mid_point as i64 - (half_committee_size - 1) as i64) % vns.len() as i64) as usize;
        let end = ((mid_point as i64 + half_committee_size as i64) % vns.len() as i64) as usize;
        let mut result = Vec::with_capacity(half_committee_size * 2);
        if begin > mid_point {
            result.extend_from_slice(&vns[begin..]);
            result.extend_from_slice(&vns[0..mid_point as usize]);
        } else {
            result.extend_from_slice(&vns[begin..mid_point as usize]);
        }

        if end < mid_point {
            result.extend_from_slice(&vns[mid_point as usize..]);
            result.extend_from_slice(&vns[0..end]);
        } else {
            result.extend_from_slice(&vns[mid_point as usize..end]);
        }

        Ok(result)
    }

    pub fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<CommsPublicKey>, EpochManagerError> {
        let result = self.get_committee_vns_from_shard_key(epoch, shard)?;
        Ok(Committee::new(result.into_iter().map(|v| v.public_key).collect()))
    }

    pub fn get_validator_nodes_per_epoch(&self, epoch: Epoch) -> Result<Vec<ValidatorNode>, EpochManagerError> {
        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db
            .create_transaction()
            .map_err(|e| EpochManagerError::StorageError(e.into()))?;
        let db_vns = db
            .validator_nodes(&tx)
            .get_validator_nodes_per_epoch(epoch.0)
            .map_err(|e| EpochManagerError::StorageError(e.into()))?;
        if db_vns.is_empty() {
            return Err(EpochManagerError::NoEpochFound(epoch));
        }
        let mut vns: Vec<ValidatorNode> = db_vns
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        // TODO: sort on the db instead
        vns.sort_by(|a, b| a.shard_key.partial_cmp(&b.shard_key).unwrap());
        Ok(vns)
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
