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

use futures::StreamExt;
use log::info;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{SubstateShardData, ValidatorNode},
    services::{epoch_manager::EpochManagerError, ValidatorNodeClientFactory},
    storage::shard_store::{ShardStore, ShardStoreReadTransaction, ShardStoreWriteTransaction},
};

use crate::{p2p, p2p::services::rpc_client::TariCommsValidatorNodeClientFactory};

const LOG_TARGET: &str = "tari::validator_node::state_sync";

pub struct PeerSyncManagerService<TShardStore> {
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    shard_store: TShardStore,
}

impl<TShardStore: ShardStore> PeerSyncManagerService<TShardStore> {
    pub(crate) fn new(
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
        shard_store: TShardStore,
    ) -> Self {
        Self {
            validator_node_client_factory,
            shard_store,
        }
    }

    pub(crate) async fn sync_peers_state(
        &self,
        committee_vns: Vec<ValidatorNode<CommsPublicKey>>,
        start_shard_id: ShardId,
        end_shard_id: ShardId,
        vn_shard_key: ShardId,
    ) -> Result<(), EpochManagerError> {
        let inventory = {
            let shard_db = self.shard_store.create_read_tx()?;
            shard_db
                .get_state_inventory()
                .map_err(EpochManagerError::StorageError)?
        };

        let inventory = inventory
            .into_iter()
            .map(p2p::proto::common::ShardId::from)
            .collect::<Vec<_>>();

        // the validator node has to sync state with vn's in the committee
        for sync_vn in committee_vns {
            if sync_vn.shard_key == vn_shard_key {
                continue;
            }
            info!(target: LOG_TARGET, "🌍 Connecting to sync peer: {}", sync_vn.public_key);
            let mut sync_vn_client = self.validator_node_client_factory.create_client(&sync_vn.public_key);
            let mut sync_vn_rpc_client = sync_vn_client
                .create_connection()
                .await
                .map_err(EpochManagerError::ValidatorNodeClientError)?;

            let request = p2p::proto::rpc::VnStateSyncRequest {
                start_shard_id: Some(start_shard_id.into()),
                end_shard_id: Some(end_shard_id.into()),
                inventory: inventory.clone(),
            };
            let mut vn_state_stream = sync_vn_rpc_client
                .vn_state_sync(request)
                .await
                .map_err(EpochManagerError::RpcError)?;

            info!(target: LOG_TARGET, "🌍 Syncing...");
            let mut substate_count = 0;
            while let Some(resp) = vn_state_stream.next().await {
                let msg = resp.map_err(EpochManagerError::RpcStatus)?;
                let substate_shard_data: SubstateShardData =
                    msg.try_into().map_err(EpochManagerError::InvalidStateSyncData)?;

                // insert response state values in the shard db
                self.shard_store.with_write_tx(|tx| {
                    tx.insert_substates(substate_shard_data)
                        .map_err(EpochManagerError::StorageError)
                })?;

                // increase node inventory
                // inventory.push(sync_vn_shard.into());
                substate_count += 1;
            }

            info!(
                target: LOG_TARGET,
                "🌍 Sync from peer {} complete. {} substate(s)", sync_vn.public_key, substate_count
            );
        }

        info!(target: LOG_TARGET, "🌍 Sync complete.");

        Ok(())
    }
}
