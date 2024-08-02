//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::cmp;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::StreamExt;
use log::*;
use tari_consensus::{
    hotstuff::substate_store::{ShardScopedTreeStoreReader, ShardScopedTreeStoreWriter},
    traits::{ConsensusSpec, SyncManager, SyncStatus},
};
use tari_dan_common_types::{committee::Committee, optional::Optional, shard::Shard, Epoch, NodeHeight, PeerAddress};
use tari_dan_p2p::proto::rpc::{GetCheckpointRequest, GetCheckpointResponse, SyncStateRequest};
use tari_dan_storage::{
    consensus_models::{
        Block,
        EpochCheckpoint,
        LeafBlock,
        QcId,
        StateTransition,
        StateTransitionId,
        SubstateCreatedProof,
        SubstateDestroyedProof,
        SubstateRecord,
        SubstateUpdate,
    },
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_engine_types::substate::hash_substate;
use tari_epoch_manager::EpochManagerReader;
use tari_state_tree::{Hash, SpreadPrefixStateTree, SubstateTreeChange, Version};
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

    async fn fetch_epoch_checkpoint(
        &self,
        client: &mut ValidatorNodeRpcClient,
        current_epoch: Epoch,
    ) -> Result<Option<EpochCheckpoint>, CommsRpcConsensusSyncError> {
        match client
            .get_checkpoint(GetCheckpointRequest {
                current_epoch: current_epoch.as_u64(),
            })
            .await
        {
            Ok(GetCheckpointResponse {
                checkpoint: Some(checkpoint),
            }) => match EpochCheckpoint::try_from(checkpoint) {
                Ok(cp) => Ok(Some(cp)),
                Err(err) => Err(CommsRpcConsensusSyncError::InvalidResponse(err)),
            },
            Ok(GetCheckpointResponse { checkpoint: None }) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn start_state_sync(
        &self,
        client: &mut ValidatorNodeRpcClient,
        shard: Shard,
        checkpoint: EpochCheckpoint,
    ) -> Result<Version, CommsRpcConsensusSyncError> {
        let current_epoch = self.epoch_manager.current_epoch().await?;

        let last_state_transition_id = self
            .state_store
            .with_read_tx(|tx| StateTransition::get_last_id(tx, shard))
            .optional()?
            .unwrap_or_else(|| StateTransitionId::initial(shard));

        let persisted_version = self
            .state_store
            .with_read_tx(|tx| tx.state_tree_versions_get_latest(shard))?
            .unwrap_or(0);

        if current_epoch == last_state_transition_id.epoch() {
            info!(target: LOG_TARGET, "🛜Already up to date. No need to sync.");
            return Ok(persisted_version);
        }

        let mut current_version = persisted_version;

        // Minimum epoch we should request is 1 since Epoch(0) is the genesis epoch.
        let last_state_transition_id = StateTransitionId::new(
            cmp::max(last_state_transition_id.epoch(), Epoch(1)),
            last_state_transition_id.shard(),
            last_state_transition_id.seq(),
        );

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
            })
            .await?;

        while let Some(result) = state_stream.next().await {
            let msg = match result {
                Ok(msg) => msg,
                Err(err) if err.is_not_found() => {
                    return Ok(current_version);
                },
                Err(err) => {
                    return Err(err.into());
                },
            };

            if msg.transitions.is_empty() {
                return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow!(
                    "Received empty state transition batch."
                )));
            }

            self.state_store.with_write_tx(|tx| {
                let mut next_version = msg.transitions.first().expect("non-empty batch already checked").state_tree_version;

                info!(
                    target: LOG_TARGET,
                    "🛜 Next state updates batch of size {} (v{}-v{})",
                    msg.transitions.len(),
                    current_version,
                    msg.transitions.last().unwrap().state_tree_version,
                );

                let mut store = ShardScopedTreeStoreWriter::new(tx, shard);
                let mut tree_changes = vec![];


                for transition in msg.transitions {
                    let transition =
                        StateTransition::try_from(transition).map_err(CommsRpcConsensusSyncError::InvalidResponse)?;
                    if transition.id.shard() != shard {
                        return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow!(
                            "Received state transition for shard {} which is not the expected shard {}.",
                            transition.id.shard(),
                            shard
                        )));
                    }

                    if transition.state_tree_version < current_version {
                        return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow!(
                            "Received state transition with version {} that is not monotonically increasing (expected \
                             >= {})",
                            transition.state_tree_version,
                            persisted_version
                        )));
                    }

                    if transition.id.epoch().is_zero() {
                        return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow!(
                            "Received state transition with epoch 0."
                        )));
                    }

                    if transition.id.epoch() >= current_epoch {
                        return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow!(
                            "Received state transition for epoch {} which is at or ahead of our current epoch {}.",
                            transition.id.epoch(),
                            current_epoch
                        )));
                    }

                    let change = match &transition.update {
                        SubstateUpdate::Create(create) => SubstateTreeChange::Up {
                            id: create.substate.substate_id.clone(),
                            value_hash: hash_substate(&create.substate.substate_value, create.substate.version),
                        },
                        SubstateUpdate::Destroy(destroy) => SubstateTreeChange::Down {
                            id: destroy.substate_id.clone(),
                        },
                    };

                    info!(target: LOG_TARGET, "🛜 Applying state update {transition} (v{} to v{})", current_version, transition.state_tree_version);
                    if next_version != transition.state_tree_version {
                        let mut state_tree = SpreadPrefixStateTree::new(&mut store);
                        state_tree.put_substate_changes(Some(current_version).filter(|v| *v > 0), next_version, tree_changes.drain(..))?;
                        current_version = next_version;
                        next_version = transition.state_tree_version;
                    }
                    tree_changes.push(change);

                    self.commit_update(store.transaction(), &checkpoint, transition)?;
                }

                if !tree_changes.is_empty() {
                    let mut state_tree = SpreadPrefixStateTree::new(&mut store);
                    state_tree.put_substate_changes(Some(current_version).filter(|v| *v > 0), next_version, tree_changes.drain(..))?;
                }
                current_version = next_version;

                if current_version > 0 {
                    store.set_version(current_version)?;
                }

                Ok::<_, CommsRpcConsensusSyncError>(())
            })?;
        }

        Ok(current_version)
    }

    fn get_state_root_for_shard(&self, shard: Shard, version: Version) -> Result<Hash, CommsRpcConsensusSyncError> {
        self.state_store.with_read_tx(|tx| {
            let mut store = ShardScopedTreeStoreReader::new(tx, shard);
            let state_tree = SpreadPrefixStateTree::new(&mut store);
            let root_hash = state_tree.get_root_hash(version)?;
            Ok(root_hash)
        })
    }

    pub fn commit_update<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        checkpoint: &EpochCheckpoint,
        transition: StateTransition,
    ) -> Result<(), StorageError> {
        match transition.update {
            SubstateUpdate::Create(SubstateCreatedProof { substate }) => {
                SubstateRecord::new(
                    substate.substate_id,
                    substate.version,
                    substate.substate_value,
                    transition.id.shard(),
                    transition.id.epoch(),
                    NodeHeight(0),
                    *checkpoint.block().id(),
                    substate.created_by_transaction,
                    // TODO: correct QC ID
                    QcId::zero(),
                    // *created_qc.id(),
                )
                .create(tx)?;
            },
            SubstateUpdate::Destroy(SubstateDestroyedProof {
                substate_id,
                version,
                destroyed_by_transaction,
            }) => {
                SubstateRecord::destroy(
                    tx,
                    VersionedSubstateId::new(substate_id, version),
                    transition.id.shard(),
                    transition.id.epoch(),
                    // TODO
                    checkpoint.block().height(),
                    &QcId::zero(),
                    &destroyed_by_transaction,
                )?;
            },
        }

        Ok(())
    }

    async fn get_sync_committees(
        &self,
        current_epoch: Epoch,
    ) -> Result<Vec<(Shard, Committee<PeerAddress>)>, CommsRpcConsensusSyncError> {
        // We are behind at least one epoch.
        // We get the current substate range, and we asks committees from previous epoch in this range to give us
        // data.
        let local_info = self.epoch_manager.get_local_committee_info(current_epoch).await?;
        let prev_epoch = current_epoch.saturating_sub(Epoch(1));
        info!(target: LOG_TARGET,"Previous epoch is {}", prev_epoch);
        let committees = self
            .epoch_manager
            .get_committees_by_shard_group(prev_epoch, local_info.shard_group())
            .await?;

        // TODO: not strictly necessary to sort by shard but easier on the eyes in logs
        let mut committees = committees.into_iter().collect::<Vec<_>>();
        committees.sort_by_key(|(k, _)| *k);
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
            info!(target: LOG_TARGET, "🛜Syncing state for {shard} and {}", current_epoch.saturating_sub(Epoch(1)));
            committee.shuffle();
            for (addr, public_key) in committee {
                if our_vn.public_key == public_key {
                    continue;
                }
                let mut client = match self.establish_rpc_session(&addr).await {
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

                let checkpoint = match self.fetch_epoch_checkpoint(&mut client, current_epoch).await {
                    Ok(Some(cp)) => cp,
                    Ok(None) => {
                        // EDGE-CASE: This may occur because the previous epoch had not started consensus, typically in
                        // testing cases where transactions
                        warn!(
                            target: LOG_TARGET,
                            "❓No checkpoint for epoch {current_epoch}. This may mean that this is the first epoch in the network"
                        );
                        return Ok(());
                    },
                    Err(err) => {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️Failed to fetch checkpoint from {addr}: {err}. Attempting another peer if available"
                        );
                        last_error = Some(err);
                        continue;
                    },
                };
                info!(target: LOG_TARGET, "🛜 Checkpoint: {checkpoint}");

                match self.start_state_sync(&mut client, shard, checkpoint).await {
                    Ok(current_version) => {
                        // TODO: validate this MR against the checkpoint
                        let merkle_root = self.get_state_root_for_shard(shard, current_version)?;
                        info!(target: LOG_TARGET, "🛜Synced state for {shard} to v{current_version} with root {merkle_root}");
                    },
                    Err(err) => {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️Failed to sync state from {addr}: {err}. Attempting another peer if available"
                        );
                        last_error = Some(err);
                        continue;
                    },
                }
                break;
            }
        }

        last_error.map(Err).unwrap_or(Ok(()))
    }
}
