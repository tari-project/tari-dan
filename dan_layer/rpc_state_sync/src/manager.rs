//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use async_trait::async_trait;
use futures::StreamExt;
use log::*;
use tari_consensus::traits::{ConsensusSpec, SyncManager, SyncStatus};
use tari_dan_common_types::{committee::Committee, optional::Optional, shard::Shard, Epoch, NodeHeight, PeerAddress};
use tari_dan_p2p::proto::rpc::{GetCheckpointRequest, GetCheckpointResponse, SyncStateRequest};
use tari_dan_storage::{
    consensus_models::{
        Block,
        EpochCheckpoint,
        LeafBlock,
        StateTransition,
        SubstateCreatedProof,
        SubstateDestroyedProof,
        SubstateRecord,
        SubstateUpdate,
    },
    StateStore,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::VersionedSubstateId;
use tari_validator_node_rpc::{
    client::{TariValidatorNodeRpcClientFactory, ValidatorNodeClientFactory},
    rpc_service::ValidatorNodeRpcClient,
};

use crate::error::CommsRpcConsensusSyncError;

const LOG_TARGET: &str = "tari::dan::comms_rpc_state_sync";

pub struct RpcStateSyncManager<TConsensusSpec: ConsensusSpec> {
    epoch_manager: TConsensusSpec::EpochManager,
    state_store: TConsensusSpec::StateStore,
    client_factory: TariValidatorNodeRpcClientFactory,
}

impl<TConsensusSpec> RpcStateSyncManager<TConsensusSpec>
where TConsensusSpec: ConsensusSpec<Addr = PeerAddress>
{
    pub fn new(
        epoch_manager: TConsensusSpec::EpochManager,
        state_store: TConsensusSpec::StateStore,
        client_factory: TariValidatorNodeRpcClientFactory,
    ) -> Self {
        Self {
            epoch_manager,
            state_store,
            client_factory,
        }
    }

    async fn establish_rpc_session(
        &self,
        addr: &PeerAddress,
    ) -> Result<ValidatorNodeRpcClient, CommsRpcConsensusSyncError> {
        let mut rpc_client = self.client_factory.create_client(addr);
        let client = rpc_client.client_connection().await?;
        Ok(client)
    }

    async fn fetch_epoch_checkpoint(&self, client: &mut ValidatorNodeRpcClient) -> Option<EpochCheckpoint> {
        match client.get_checkpoint(GetCheckpointRequest {}).await {
            Ok(GetCheckpointResponse {
                checkpoint: Some(checkpoint),
            }) => match EpochCheckpoint::try_from(checkpoint) {
                Ok(cp) => Some(cp),
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "Invalid message received from peer: {err}",
                    );
                    None
                },
            },
            Ok(GetCheckpointResponse { checkpoint: None }) => {
                warn!(
                    target: LOG_TARGET,
                    "Invalid response: no checkpoint provided. Attempting to fetch from another validator node",
                );
                None
            },
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to fetch checkpoint from peer: {err}",
                );
                None
            },
        }
    }

    async fn start_state_sync(
        &self,
        client: &mut ValidatorNodeRpcClient,
        checkpoint: EpochCheckpoint,
    ) -> Result<(), CommsRpcConsensusSyncError> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        let committee_info = self.epoch_manager.get_local_committee_info(current_epoch).await?;

        let last_state_transition_id = self.state_store.with_read_tx(|tx| StateTransition::get_last_id(tx))?;
        if current_epoch == last_state_transition_id.epoch() {
            info!(target: LOG_TARGET, "🛜Already up to date. No need to sync.");
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "🛜Syncing from state transition {last_state_transition_id}"
        );

        let mut state_stream = client
            .sync_state(SyncStateRequest {
                start_epoch: last_state_transition_id.epoch().as_u64(),
                start_shard: last_state_transition_id.shard().as_u32(),
                start_seq: last_state_transition_id.seq(),
                current_epoch: current_epoch.as_u64(),
                current_shard: committee_info.shard().as_u32(),
            })
            .await?;

        while let Some(result) = state_stream.next().await {
            let msg = match result {
                Ok(msg) => msg,
                Err(err) if err.is_not_found() => {
                    return Ok(());
                },
                Err(err) => {
                    return Err(err.into());
                },
            };

            info!(target: LOG_TARGET, "🛜 Next state updates batch of size {}", msg.transitions.len());

            self.state_store.with_write_tx(|tx| {
                for transition in msg.transitions {
                    let transition =
                        StateTransition::try_from(transition).map_err(CommsRpcConsensusSyncError::InvalidResponse)?;
                    info!(target: LOG_TARGET, "🛜 Applied state update {transition}");
                    if transition.id.epoch() >= current_epoch {
                        return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow!(
                            "Received state transition for epoch {} which is at or ahead of our current epoch {}.",
                            transition.id.epoch(),
                            current_epoch
                        )));
                    }

                    self.commit_update(tx, &checkpoint, transition)?;
                }

                // let current_version = block.justify().block_height().as_u64();
                // let next_version = block.height().as_u64();
                //
                // let changes = updates.iter().map(|update| match update {
                //     SubstateUpdate::Create(create) => SubstateTreeChange::Up {
                //         id: create.substate.substate_id.clone(),
                //         value_hash: hash_substate(&create.substate.substate_value, create.substate.version),
                //     },
                //     SubstateUpdate::Destroy(destroy) => SubstateTreeChange::Down {
                //         id: destroy.substate_id.clone(),
                //     },
                // });
                //
                // let mut store = ChainScopedTreeStore::new(epoch, shard, tx);
                // let mut tree = tari_state_tree::SpreadPrefixStateTree::new(&mut store);
                // let _state_root = tree.put_substate_changes(current_version, next_version, changes)?;

                Ok::<_, CommsRpcConsensusSyncError>(())
            })?;
        }

        Ok(())
    }

    pub fn commit_update<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        checkpoint: &EpochCheckpoint,
        transition: StateTransition,
    ) -> Result<(), StorageError> {
        match transition.update {
            SubstateUpdate::Create(SubstateCreatedProof { substate, created_qc }) => {
                SubstateRecord::new(
                    substate.substate_id,
                    substate.version,
                    substate.substate_value,
                    transition.id.shard(),
                    transition.id.epoch(),
                    NodeHeight(0),
                    *checkpoint.block().id(),
                    substate.created_by_transaction,
                    *created_qc.id(),
                )
                .create(tx)?;
            },
            SubstateUpdate::Destroy(SubstateDestroyedProof {
                substate_id,
                version,
                justify,
                destroyed_by_transaction,
            }) => {
                SubstateRecord::destroy(
                    tx,
                    VersionedSubstateId::new(substate_id, version),
                    transition.id.shard(),
                    transition.id.epoch(),
                    // TODO
                    checkpoint.block().height(),
                    justify.id(),
                    &destroyed_by_transaction,
                )?;
            },
        }
        Ok(())
    }

    async fn get_sync_committees(
        &self,
        current_epoch: Epoch,
    ) -> Result<HashMap<Shard, Committee<PeerAddress>>, CommsRpcConsensusSyncError> {
        // We are behind at least one epoch.
        // We get the current substate range, and we asks committees from previous epoch in this range to give us
        // data.
        let local_shard = self.epoch_manager.get_local_committee_info(current_epoch).await?;
        let range = local_shard.to_substate_address_range();
        let prev_epoch = current_epoch.saturating_sub(Epoch(1));
        info!(target: LOG_TARGET,"Previous epoch is {}", prev_epoch);
        let prev_num_committee = self.epoch_manager.get_num_committees(prev_epoch).await?;
        info!(target: LOG_TARGET,"Previous num committee {}", prev_num_committee);
        let start = range.start().to_shard(prev_num_committee);
        let end = range.end().to_shard(prev_num_committee);
        info!(target: LOG_TARGET,"Start: {}, End: {}", start, end);
        let committees = self
            .epoch_manager
            .get_committees_by_shards(
                prev_epoch,
                (start.as_u32()..=end.as_u32()).map(Shard::from).collect::<HashSet<_>>(),
            )
            .await?;
        Ok(committees)
    }
}

#[async_trait]
impl<TConsensusSpec> SyncManager for RpcStateSyncManager<TConsensusSpec>
where TConsensusSpec: ConsensusSpec<Addr = PeerAddress> + Send + Sync + 'static
{
    type Error = CommsRpcConsensusSyncError;

    async fn check_sync(&self) -> Result<SyncStatus, Self::Error> {
        let current_epoch = self.epoch_manager.current_epoch().await?;

        let leaf_epoch = self.state_store.with_read_tx(|tx| {
            let epoch = LeafBlock::get(tx)
                .optional()?
                .map(|leaf| Block::get(tx, leaf.block_id()))
                .transpose()?
                .map(|b| b.epoch())
                .unwrap_or(Epoch(0));
            Ok::<_, Self::Error>(epoch)
        })?;

        // We only sync if we're behind by an epoch. The current epoch is replayed in consensus.
        if current_epoch > leaf_epoch {
            info!(target: LOG_TARGET, "🛜Our current leaf block is behind the current epoch. Syncing...");
            return Ok(SyncStatus::Behind);
        }

        Ok(SyncStatus::UpToDate)
    }

    async fn sync(&mut self) -> Result<(), Self::Error> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        let prev_epoch_committees = self.get_sync_committees(current_epoch).await?;
        let our_vn = self.epoch_manager.get_our_validator_node(current_epoch).await?;

        let mut last_error = None;
        // Sync data from each committee in range of the committee we're joining.
        // NOTE: we don't have to worry about substates in address range because shard boundaries are fixed.
        for (shard, mut committee) in prev_epoch_committees {
            info!(target: LOG_TARGET, "🛜Syncing state for shard {shard} for epoch {}", current_epoch.saturating_sub(Epoch(1)));
            committee.shuffle();
            for addr in committee.members() {
                if our_vn.address == *addr {
                    continue;
                }
                let mut client = match self.establish_rpc_session(addr).await {
                    Ok(c) => c,
                    Err(err) => {
                        warn!(
                            target: LOG_TARGET,
                            "Failed to establish RPC session with vn {addr}: {err}. Attempting another VN if available"
                        );
                        last_error = Some(err);
                        continue;
                    },
                };

                let Some(checkpoint) = self.fetch_epoch_checkpoint(&mut client).await else {
                    continue;
                };
                info!(target: LOG_TARGET, "🛜 Checkpoint: {checkpoint}");

                if let Err(err) = self.start_state_sync(&mut client, checkpoint).await {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️Failed to sync state from {addr}: {err}. Attempting another peer if available"
                    );
                    last_error = Some(err);
                    continue;
                }
            }
        }

        last_error.map(Err).unwrap_or(Ok(()))
    }
}
