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

use std::convert::TryFrom;

use futures::StreamExt;
use tari_dan_common_types::{PayloadId, ShardId, SubstateState};
use tari_dan_core::{
    models::{NodeHeight, QuorumCertificate, SubstateShardData, TreeNodeHash, ValidatorNode},
    services::{epoch_manager::EpochManagerError, ValidatorNodeClientFactory},
    storage::shard_store::{ShardStoreFactory, ShardStoreTransaction},
};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::{SqliteShardStoreFactory, SqliteShardStoreTransaction};

use crate::{p2p, p2p::services::rpc_client::TariCommsValidatorNodeClientFactory, ValidatorNodeConfig};

pub struct PeerSyncManagerService {
    validator_node_config: ValidatorNodeConfig,
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
}

impl PeerSyncManagerService {
    pub(crate) fn new(
        validator_node_config: ValidatorNodeConfig,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    ) -> Self {
        Self {
            validator_node_config,
            validator_node_client_factory,
        }
    }

    fn get_vn_shard_db(&self) -> Result<SqliteShardStoreTransaction, EpochManagerError> {
        let data_dir = self.validator_node_config.data_dir.clone();
        let shard_db_factory =
            SqliteShardStoreFactory::try_create(data_dir).map_err(EpochManagerError::StorageError)?;
        let shard_db = shard_db_factory.create_tx().map_err(EpochManagerError::StorageError)?;
        Ok(shard_db)
    }

    pub(crate) async fn sync_peers_state(
        &self,
        committee_vns: Vec<ValidatorNode>,
        start_shard_id: ShardId,
        end_shard_id: ShardId,
        vn_shard_key: ShardId,
    ) -> Result<(), EpochManagerError> {
        let mut shard_db = self.get_vn_shard_db()?;

        let inventory = shard_db
            .get_state_inventory(start_shard_id, end_shard_id)
            .map_err(EpochManagerError::StorageError)?;

        let start_shard_id = p2p::proto::common::ShardId::from(start_shard_id);
        let end_shard_id = p2p::proto::common::ShardId::from(end_shard_id);

        let mut inventory = inventory
            .into_iter()
            .map(p2p::proto::common::ShardId::from)
            .collect::<Vec<p2p::proto::common::ShardId>>();

        // the validator node has to sync state with vn's in the committee
        for sync_vn in committee_vns {
            if sync_vn.shard_key == vn_shard_key {
                continue;
            }
            let mut sync_vn_client = self.validator_node_client_factory.create_client(&sync_vn.public_key);
            let mut sync_vn_rpc_client = sync_vn_client
                .create_connection()
                .await
                .map_err(EpochManagerError::ValidatorNodeClientError)?;

            let request = crate::p2p::proto::rpc::VnStateSyncRequest {
                start_shard_id: Some(start_shard_id.clone()),
                end_shard_id: Some(end_shard_id.clone()),
                inventory: inventory.clone(),
            };
            let mut vn_state_stream = sync_vn_rpc_client
                .vn_state_sync(request)
                .await
                .map_err(EpochManagerError::RpcError)?;

            while let Some(resp) = vn_state_stream.next().await {
                let msg = resp.map_err(EpochManagerError::RpcStatus)?;
                let sync_vn_shard = ShardId::try_from(msg.shard_id.ok_or(EpochManagerError::UnexpectedResponse)?)
                    .map_err(|_| EpochManagerError::UnexpectedResponse)?;
                let sync_vn_substate =
                    SubstateState::try_from(msg.substate_state.ok_or(EpochManagerError::UnexpectedResponse)?)
                        .map_err(|_| EpochManagerError::UnexpectedResponse)?;
                let sync_vn_node_height = NodeHeight::from(msg.node_height);

                let sync_vn_tree_node_hash = if msg.tree_node_hash.is_empty() {
                    None
                } else {
                    Some(
                        TreeNodeHash::try_from(msg.tree_node_hash)
                            .map_err(|_| EpochManagerError::UnexpectedResponse)?,
                    )
                };

                let sync_vn_payload_id =
                    PayloadId::try_from(msg.payload_id).map_err(|_| EpochManagerError::UnexpectedResponse)?;

                let sync_vn_certificate = if let Some(qc) = msg.certificate {
                    Some(QuorumCertificate::try_from(qc).map_err(|_| EpochManagerError::UnexpectedResponse)?)
                } else {
                    None
                };

                let substate_shard_data = SubstateShardData::new(
                    sync_vn_shard,
                    sync_vn_substate,
                    sync_vn_node_height,
                    sync_vn_tree_node_hash,
                    sync_vn_payload_id,
                    sync_vn_certificate,
                );

                // insert response state values in the shard db
                shard_db
                    .insert_substates(substate_shard_data)
                    .map_err(EpochManagerError::StorageError)?;

                // increase node inventory
                inventory.push(p2p::proto::common::ShardId::from(sync_vn_shard));
            }
        }

        Ok(())
    }
}
