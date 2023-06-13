//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{convert::TryInto, ops::RangeInclusive};

use futures::StreamExt;
use log::info;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::{committee::Committee, Epoch, ShardId};
use tari_dan_storage::{
    global::{GlobalDb, MetadataKey},
    models::SubstateShardData,
    ShardStore,
    ShardStoreReadTransaction,
    ShardStoreWriteTransaction,
    StorageError,
};
use tari_dan_storage_sqlite::{
    error::SqliteStorageError,
    global::SqliteGlobalDbAdapter,
    sqlite_shard_store_factory::SqliteShardStore,
};
use tari_epoch_manager::{
    base_layer::{EpochManagerError, EpochManagerHandle},
    EpochManager,
};
use tari_validator_node_rpc::{
    client::{TariCommsValidatorNodeClientFactory, ValidatorNodeClientFactory},
    ValidatorNodeRpcClientError,
};

const LOG_TARGET: &str = "tari::dan::committee_state_sync";

pub struct CommitteeStateSync {
    epoch_manager: EpochManagerHandle,
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    shard_store: SqliteShardStore,
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    node_public_key: CommsPublicKey,
}

impl CommitteeStateSync {
    pub fn new(
        epoch_manager: EpochManagerHandle,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
        shard_store: SqliteShardStore,
        global_db: GlobalDb<SqliteGlobalDbAdapter>,
        node_public_key: CommsPublicKey,
    ) -> Self {
        Self {
            epoch_manager,
            validator_node_client_factory,
            shard_store,
            global_db,
            node_public_key,
        }
    }

    pub async fn sync_state(&self, epoch: Epoch) -> Result<(), CommitteeStateSyncError> {
        if self.is_synced_to(epoch)? {
            info!(target: LOG_TARGET, "🌍️ Already synced to epoch {}", epoch);
            return Ok(());
        }
        info!(target: LOG_TARGET, "🌍️ Syncing epoch {}", epoch);

        // TODO: When switching epochs, we should sync from the previous committee. Then "at some point" it becomes
        //       better to sync from the current committee. Hmm...
        let previous_epoch = epoch;
        // let Some(previous_epoch) = epoch.checked_sub(Epoch(1)) else {
        //     info!(target: LOG_TARGET, "📋 Nothing to sync for epoch zero");
        //     return Ok(());
        // };

        // Get the shard range for our local committee
        let new_local_shard_range = match self
            .epoch_manager
            .get_local_shard_range(epoch, self.node_public_key.clone())
            .await
        {
            Ok(range) => range,
            Err(EpochManagerError::ValidatorNodeNotRegistered) => {
                info!(
                    target: LOG_TARGET,
                    "🌍️ Validator is not registered for epoch {}, Skipping state sync", epoch
                );
                return Ok(());
            },
            Err(err) => return Err(err.into()),
        };

        // Find previous epoch committee to contact for state sync.
        // Since the actual shard space (slice of pie) that this node is responsible
        // for is necessarily the same as previous committee's shard space in the previous epoch
        // we have to find nodes within a shard range.
        let prev_committee = self
            .epoch_manager
            .get_committee_for_shard_range(previous_epoch, new_local_shard_range.clone())
            .await?;

        info!(
            target: LOG_TARGET,
            "🌍 Syncing from {} peer(s) in range {} to {}",
            prev_committee.len(),
            new_local_shard_range.start(),
            new_local_shard_range.end()
        );

        // synchronize state with committee validator nodes
        // TODO: some mechanism for retry
        self.sync_peers_state(prev_committee, new_local_shard_range).await?;

        let mut tx = self.global_db.create_transaction()?;
        self.global_db
            .metadata(&mut tx)
            .set_metadata(MetadataKey::EpochManagerLastSyncedEpoch, &epoch)?;
        tx.commit()?;

        Ok(())
    }

    async fn sync_peers_state(
        &self,
        committee_vns: Committee<CommsPublicKey>,
        shard_range: RangeInclusive<ShardId>,
    ) -> Result<(), CommitteeStateSyncError> {
        let inventory = {
            let mut shard_db = self.shard_store.create_read_tx()?;
            shard_db.get_state_inventory()?
        };

        let inventory = inventory
            .into_iter()
            .map(tari_validator_node_rpc::proto::common::ShardId::from)
            .collect::<Vec<_>>();

        // the validator node has to sync state with vn's in the committee
        for sync_vn in committee_vns.members {
            if sync_vn == self.node_public_key {
                continue;
            }
            info!(
                target: LOG_TARGET,
                "🌍 Connecting to sync peer: {} and syncing from {} to {}",
                sync_vn,
                shard_range.start(),
                shard_range.end()
            );
            let mut sync_vn_client = self.validator_node_client_factory.create_client(&sync_vn);
            let mut sync_vn_rpc_client = sync_vn_client.client_connection().await?;
            let request = tari_validator_node_rpc::proto::rpc::VnStateSyncRequest {
                start_shard_id: Some((*shard_range.start()).into()),
                end_shard_id: Some((*shard_range.end()).into()),
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
                    msg.try_into().map_err(CommitteeStateSyncError::InvalidStateSyncData)?;

                // insert response state values in the shard db
                self.shard_store
                    .with_write_tx(|tx| tx.insert_substates(substate_shard_data))?;

                // increase node inventory
                // inventory.push(sync_vn_shard.into());
                substate_count += 1;
            }

            info!(
                target: LOG_TARGET,
                "🌍 Sync from peer {} complete. {} substate(s)", sync_vn, substate_count
            );
        }

        info!(target: LOG_TARGET, "🌍 Sync complete.");

        Ok(())
    }

    fn is_synced_to(&self, epoch: Epoch) -> Result<bool, CommitteeStateSyncError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut metadata = self.global_db.metadata(&mut tx);
        let last_sync_epoch = metadata.get_metadata::<Epoch>(MetadataKey::EpochManagerLastSyncedEpoch)?;
        Ok(last_sync_epoch.map(|ep| ep >= epoch).unwrap_or(false))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommitteeStateSyncError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Storage error: {0}")]
    StorageError(#[from] SqliteStorageError),
    #[error("Shard store error: {0}")]
    ShardStoreError(#[from] StorageError),
    #[error("Validator node client error: {0}")]
    ValidatorNodeClientError(#[from] ValidatorNodeRpcClientError),
    #[error("Invalid state sync data: {0}")]
    InvalidStateSyncData(anyhow::Error),
}
