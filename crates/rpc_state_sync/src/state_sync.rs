//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use anyhow::anyhow;
use futures::StreamExt;
use log::*;
use ootle_network::Network;
use rand::seq::SliceRandom;
use tari_consensus::{
    check_quorum_certificate_signatures,
    traits::{ConsensusSpec, SyncManager, SyncStatus},
};
use tari_consensus_types::{LeafBlock, ProposalCertificate};
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    VersionedSubstateId,
    VotePower,
    committee::{Committee, CommitteeMember},
    optional::Optional,
    shard::Shard,
};
use tari_ootle_p2p::{
    PeerAddress,
    proto::rpc::{
        GetCheckpointsRequest,
        GetCheckpointsResponse,
        GetHighQcRequest,
        ShardCursor,
        SyncStateRequest,
        sync_state_response,
    },
};
use tari_ootle_storage::{
    ShardScopedTreeStoreReader,
    ShardScopedTreeStoreWriter,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{
        BookkeepingEpochAgnosticRead,
        EpochCheckpoint,
        SubstateRecord,
        SubstateTransition,
        SubstateUpdateBatch,
        SubstateUpdateProof,
        SubstateValueFilterFlags,
    },
};
use tari_rpc_framework::{RpcError, RpcStatusCode};
use tari_state_tree::{SPARSE_MERKLE_PLACEHOLDER_HASH, SpreadPrefixStateTree, SubstateTreeChange, TreeHash, Version};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tari_validator_node_rpc::{
    STATE_SYNC_MAX_BATCH_SIZE,
    client::{TariValidatorNodeRpcClientFactory, ValidatorNodeClientFactory},
    rpc_service::ValidatorNodeRpcClient,
};

use crate::{error::RpcStateSyncError, stats::StateSyncStats};

/// Upper bound on checkpoints requested per peer. A peer stores one checkpoint per shard group it synced from
/// in an epoch, so this only needs to cover the previous epoch's committee count.
const MAX_CHECKPOINTS_PER_REQUEST: u32 = 32;

const LOG_TARGET: &str = "tari::ootle::rpc_state_sync";

pub struct RpcStateSyncClientProtocol<TConsensusSpec: ConsensusSpec> {
    network: Network,
    epoch_manager: TConsensusSpec::EpochManager,
    state_store: TConsensusSpec::StateStore,
    client_factory: TariValidatorNodeRpcClientFactory,
    signer_service: TConsensusSpec::SignerService,
    stats: StateSyncStats,
    skip_sync: bool,
}

impl<TConsensusSpec> RpcStateSyncClientProtocol<TConsensusSpec>
where TConsensusSpec: ConsensusSpec<Addr = PeerAddress>
{
    pub fn new(
        network: Network,
        epoch_manager: TConsensusSpec::EpochManager,
        state_store: TConsensusSpec::StateStore,
        client_factory: TariValidatorNodeRpcClientFactory,
        signer_service: TConsensusSpec::SignerService,
    ) -> Self {
        Self {
            network,
            epoch_manager,
            state_store,
            client_factory,
            signer_service,
            stats: StateSyncStats::default(),
            skip_sync: false,
        }
    }

    pub fn with_skip_sync(mut self, skip_sync: bool) -> Self {
        self.skip_sync = skip_sync;
        self
    }

    async fn establish_rpc_session(&self, addr: &PeerAddress) -> Result<ValidatorNodeRpcClient, RpcStateSyncError> {
        let rpc_client = self.client_factory.create_client(addr);
        let client = rpc_client.client_connection().await?;
        Ok(client)
    }

    async fn get_or_fetch_valid_epoch_checkpoint(
        &mut self,
        client: &mut ValidatorNodeRpcClient,
        for_shard_group: ShardGroup,
        prev_committee: &Committee<PeerAddress>,
        prev_epoch: Epoch,
    ) -> Result<Option<EpochCheckpoint>, RpcStateSyncError> {
        let valid_checkpoint = self
            .state_store
            .with_read_tx(|tx| EpochCheckpoint::get_by_shard_group(tx, prev_epoch, for_shard_group))
            .optional()?;

        if let Some(cp) = valid_checkpoint {
            info!(target: LOG_TARGET, "🛜 Checkpoint already fetched and valid: {cp}");
            return Ok(Some(cp));
        }

        self.stats.total_requests += 1;

        // A peer holds one checkpoint per shard group it synced from at prev_epoch, and the request has no
        // shard-group selector, so fetch a batch and pick the one for the requested group.
        let checkpoints = match client
            .get_checkpoints(GetCheckpointsRequest {
                from_epoch: Some(prev_epoch.into()),
                num_to_return: MAX_CHECKPOINTS_PER_REQUEST,
            })
            .await
        {
            Ok(GetCheckpointsResponse { checkpoints }) => checkpoints,
            Err(RpcError::RequestFailed(err)) if err.is_not_found() => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        for checkpoint in checkpoints {
            let checkpoint = EpochCheckpoint::try_from(checkpoint).map_err(RpcStateSyncError::InvalidResponse)?;
            let shard_group = checkpoint.checked_shard_group().map_err(|err| {
                RpcStateSyncError::InvalidResponse(anyhow!(
                    "Fetched checkpoint for epoch {} has invalid shard group: {err}",
                    checkpoint.epoch()
                ))
            })?;
            if checkpoint.epoch() != prev_epoch || shard_group != for_shard_group {
                continue;
            }
            info!(target: LOG_TARGET, "🛜 Checkpoint: {checkpoint}");
            self.validate_checkpoint(&checkpoint, prev_committee, prev_epoch)?;
            self.state_store.with_write_tx(|tx| checkpoint.save(tx))?;
            return Ok(Some(checkpoint));
        }

        Ok(None)
    }

    #[allow(clippy::too_many_lines)]
    async fn start_state_sync(
        &mut self,
        client: &mut ValidatorNodeRpcClient,
        shard: Shard,
        checkpoint: &EpochCheckpoint,
        mut maybe_persisted_state_version: Option<Version>,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let checkpoint_shard_root = checkpoint.get_shard_root(shard);

        let initial_local_state_root = self
            .state_store
            .with_read_tx(|tx| self.calculate_state_root_for_shard(tx, shard, maybe_persisted_state_version))?;
        if checkpoint_shard_root == initial_local_state_root {
            info!(target: LOG_TARGET, "Checkpoint state root indicates no further state changes. Nothing to sync for {shard}");
            return Ok(None);
        }

        // The stream is inclusive of start_state_version, so it must be the first version we have not
        // yet persisted. A persisted version must never be written a second time: JMT nodes are keyed
        // by (version, nibble_path), so rewriting a version overwrites live nodes and records those
        // very keys as stale at that version, and the stale-node GC then deletes them from under the
        // current tree. Bootstrapped genesis state is committed at version 0 and is never synced -
        // every node bootstraps it - so a freshly bootstrapped node starts at version 1, which is also
        // the minimum the peer accepts.
        let start_state_version = maybe_persisted_state_version.map_or(1, |v| v + 1);
        let mut last_state_version = start_state_version;
        info!(
            target: LOG_TARGET,
            "🛜Syncing from v{start_state_version}",
        );

        self.stats.total_requests += 1;
        let mut state_stream = client
            .sync_state(SyncStateRequest {
                cursors: vec![ShardCursor {
                    shard: shard.as_u32(),
                    start_state_version,
                }],
                until_epoch: Some(checkpoint.epoch().into()),
                value_filters: SubstateValueFilterFlags::all_substates().bits(),
                follow: false,
            })
            .await?;

        let mut tree_changes = vec![];
        let mut updates = vec![];
        let mut expected_state_version = None;

        // syncing states
        while let Some(result) = state_stream.next().await {
            let msg = result?;
            let batch = match msg.response {
                Some(sync_state_response::Response::Batch(batch)) => batch,
                Some(sync_state_response::Response::Complete(complete)) => {
                    if complete.shard != shard.as_u32() {
                        return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                            "Received completion marker for shard {} but requested {shard}",
                            complete.shard,
                        )));
                    }
                    // The stream always terminates with a completion marker. Verify the synced shard
                    // root against the trusted checkpoint at our last committed version: the producer
                    // streamed every transition up to the checkpoint epoch, so any gap to
                    // checkpoint_state_version is tree-only (no substate change) and the root at our
                    // last written version equals the checkpoint root. The marker's own version is the
                    // producer's claim and is not trusted as the verification target.
                    debug!(
                        target: LOG_TARGET,
                        "🛜 Stream complete for {shard} (peer reported v{}, locally committed v{})",
                        complete.synced_to_version,
                        maybe_persisted_state_version.unwrap_or(0),
                    );
                    let local_state_root = self.state_store.with_read_tx(|tx| {
                        self.calculate_state_root_for_shard(tx, shard, maybe_persisted_state_version)
                    })?;
                    if local_state_root != checkpoint_shard_root {
                        error!(
                            target: LOG_TARGET,
                            "❌ State root mismatch for {shard}. Checkpoint {expected} but got {actual}. Everything \
                             streamed so far is already committed; the next peer resumes after it.",
                            expected = checkpoint_shard_root,
                            actual = local_state_root,
                        );
                        return Err(RpcStateSyncError::StateRootMismatch {
                            expected: checkpoint_shard_root,
                            actual: local_state_root,
                        });
                    }
                    info!(
                        target: LOG_TARGET,
                        "🛜 ✅ State root for {shard} matches checkpoint: {local_state_root} (v{})",
                        maybe_persisted_state_version.unwrap_or(0),
                    );
                    return Ok(maybe_persisted_state_version);
                },
                None => {
                    return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                        "Received sync state response with no variant set."
                    )));
                },
            };

            if batch.shard != shard.as_u32() {
                return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                    "Received batch for shard {} but requested {shard}",
                    batch.shard,
                )));
            }
            if batch.updates.is_empty() {
                return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                    "Received empty state transition batch."
                )));
            }
            if batch.updates.len() > STATE_SYNC_MAX_BATCH_SIZE {
                return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                    "Received too many state updates in a batch: {}. Expected at most {}.",
                    batch.updates.len(),
                    STATE_SYNC_MAX_BATCH_SIZE
                )));
            }
            if batch.state_version < start_state_version {
                return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                    "Received state version {} that is less than the persisted state version {}.",
                    batch.state_version,
                    start_state_version
                )));
            }

            if expected_state_version.is_some_and(|v| v != batch.state_version) {
                return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                    "Received state version {} that is not the expected state version {}.",
                    batch.state_version,
                    expected_state_version.unwrap()
                )));
            }

            let state_version = batch.state_version;
            if state_version < last_state_version {
                return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                    "Received state version {} that is less than the last state version {}.",
                    state_version,
                    last_state_version
                )));
            }

            last_state_version = state_version;

            self.stats.total_transitions += batch.updates.len() as u64;

            tree_changes.reserve_exact(batch.updates.len());
            updates.reserve_exact(batch.updates.len());

            let updates_for_state_version = batch
                .updates
                .into_iter()
                .map(|t| SubstateUpdateProof::try_from(t).map_err(RpcStateSyncError::InvalidResponse));
            let msg_epoch = batch.epoch.map(Epoch::from).ok_or_else(|| {
                RpcStateSyncError::InvalidResponse(anyhow!("Received state transition with no epoch"))
            })?;

            info!(target: LOG_TARGET, "🛜 Buffering {} state update(s) (state version: v{})", updates_for_state_version.len(), state_version);
            for result in updates_for_state_version {
                let update = result?;
                let tree_change = extract_tree_change(self.network, &update, msg_epoch)?;

                debug!(target: LOG_TARGET, "🛜 -> state update (v{}) {}", state_version, update);
                tree_changes.push(tree_change);
                updates.push(update);
            }

            info!(target: LOG_TARGET, "🛜 Sync: {} state update(s), state version: v{}", updates.len(), state_version);

            if batch.has_more {
                info!(
                    target: LOG_TARGET,
                    "🛜 Received more state updates for v{}. Continuing to buffer...",
                    state_version
                );
                // Continue buffering
                // TODO: maximum possible state transitions within a single state version?
                expected_state_version = Some(state_version);
                continue;
            }

            expected_state_version = None;

            // Commit the buffered changes for this state version. The shard root is verified once, on
            // the terminal SyncComplete, against the trusted checkpoint.
            self.state_store.with_write_tx(|tx| {
                info!(
                    target: LOG_TARGET,
                    "🛜 Next state updates batch of size {} from v{}",
                    updates.len(),
                    state_version
                );

                let mut store = ShardScopedTreeStoreWriter::new(tx, shard);

                info!(target: LOG_TARGET, "🛜 {} state update(s) for v{}", updates.len(), state_version);
                self.commit_updates(
                    store.transaction(),
                    shard,
                    msg_epoch,
                    state_version,
                    updates.drain(..),
                )?;

                // Persist tree changes
                if !tree_changes.is_empty() {
                    let mut state_tree = SpreadPrefixStateTree::new(&mut store);
                    info!(target: LOG_TARGET, "🛜 Committing {} state tree changes batch v{}", tree_changes.len(), state_version);
                    state_tree.batch_put_substate_changes(maybe_persisted_state_version, state_version, tree_changes.drain(..))?;
                    maybe_persisted_state_version = Some(state_version);
                    store.set_state_version(state_version)?;
                }

                Ok::<_, RpcStateSyncError>(())
            })?;
        }

        // The stream ended without a SyncComplete - the peer closed early, so the sync is unverified.
        Err(RpcStateSyncError::InvalidResponse(anyhow!(
            "State sync stream for {shard} ended without a completion marker"
        )))
    }

    /// True if this node's committed state already matches `checkpoint` for every shard it is responsible for (its
    /// local shard group plus the global shard). Distinguishes a rolled-back node (complete state, checkpoint
    /// retained) from one whose first-time state sync was interrupted after the checkpoint was persisted but before
    /// the state finished streaming (checkpoint present, state incomplete).
    async fn local_state_matches_checkpoint(&self, checkpoint: &EpochCheckpoint) -> Result<bool, RpcStateSyncError> {
        let local_info = self.epoch_manager.get_local_committee_info(checkpoint.epoch()).await?;
        self.state_store.with_read_tx(|tx| {
            for shard in local_info.shard_group().shard_iter_with_global() {
                let version = tx.state_tree_versions_get_latest(shard)?;
                let local_root = self.calculate_state_root_for_shard(tx, shard, version)?;
                if local_root != checkpoint.get_shard_root(shard) {
                    return Ok(false);
                }
            }
            Ok(true)
        })
    }

    fn calculate_state_root_for_shard(
        &self,
        tx: &<TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        shard: Shard,
        version: Option<Version>,
    ) -> Result<TreeHash, RpcStateSyncError> {
        let Some(version) = version else {
            return Ok(SPARSE_MERKLE_PLACEHOLDER_HASH);
        };
        let mut store = ShardScopedTreeStoreReader::new(tx, shard);
        let state_tree = SpreadPrefixStateTree::new(&mut store);
        let root = state_tree.get_root_hash(version)?;
        Ok(root)
    }

    pub fn commit_updates<TTx: StateStoreWriteTransaction, I: IntoIterator<Item = SubstateUpdateProof>>(
        &self,
        tx: &mut TTx,
        shard: Shard,
        epoch: Epoch,
        state_version: Version,
        updates: I,
    ) -> Result<(), StorageError> {
        let mut batch = SubstateUpdateBatch::new(self.network, epoch);

        batch
            .with_transition(shard, state_version)
            .extend(updates.into_iter().map(|update| match update {
                SubstateUpdateProof::Create(create) => SubstateTransition::Up {
                    id: create.substate.substate_id,
                    version: create.substate.version,
                    substate_or_hash: create.substate.value,
                },
                SubstateUpdateProof::Destroy(destroy) => SubstateTransition::Down {
                    id: VersionedSubstateId::new(destroy.substate_id, destroy.version),
                },
            }));

        SubstateRecord::commit_batch(tx, batch)?;

        Ok(())
    }

    async fn get_sync_sources(
        &self,
        local_shard_group: ShardGroup,
        current_epoch: Epoch,
        our_vn_addr: &PeerAddress,
    ) -> Result<HashMap<ShardGroup, SyncSource>, RpcStateSyncError> {
        // We are behind at least one epoch.
        // We get the current substate range, and we ask committees from previous epoch in this range to give us
        // data.
        let prev_epoch = current_epoch
            .checked_sub(Epoch(1))
            .ok_or_else(|| RpcStateSyncError::NoCommittees(Epoch::zero()))?;
        info!(target: LOG_TARGET,"Previous epoch is {}", prev_epoch);
        // We want to get any committees from the previous epoch that overlap with our shard group in this epoch
        let prev_committees = self
            .epoch_manager
            .get_committees_overlapping_shard_group(prev_epoch, local_shard_group)
            .await?;

        if prev_committees.is_empty() {
            return Err(RpcStateSyncError::NoCommittees(prev_epoch));
        }

        // Every shard this sync requests lies in local_shard_group, and every member of our current committee
        // holds the previous epoch's state for exactly that range. After a split, members of other current
        // committees hold only their own sub-range of the previous shard group.
        let local_committee = self.epoch_manager.get_local_committee(current_epoch).await?;
        let sources = prev_committees
            .into_iter()
            .map(|(shard_group, signing_committee)| {
                (
                    shard_group,
                    SyncSource::new(signing_committee, &local_committee, our_vn_addr),
                )
            })
            .collect::<HashMap<_, _>>();
        info!(target: LOG_TARGET, "🛜 Querying {} committee(s) from epoch {}", sources.len(), prev_epoch);
        Ok(sources)
    }

    fn validate_checkpoint(
        &self,
        checkpoint: &EpochCheckpoint,
        committee: &Committee<PeerAddress>,
        epoch: Epoch,
    ) -> Result<(), RpcStateSyncError> {
        let quorum_threshold = committee.quorum_threshold();
        checkpoint
            .validate(epoch, quorum_threshold, |pk| {
                Ok(committee.get_power_by_public_key(pk).unwrap_or_else(VotePower::zero))
            })
            .map_err(|err| RpcStateSyncError::InvalidResponse(anyhow!("Checkpoint is not valid: {err}",)))?;

        info!(
            target: LOG_TARGET,
            "🛜 ✅ Checkpoint {} is valid",
            checkpoint,
        );

        Ok(())
    }

    fn is_checkpoint_temporarily_unavailable_error(err: &RpcStateSyncError, prev_epoch: Epoch) -> bool {
        match err {
            RpcStateSyncError::CheckpointNotAvailable { epoch } => *epoch == prev_epoch,
            RpcStateSyncError::RpcError(RpcError::RequestFailed(status)) => {
                let details = status.details();
                // Remote node count be behind in syncing the epoch oracle
                (status.as_status_code() == RpcStatusCode::BadRequest &&
                    details.contains(&format!("Peer requested checkpoint with epoch {}", prev_epoch))) ||
                    // Remote node is syncing
                    (status.as_status_code() == RpcStatusCode::General &&
                        (details.contains("Consensus is not running on this node") ||
                            details.contains("Node is still catching up to the epoch") ||
                            details.contains("Node is not in sync with the consensus epoch")))
            },
            _ => false,
        }
    }

    /// Synchronizes the given [`Shard`].
    async fn sync_shard(
        &mut self,
        shard: Shard,
        shard_group: ShardGroup,
        epoch: Epoch,
        source: &SyncSource,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let prev_epoch = epoch
            .checked_sub(Epoch(1))
            .ok_or_else(|| RpcStateSyncError::InvalidResponse(anyhow!("Epoch is zero")))?;
        info!(target: LOG_TARGET, "🛜 Syncing state for shard {shard} and epoch {}", prev_epoch);

        let mut last_hard_error = None;
        let mut saw_unavailable_checkpoint = false;

        for member in &source.serving_peers {
            let mut client = match self.establish_rpc_session(&member.address).await {
                Ok(c) => c,
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "Failed to establish RPC session with vn {member}: {err}. Attempting another VN if available"
                    );
                    last_hard_error = Some(err);
                    continue;
                },
            };

            // fetch checkpoint
            let checkpoint = match self
                .get_or_fetch_valid_epoch_checkpoint(&mut client, shard_group, &source.signing_committee, prev_epoch)
                .await
            {
                Ok(Some(cp)) => cp,
                Ok(None) => {
                    warn!(
                        target: LOG_TARGET,
                        "❓️ No checkpoint for epoch {prev_epoch} from {member}. Previous committee exists, so state \
                         sync will retry instead of proceeding without a checkpoint.",
                    );
                    saw_unavailable_checkpoint = true;
                    continue;
                },
                Err(err) => {
                    if Self::is_checkpoint_temporarily_unavailable_error(&err, prev_epoch) {
                        saw_unavailable_checkpoint = true;
                        warn!(
                            target: LOG_TARGET,
                            "⚠️Checkpoint for epoch {prev_epoch} is not yet available from {member}: {err}. \
                             Attempting another peer if available"
                        );
                        continue;
                    }

                    warn!(
                        target: LOG_TARGET,
                        "⚠️Failed to fetch checkpoint from {member}: {err}. Attempting another peer if available"
                    );
                    last_hard_error = Some(err);
                    continue;
                },
            };

            let maybe_persisted_state_version = self
                .state_store
                .with_read_tx(|tx| tx.state_tree_versions_get_latest(shard))?;

            match self
                .start_state_sync(&mut client, shard, &checkpoint, maybe_persisted_state_version)
                .await
            {
                Ok(maybe_version) => {
                    return Ok(maybe_version);
                },
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️Failed to sync state from {member}: {err}. Attempting another peer if available"
                    );
                    last_hard_error = Some(err);
                    continue;
                },
            }
        }

        if let Some(err) = last_hard_error {
            return Err(err);
        }

        if saw_unavailable_checkpoint {
            return Err(RpcStateSyncError::CheckpointNotAvailable { epoch: prev_epoch });
        }

        Err(RpcStateSyncError::SyncFailedAllPeers {
            num_peers: source.serving_peers.len(),
        })
    }

    async fn sync_global_shard(
        &mut self,
        current_epoch: Epoch,
        sources: &HashMap<ShardGroup, SyncSource>,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let mut last_error = None;

        for (sg, source) in sources {
            // Any previous-epoch checkpoint carries the global shard root, so the first shard group to succeed
            // justifies the whole global shard sync.
            let result = self.sync_shard(Shard::global(), *sg, current_epoch, source).await;
            match result {
                Ok(maybe_version) => {
                    let Some(version) = maybe_version else {
                        info!(target: LOG_TARGET, "🛜 No state changes for global shard");
                        return Ok(None);
                    };
                    info!(target: LOG_TARGET, "🛜 Synced global shard to v{}", version);
                    return Ok(Some(version));
                },
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Failed to sync global shard from {sg}: {err}. Attempting another committee if available"
                    );
                    last_error = Some(err);
                },
            }
        }

        if let Some(err) = last_error {
            return Err(err);
        }

        Err(RpcStateSyncError::SyncFailedAllPeers {
            num_peers: sources.values().map(|s| s.serving_peers.len()).sum(),
        })
    }

    async fn sync_inner(&mut self, target_epoch: Option<Epoch>) -> Result<(), RpcStateSyncError> {
        let timer = Instant::now();
        // Use the caller-provided target if any (typically the highest epoch resolved by a
        // stall-recovery probe), otherwise fall back to the oracle's current epoch.
        let current_epoch = match target_epoch {
            Some(probed) => {
                info!(
                    target: LOG_TARGET,
                    "🛜 Sync target from caller: epoch {probed}",
                );
                probed
            },
            None => self.epoch_manager.current_epoch().await?,
        };
        let our_vn = self.epoch_manager.get_our_validator_node(current_epoch).await?;
        let local_info = self.epoch_manager.get_local_committee_info(current_epoch).await?;
        let sync_sources = match self
            .get_sync_sources(local_info.shard_group(), current_epoch, &our_vn.address)
            .await
        {
            Ok(sources) => sources,
            Err(RpcStateSyncError::NoCommittees(prev_epoch)) => {
                info!(target: LOG_TARGET, "No committees for the previous epoch {prev_epoch}. This is the first committee.");
                return Ok(());
            },
            Err(err) => return Err(err),
        };

        // Edge case: we're the only VN in a previous committee
        if sync_sources.len() == 1 &&
            sync_sources.values().all(|source| {
                source.signing_committee.len() == 1 &&
                    source
                        .signing_committee
                        .address_iter()
                        .all(|addr| *addr == our_vn.address)
            })
        {
            info!(target: LOG_TARGET, "This node is the only Validator in the previous committee - no need to sync.");
            return Ok(());
        }

        let local_shard_group = local_info.shard_group();

        self.sync_global_shard(current_epoch, &sync_sources).await?;

        // Sync data from each committee in range of the committee we're joining.
        // NOTE: we don't have to worry about substates in address range because shard boundaries are fixed.
        for (shard_group, source) in sync_sources {
            let Some(intersect_shard_group) = shard_group.intersection(&local_shard_group) else {
                warn!(
                    target: LOG_TARGET,
                    "❗️ Shard group {shard_group} does not intersect with our shard group {local_shard_group}. Skipping."
                );
                continue;
            };
            for shard in intersect_shard_group.shard_iter() {
                self.sync_shard(shard, shard_group, current_epoch, &source).await?;
            }
        }

        self.stats.total_time = timer.elapsed();
        Ok(())
    }
}

/// Where to sync one shard group's state from. Checkpoint validity depends only on the quorum signatures of
/// the committee that produced it, so who serves the bytes is chosen for availability rather than trust.
struct SyncSource {
    /// The previous epoch's committee for the shard group. Its quorum threshold and voting powers are the
    /// only thing a fetched checkpoint is validated against.
    signing_committee: Committee<PeerAddress>,
    /// Peers to request the checkpoint and state from, in order of preference, excluding this node.
    /// Members of our current committee are running consensus and hold the previous epoch's state for our
    /// whole shard group (either as continuing members or because they synced it from the same checkpoint),
    /// so they come first; signing-committee members that left at the epoch boundary are the fallback.
    serving_peers: Vec<CommitteeMember<PeerAddress>>,
}

impl SyncSource {
    fn new(
        signing_committee: Committee<PeerAddress>,
        local_committee: &Committee<PeerAddress>,
        our_vn_addr: &PeerAddress,
    ) -> Self {
        let signing_addrs = signing_committee.address_iter().collect::<HashSet<_>>();
        let local_addrs = local_committee.address_iter().collect::<HashSet<_>>();

        let mut continuing = Vec::new();
        let mut joined = Vec::new();
        for member in local_committee.iter().filter(|m| m.address != *our_vn_addr) {
            if signing_addrs.contains(&member.address) {
                continuing.push(member.clone());
            } else {
                joined.push(member.clone());
            }
        }
        let departed = signing_committee
            .iter()
            .filter(|m| m.address != *our_vn_addr && !local_addrs.contains(&m.address))
            .cloned()
            .collect::<Vec<_>>();

        // Shuffling within each tier spreads load across equally preferred peers.
        let mut rng = rand::rng();
        let serving_peers = [continuing, joined, departed]
            .into_iter()
            .flat_map(|mut tier| {
                tier.shuffle(&mut rng);
                tier
            })
            .collect();

        Self {
            signing_committee,
            serving_peers,
        }
    }
}

enum ProbeOutcome {
    /// At least one peer returned a verified QC for an epoch strictly higher than our leaf.
    HigherQcSeen { epoch: Epoch },
    /// A quorum of distinct committee members (by stake-weighted power) attested no higher QC
    /// than our leaf's epoch.
    QuorumAtLeaf { attested_power: VotePower },
    /// Not enough committee power responded to make either decision.
    Inconclusive {
        attested_power: VotePower,
        quorum_threshold: VotePower,
    },
}

impl<TConsensusSpec> RpcStateSyncClientProtocol<TConsensusSpec>
where TConsensusSpec: ConsensusSpec<Addr = PeerAddress> + Send + Sync + 'static
{
    /// The shard group this node serves at `epoch`, when a committee change has moved it off the one
    /// `leaf` was committed under.
    ///
    /// A node in that position holds none of the state for the shards it has been handed, which no
    /// agreement with the committee it is leaving can tell it: that committee is level with it on the
    /// shards they shared, and silent about the rest.
    async fn shard_group_moved_since(
        &self,
        epoch: Epoch,
        leaf: &LeafBlock,
    ) -> Result<Option<ShardGroup>, RpcStateSyncError> {
        let info = self.epoch_manager.get_local_committee_info(epoch).await.optional()?;
        Ok(info
            .map(|info| info.shard_group())
            .filter(|shard_group| *shard_group != leaf.shard_group()))
    }

    /// Probe committee members for their highest QCs and classify the result. See `ProbeOutcome`.
    #[expect(clippy::too_many_lines)]
    async fn probe_high_qcs_at_leaf(
        &self,
        committee: &Committee<PeerAddress>,
        leaf: &LeafBlock,
        our_addr: &PeerAddress,
    ) -> Result<ProbeOutcome, RpcStateSyncError> {
        let leaf_epoch = leaf.epoch();
        let quorum_threshold = committee.quorum_threshold();

        // Track the highest verified QC we see so we never decide "no higher QC exists" based
        // on weaker evidence than what we already hold locally.
        let mut highest_height_seen = leaf.height();
        let mut highest_epoch_seen = leaf_epoch;

        let mut counted_pks: HashSet<RistrettoPublicKeyBytes> = HashSet::new();
        let mut attested_power = VotePower::zero();

        // Pre-credit our own attestation. We hold the leaf QC ourselves and by definition have no
        // QC higher than our leaf's epoch — that's the question the probe is asking. Excluding
        // ourselves makes the quorum unreachable on any committee where total power equals
        // quorum threshold (e.g. 4 members with one zero-power node: total=3, threshold=3, and
        // peer responses can never reach 3 without including us).
        if let Some(self_member) = committee.iter().find(|m| &m.address == our_addr) {
            counted_pks.insert(self_member.public_key);
            attested_power += self_member.vote_power;
        }

        // Iterate committee members in a randomised order so that under repeated probes we
        // sample broadly across the committee rather than always hitting the same f peers.
        for member in committee.shuffled() {
            if &member.address == our_addr {
                continue;
            }

            let mut client = match self.establish_rpc_session(&member.address).await {
                Ok(c) => c,
                Err(e) => {
                    debug!(target: LOG_TARGET, "🛜 Probe: skipping {} (rpc session failed: {})", member.address, e);
                    continue;
                },
            };

            let response = match client
                .get_high_qc(GetHighQcRequest {
                    from_epoch: Some(leaf_epoch.into()),
                })
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    debug!(target: LOG_TARGET, "🛜 Probe: {} returned error: {}", member.address, e);
                    continue;
                },
            };

            let Some(proto_qc) = response.high_qc else {
                debug!(target: LOG_TARGET, "🛜 Probe: {} returned no QC", member.address);
                continue;
            };

            let qc = match ProposalCertificate::try_from(proto_qc) {
                Ok(qc) => qc,
                Err(e) => {
                    debug!(target: LOG_TARGET, "🛜 Probe: {} returned malformed QC: {}", member.address, e);
                    continue;
                },
            };

            // Stale peer: their QC is older than what we already know finalised. Don't count.
            if qc.epoch() < leaf_epoch {
                continue;
            }

            // Verify QC against the committee that COULD have signed at qc.epoch().
            let verify_committee = if qc.epoch() == leaf_epoch {
                committee.clone()
            } else {
                let group = qc.shard_group();
                match self.epoch_manager.get_committee_by_shard_group(qc.epoch(), group).await {
                    Ok(c) => c.as_ref().clone(),
                    Err(e) => {
                        debug!(
                            target: LOG_TARGET,
                            "🛜 Probe: no committee for QC epoch {} ({}); skipping {}",
                            qc.epoch(), e, member.address,
                        );
                        continue;
                    },
                }
            };

            if let Err(e) = check_quorum_certificate_signatures::<TConsensusSpec>(
                (&qc).into(),
                &verify_committee,
                &self.signer_service,
            ) {
                debug!(
                    target: LOG_TARGET,
                    "🛜 Probe: {} returned QC with invalid signatures: {}",
                    member.address, e,
                );
                continue;
            }

            // Track the highest verified QC we've seen.
            if qc.epoch() > highest_epoch_seen ||
                (qc.epoch() == highest_epoch_seen && qc.height() > highest_height_seen)
            {
                highest_epoch_seen = qc.epoch();
                highest_height_seen = qc.height();
            }

            // Any verified QC at an epoch beyond our leaf is sufficient evidence that consensus
            // progressed — short-circuit and tell the caller to state-sync.
            if qc.epoch() > leaf_epoch {
                return Ok(ProbeOutcome::HigherQcSeen { epoch: qc.epoch() });
            }

            // QC is at our leaf's epoch — count this peer's attestation (by public key) toward
            // the "no-higher-QC" quorum.
            if counted_pks.insert(member.public_key) {
                attested_power += member.vote_power;
            }

            if attested_power >= quorum_threshold {
                // We have a quorum at the leaf epoch and have not (yet) seen a higher QC.
                // Continue iterating remaining peers anyway so that a higher QC from a peer we
                // haven't asked yet still trumps the quorum result. This is bounded by the
                // committee size.
                continue;
            }
        }

        if highest_epoch_seen > leaf_epoch {
            // Defensive: we should have returned above as soon as the higher QC was verified.
            return Ok(ProbeOutcome::HigherQcSeen {
                epoch: highest_epoch_seen,
            });
        }

        if attested_power >= quorum_threshold {
            Ok(ProbeOutcome::QuorumAtLeaf { attested_power })
        } else {
            Ok(ProbeOutcome::Inconclusive {
                attested_power,
                quorum_threshold,
            })
        }
    }
}

impl<TConsensusSpec> SyncManager for RpcStateSyncClientProtocol<TConsensusSpec>
where TConsensusSpec: ConsensusSpec<Addr = PeerAddress> + Send + Sync + 'static
{
    type Error = RpcStateSyncError;

    async fn check_sync(&self) -> Result<SyncStatus, Self::Error> {
        if self.skip_sync {
            warn!(target: LOG_TARGET, "🛜 State sync is disabled (--skip-sync). Reporting as up to date without checking.");
            return Ok(SyncStatus::UpToDate);
        }

        let oracle_epoch = self.epoch_manager.current_epoch().await?;

        // Load the persisted leaf regardless of which epoch it was written under. The previous
        // implementation looked up by oracle_epoch, which returns NotFound for any stalled-leaf
        // case and conflates "no leaf at this epoch" with "no leaf at all".
        let leaf_block = self.state_store.with_read_tx(|tx| LeafBlock::get_any(tx).optional())?;

        // Cold start: a node that has never entered consensus has no leaf block (a fresh node, or one
        // whose state was wiped). The birthday epoch - the first epoch any validator was active on the
        // network - is a cheap local proxy for "is there a previous epoch's checkpoint to adopt": if
        // the oracle has moved past it there is prior committed state to sync; at or before it there is
        // nothing, so we join consensus directly.
        let Some(leaf) = leaf_block else {
            // A leaf-less node that still holds an `EpochCheckpoint` covering the epoch immediately before the
            // oracle's current epoch — and whose committed state already matches that checkpoint — was rolled back:
            // the offline rollback tool clears the consensus pointers (leaf, locked, high QC) but keeps the committed
            // state and its checkpoint. There is nothing to state-sync; consensus recreates the current epoch's
            // genesis from local state on entering `Running` (see
            // `HotstuffWorker::get_starting_epoch`/`create_genesis_block_if_required`). Reporting `Behind` here would
            // route it to `Syncing`, which fails because every committee member rolled back together holds the same
            // (absent) state, wedging consensus in `Sleeping`.
            //
            // The state-root match is required, not merely the checkpoint's presence: state sync persists the
            // checkpoint before streaming state (`get_or_fetch_valid_epoch_checkpoint`), so a first-time sync
            // interrupted mid-stream also leaves a leaf-less node holding a checkpoint — but with incomplete state.
            // Its roots will not match, so it correctly falls through here and resumes the sync.
            let maybe_checkpoint = self
                .state_store
                .with_read_tx(|tx| EpochCheckpoint::get_last_checkpoint(tx))
                .optional()?;
            if let Some(checkpoint) = maybe_checkpoint &&
                checkpoint.epoch() + Epoch(1) >= oracle_epoch &&
                self.local_state_matches_checkpoint(&checkpoint).await?
            {
                return Ok(SyncStatus::UpToDate);
            }

            let Some(birthday_epoch) = self.epoch_manager.get_birthday_epoch().await? else {
                return Err(RpcStateSyncError::InvariantError {
                    details: "Check sync called before the birthday epoch was determined".to_string(),
                });
            };
            return Ok(if oracle_epoch > birthday_epoch {
                SyncStatus::Behind { target_epoch: None }
            } else {
                SyncStatus::UpToDate
            });
        };

        // Fast path: oracle hasn't advanced past our leaf — nothing to do.
        if oracle_epoch <= leaf.epoch() {
            return Ok(SyncStatus::UpToDate);
        }

        if let Some(shard_group) = self.shard_group_moved_since(oracle_epoch, &leaf).await? {
            info!(target: LOG_TARGET, "🛜 Our shard group changed from {} to {shard_group} at {oracle_epoch}; will state-sync the new group.", leaf.shard_group());
            return Ok(SyncStatus::Behind { target_epoch: None });
        }

        // Fast path: we have a finalised epoch checkpoint at our leaf's epoch. The committee
        // finalised cleanly; this is the normal "behind by one epoch" case that today's
        // state-sync handles. Proceed.
        let has_local_checkpoint = self
            .state_store
            .with_read_tx(|tx| EpochCheckpoint::get_by_shard_group(tx, leaf.epoch(), leaf.shard_group()).optional())?
            .is_some();
        if has_local_checkpoint {
            info!(
                target: LOG_TARGET,
                "🛜 Our leaf {} is behind oracle epoch {}; checkpoint at leaf epoch present, will state-sync.",
                leaf,
                oracle_epoch,
            );
            // No probe was run on this path — fall back to the oracle's current epoch.
            return Ok(SyncStatus::Behind { target_epoch: None });
        }

        // Stall-recovery probe: no checkpoint at our leaf's epoch and oracle has moved on. Ask
        // the leaf-epoch committee for their high QCs and decide based on what they hold.
        info!(
            target: LOG_TARGET,
            "🛜 No checkpoint at leaf epoch {} (oracle at {}); probing committee for highest QCs.",
            leaf.epoch(), oracle_epoch,
        );

        let committee = self
            .epoch_manager
            .get_committee_by_shard_group(leaf.epoch(), leaf.shard_group())
            .await?;
        let our_vn = self.epoch_manager.get_our_validator_node(leaf.epoch()).await?;

        match self
            .probe_high_qcs_at_leaf(committee.as_ref(), &leaf, &our_vn.address)
            .await?
        {
            ProbeOutcome::HigherQcSeen { epoch } => {
                info!(
                    target: LOG_TARGET,
                    "🛜 Peer presented verified high QC at epoch {} > our leaf {}; will state-sync.",
                    epoch, leaf.epoch(),
                );
                // Anchor sync at the probed epoch — the most recent one we've proven was
                // finalised. The oracle may have rolled past it to an epoch with no checkpoint
                // yet; anchoring there would reproduce the `CheckpointNotAvailable` failure that
                // motivated the probe in the first place.
                Ok(SyncStatus::Behind {
                    target_epoch: Some(epoch),
                })
            },
            ProbeOutcome::QuorumAtLeaf { attested_power } => {
                info!(
                    target: LOG_TARGET,
                    "🛜 Committee stalled at our leaf epoch {} (attested power {} ≥ quorum {}). Suppressing state-sync; joining consensus directly.",
                    leaf.epoch(),
                    attested_power,
                    committee.quorum_threshold(),
                );
                Ok(SyncStatus::UpToDate)
            },
            ProbeOutcome::Inconclusive {
                attested_power,
                quorum_threshold,
            } => {
                warn!(
                    target: LOG_TARGET,
                    "🛜 Stall-recovery probe inconclusive: only {} of {} required power attested at leaf epoch {}.",
                    attested_power, quorum_threshold, leaf.epoch(),
                );
                Ok(SyncStatus::Inconclusive)
            },
        }
    }

    async fn sync(&mut self, target_epoch: Option<Epoch>) -> Result<(), Self::Error> {
        if let Err(err) = self.sync_inner(target_epoch).await {
            warn!(target: LOG_TARGET, "🛜State sync failed: {err} (stats: {})", self.stats);
            self.stats = StateSyncStats::default();
            return Err(err);
        }

        info!(target: LOG_TARGET, "🛜State sync completed successfully: {}", self.stats);

        self.stats = StateSyncStats::default();
        Ok(())
    }
}

fn extract_tree_change(
    network: Network,
    update: &SubstateUpdateProof,
    epoch: Epoch,
) -> Result<SubstateTreeChange, RpcStateSyncError> {
    match update {
        SubstateUpdateProof::Create(create) => {
            let id = create.substate.as_versioned_substate_id_ref();
            Ok(SubstateTreeChange::Up {
                id: id.to_owned(),
                value_hash: create.substate.to_value_hash(network, epoch),
            })
        },
        SubstateUpdateProof::Destroy(destroy) => Ok(SubstateTreeChange::Down {
            id: destroy.to_versioned_substate_id(),
        }),
    }
}
