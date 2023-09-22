//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use async_trait::async_trait;
use futures::StreamExt;
use log::*;
use tari_comms::{protocol::rpc::RpcError, types::CommsPublicKey};
use tari_consensus::traits::{SyncManager, SyncStatus};
use tari_dan_common_types::{committee::Committee, optional::Optional, Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{
        Block,
        HighQc,
        LastExecuted,
        LockedBlock,
        QuorumCertificate,
        SubstateUpdate,
        TransactionPoolRecord,
    },
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_validator_node_rpc::{
    client::{TariCommsValidatorNodeClientFactory, ValidatorNodeClientFactory},
    proto::rpc::{GetHighQcRequest, SyncBlocksRequest},
    rpc_service::ValidatorNodeRpcClient,
};

use crate::error::CommsRpcConsensusSyncError;

const LOG_TARGET: &str = "tari::dan::comms_rpc_state_sync";

pub struct CommsRpcStateSyncManager<TEpochManager, TStateStore> {
    epoch_manager: TEpochManager,
    state_store: TStateStore,
    client_factory: TariCommsValidatorNodeClientFactory,
}

impl<TEpochManager, TStateStore> CommsRpcStateSyncManager<TEpochManager, TStateStore>
where
    TEpochManager: EpochManagerReader<Addr = CommsPublicKey>,
    TStateStore: StateStore<Addr = CommsPublicKey>,
{
    pub fn new(
        epoch_manager: TEpochManager,
        state_store: TStateStore,
        client_factory: TariCommsValidatorNodeClientFactory,
    ) -> Self {
        Self {
            epoch_manager,
            state_store,
            client_factory,
        }
    }

    async fn get_sync_peers(
        &self,
        current_epoch: Epoch,
    ) -> Result<Committee<CommsPublicKey>, CommsRpcConsensusSyncError> {
        let this_vn = self.epoch_manager.get_our_validator_node(current_epoch).await?;
        let mut committee = self.epoch_manager.get_local_committee(current_epoch).await?;
        committee.members.retain(|m| *m != this_vn.address);
        committee.shuffle();
        Ok(committee)
    }

    async fn sync_with_peer(
        &self,
        addr: &CommsPublicKey,
        locked_block: &LockedBlock,
    ) -> Result<(), CommsRpcConsensusSyncError> {
        self.create_zero_block_if_required()?;
        let mut rpc_client = self.client_factory.create_client(addr);
        let mut client = rpc_client.client_connection().await?;

        info!(target: LOG_TARGET, "🌐 Syncing blocks from peer '{}' from Locked block {}", addr, locked_block);
        self.sync_blocks(&mut client, locked_block).await?;

        Ok(())
    }

    fn create_zero_block_if_required(&self) -> Result<(), CommsRpcConsensusSyncError> {
        let mut tx = self.state_store.create_write_tx()?;

        let zero_block = Block::zero_block();
        if !zero_block.exists(tx.deref_mut())? {
            debug!(target: LOG_TARGET, "Creating zero block");
            zero_block.justify().insert(&mut tx)?;
            zero_block.insert(&mut tx)?;
            zero_block.as_locked_block().set(&mut tx)?;
            zero_block.as_leaf_block().set(&mut tx)?;
            zero_block.as_last_executed().set(&mut tx)?;
            zero_block.justify().as_high_qc().set(&mut tx)?;
            zero_block.commit(&mut tx)?;
        }

        tx.commit()?;

        Ok(())
    }

    async fn sync_blocks(
        &self,
        client: &mut ValidatorNodeRpcClient,
        locked_block: &LockedBlock,
    ) -> Result<(), CommsRpcConsensusSyncError> {
        let mut stream = client
            .sync_blocks(SyncBlocksRequest {
                start_block_id: locked_block.block_id.as_bytes().to_vec(),
            })
            .await?;

        let mut counter = 0usize;

        let mut expected_height = locked_block.height + NodeHeight(1);

        while let Some(resp) = stream.next().await {
            let msg = resp.map_err(RpcError::from)?;
            let block = msg
                .block
                .map(Block::<CommsPublicKey>::try_from)
                .transpose()
                .map_err(CommsRpcConsensusSyncError::InvalidResponse)?
                .ok_or_else(|| {
                    CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                        "Peer returned an empty block response"
                    ))
                })?;
            if block.height() != expected_height {
                return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                    "Peer returned block at height {} but expected {}",
                    block.height(),
                    expected_height,
                )));
            }
            let qcs = msg
                .quorum_certificates
                .into_iter()
                .map(QuorumCertificate::<CommsPublicKey>::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommsRpcConsensusSyncError::InvalidResponse)?;
            let updates = msg
                .substate_updates
                .into_iter()
                .map(SubstateUpdate::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommsRpcConsensusSyncError::InvalidResponse)?;

            // TODO: Validate
            debug!(
                target: LOG_TARGET,
                "🌐 Received block {}, {} qcs and {} substate updates",
                block,
                qcs.len(),
                updates.len(),
            );
            counter += 1;
            if counter % 100 == 0 {
                info!(target: LOG_TARGET, "🌐 Syncing block {block}");
            }
            expected_height += NodeHeight(1);
            self.commit_block(block, qcs, updates)?;
        }

        info!(target: LOG_TARGET, "🌐 {} blocks synced", counter);

        Ok(())
    }

    fn commit_block(
        &self,
        block: Block<CommsPublicKey>,
        qcs: Vec<QuorumCertificate<CommsPublicKey>>,
        updates: Vec<SubstateUpdate<CommsPublicKey>>,
    ) -> Result<(), CommsRpcConsensusSyncError> {
        self.state_store.with_write_tx(|tx| {
            if !block.is_safe(tx.deref_mut())? {
                return Err(CommsRpcConsensusSyncError::BlockNotSafe { block_id: *block.id() });
            }

            block.justify().save(tx)?;
            block.save(tx)?;
            for qc in qcs {
                qc.save(tx)?;
            }
            block.update_nodes(
                tx,
                |_, _| Ok(()),
                |tx, last_executed, block| {
                    Self::mark_block_executed(tx, last_executed, block)?;
                    TransactionPoolRecord::remove_any(
                        tx,
                        block.commands().iter().filter_map(|cmd| cmd.accept()).map(|t| t.id),
                    )?;
                    Ok::<_, CommsRpcConsensusSyncError>(())
                },
            )?;
            let (ups, downs) = updates.into_iter().partition::<Vec<_>, _>(|u| u.is_create());
            // First do UPs then do DOWNs
            for update in ups {
                update.apply(tx, &block)?;
            }
            for update in downs {
                update.apply(tx, &block)?;
            }
            Ok(())
        })
    }

    fn mark_block_executed(
        tx: &mut <TStateStore as StateStore>::WriteTransaction<'_>,
        last_executed: &LastExecuted,
        block: &Block<TStateStore::Addr>,
    ) -> Result<(), CommsRpcConsensusSyncError> {
        if last_executed.height < block.height() {
            let parent = block.get_parent(tx.deref_mut())?;
            // Recurse to "catch up" any parent parent blocks we may not have executed
            block.commit(tx)?;
            Self::mark_block_executed(tx, last_executed, &parent)?;
            debug!(
                target: LOG_TARGET,
                "✅ COMMIT block {}, last executed height = {}",
                block,
                last_executed.height
            );
        }
        Ok(())
    }
}

#[async_trait]
impl<TEpochManager, TStateStore> SyncManager for CommsRpcStateSyncManager<TEpochManager, TStateStore>
where
    TEpochManager: EpochManagerReader<Addr = CommsPublicKey> + Send + Sync + 'static,
    TStateStore: StateStore<Addr = CommsPublicKey> + Send + Sync + 'static,
{
    type Error = CommsRpcConsensusSyncError;

    async fn check_sync(&self) -> Result<SyncStatus, Self::Error> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        let committee = self.get_sync_peers(current_epoch).await?;
        let mut highest_qc: Option<QuorumCertificate<CommsPublicKey>> = None;
        let mut num_succeeded = 0;
        let max_failures = committee.max_failures();
        for addr in committee {
            let mut rpc_client = self.client_factory.create_client(&addr);
            let mut client = match rpc_client.client_connection().await {
                Ok(client) => client,
                Err(err) => {
                    warn!(target: LOG_TARGET, "Failed to connect to peer {}: {}", addr, err);
                    continue;
                },
            };
            let result = client
                .get_high_qc(GetHighQcRequest {})
                .await
                .map_err(CommsRpcConsensusSyncError::RpcError)
                .and_then(|resp| {
                    resp.high_qc
                        .map(QuorumCertificate::<CommsPublicKey>::try_from)
                        .transpose()
                        .map_err(CommsRpcConsensusSyncError::InvalidResponse)?
                        .ok_or_else(|| {
                            CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                                "Peer returned an empty high qc"
                            ))
                        })
                });
            let remote_high_qc = match result {
                Ok(resp) => resp,
                Err(err) => {
                    warn!("Failed to get high qc from peer {}: {}", addr, err);
                    continue;
                },
            };

            num_succeeded += 1;
            if highest_qc
                .as_ref()
                .map(|qc| qc.block_height() < remote_high_qc.block_height())
                .unwrap_or(true)
            {
                // TODO: validate

                highest_qc = Some(remote_high_qc);
            }

            if num_succeeded == max_failures {
                break;
            }
        }

        if let Some(highest_qc) = highest_qc {
            let local_high_qc = self.state_store.with_read_tx(|tx| HighQc::get(tx).optional())?;
            let local_height = local_high_qc
                .as_ref()
                .map(|qc| qc.block_height())
                .unwrap_or(NodeHeight(0));
            if highest_qc.block_height() > local_height {
                info!(
                    target: LOG_TARGET,
                    "Highest QC from peers is at height {} and local high QC is at height {}",
                    highest_qc.block_height(),
                    local_height,
                );
                return Ok(SyncStatus::Behind);
            }
        }

        Ok(SyncStatus::UpToDate)
    }

    async fn sync(&self) -> Result<(), Self::Error> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        let committee = self.get_sync_peers(current_epoch).await?;

        let mut sync_error = None;
        for member in committee {
            // Refresh the HighQC each time because a partial sync could have been achieved from a peer
            let locked_block = self
                .state_store
                .with_read_tx(|tx| LockedBlock::get(tx).optional())?
                .unwrap_or_else(|| Block::<CommsPublicKey>::genesis().as_locked_block());

            match self.sync_with_peer(&member, &locked_block).await {
                Ok(()) => {
                    sync_error = None;
                    break;
                },
                Err(err) => {
                    warn!(target: LOG_TARGET, "Failed to sync with peer {}: {}", member, err);
                    sync_error = Some(err);
                    continue;
                },
            }
        }

        if let Some(err) = sync_error {
            return Err(err);
        }

        Ok(())
    }
}
