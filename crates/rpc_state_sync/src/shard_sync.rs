//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, ops::Deref, time::Duration};

use anyhow::anyhow;
use futures::{Stream, StreamExt};
use log::*;
use ootle_network::Network;
use prost::Message;
use tari_consensus::hotstuff::commit_proofs::committed_block_commit_proof;
use tari_engine_types::{ProtocolVersion, limits::MAX_CBOR_NESTING_DEPTH};
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    ToSubstateAddress,
    VersionedSubstateId,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_p2p::proto::rpc::{self as proto, SyncStateResponse, sync_state_response};
use tari_ootle_storage::{
    ShardScopedTreeStoreReader,
    ShardScopedTreeStoreWriter,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{
        CommittedBlockProof,
        STATE_VERSION_PROOF_INTERVAL,
        StateVersionProof,
        StateVersionProofSource,
        SubstateRecord,
        SubstateTransition,
        SubstateUpdateBatch,
        SubstateUpdateProof,
        index_substate_down_proofs,
        verify_state_version_leaf,
    },
};
use tari_state_tree::{SPARSE_MERKLE_PLACEHOLDER_HASH, SpreadPrefixStateTree, SubstateTreeChange, TreeHash, Version};
use tari_validator_node_rpc::STATE_SYNC_MAX_BATCH_SIZE;

use crate::{
    error::RpcStateSyncError,
    stats::StateSyncStats,
    version_proofs::{CommitProofValidator, ProofSchedule},
};

const LOG_TARGET: &str = "tari::ootle::rpc_state_sync::shard_sync";
/// The most a peer may stream, in encoded bytes, for one state version before completing it. A
/// version is buffered whole and committed at once, so this bounds the memory a peer can hold to a
/// constant multiple of it: the buffered updates are held decoded, each with its tree change.
const MAX_BUFFERED_VERSION_BYTES: usize = 256 * 1024 * 1024;
// A state version is one block's writes to the shard, which consensus bounds by
// `MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES`. The stream carries each update with its metadata, so the
// buffer leaves twice that.
const _: () =
    assert!(MAX_BUFFERED_VERSION_BYTES >= 2 * tari_ootle_common_types::MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES);
/// The most state versions a peer may commit in one attempt without a verified state version proof in between.
///
/// It applies when the stream is held to an enforced proof schedule, where the schedule already rejects a stream
/// that goes a full proof interval without a proof, and when no proof validator is configured. A stream of a
/// network launched before V1 is not held to the schedule until its first proof, because that history cannot be
/// proven. There a batch is bounded by the checkpoint's state version when that version is trusted (see
/// [`ShardSync::with_trusted_checkpoint_version`]); a stream synced against a V0 checkpoint, which commits no state
/// versions, has no upper bound on its versions, and only the checkpoint root check at completion rejects it.
const MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT: u64 = 64;
// An honest stream held to the schedule commits up to a full proof interval before it must prove one.
const _: () = assert!(MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT > STATE_VERSION_PROOF_INTERVAL);

/// Rewinds every shard that a sync committed unverified versions of, so that everything the node reads or builds
/// on afterwards is verified state.
pub(crate) fn discard_all_unverified_state<TStore: StateStore>(store: &TStore) -> Result<(), RpcStateSyncError> {
    store.with_write_tx(|tx| {
        for (shard, rewind_point) in tx.state_sync_rewind_points_get_all()? {
            rewind_shard(tx, shard, rewind_point)?;
        }
        Ok::<_, RpcStateSyncError>(())
    })
}

fn rewind_shard<TTx: StateStoreWriteTransaction>(
    tx: &mut TTx,
    shard: Shard,
    rewind_point: Version,
) -> Result<(), StorageError> {
    let tree_stats = tx.state_tree_truncate_to_version(shard, rewind_point)?;
    let substate_stats = tx.substates_rewind_to_state_version(shard, rewind_point)?;
    tx.state_sync_rewind_point_remove(shard)?;
    warn!(
        target: LOG_TARGET,
        "🛜 Discarded unverified synced state for {shard} above v{rewind_point}: {} state version(s), {} tree node(s)",
        substate_stats.transitions_processed,
        tree_stats.nodes_deleted,
    );
    Ok(())
}

/// A validator for a stream that carries no state version proofs, which rejects any it is given.
pub(crate) struct NoVersionProofs;

impl CommitProofValidator for NoVersionProofs {
    async fn validate(&self, _commit_proof: &CommittedBlockProof) -> Result<(), RpcStateSyncError> {
        Err(RpcStateSyncError::InvalidResponse(anyhow!(
            "Peer sent a state version proof that was not requested"
        )))
    }
}

/// Syncs one shard's state from a peer's stream against the shard root of a trusted checkpoint.
pub(crate) struct ShardSync<'a, TStore, TValidator = NoVersionProofs> {
    network: Network,
    num_preshards: NumPreshards,
    store: &'a TStore,
    shard: Shard,
    checkpoint_shard_root: TreeHash,
    checkpoint_state_version: Version,
    /// Whether `checkpoint_state_version` is trusted as this shard's version. Only then is a batch above it rejected:
    /// an untrusted version may be lower than the versions an honest peer streams.
    checkpoint_state_version_is_trusted: bool,
    /// The longest the stream may go without committing a version or proving one. A zero timeout sets no limit.
    progress_timeout: Option<Duration>,
    /// Validates the commit proofs of the state version proofs the stream carries, if it was asked to carry them.
    version_proofs: Option<&'a TValidator>,
}

impl<'a, TStore: StateStore> ShardSync<'a, TStore> {
    pub fn new(
        network: Network,
        num_preshards: NumPreshards,
        store: &'a TStore,
        shard: Shard,
        checkpoint_shard_root: TreeHash,
        checkpoint_state_version: Version,
    ) -> Self {
        Self {
            network,
            num_preshards,
            store,
            shard,
            checkpoint_shard_root,
            checkpoint_state_version,
            checkpoint_state_version_is_trusted: false,
            progress_timeout: None,
            version_proofs: None,
        }
    }
}

impl<'a, TStore: StateStore, TValidator: CommitProofValidator> ShardSync<'a, TStore, TValidator> {
    /// Holds the stream to the proof schedule, validating each proof's commit proof with `validator`.
    pub fn with_version_proofs<V: CommitProofValidator>(self, validator: &'a V) -> ShardSync<'a, TStore, V> {
        ShardSync {
            network: self.network,
            num_preshards: self.num_preshards,
            store: self.store,
            shard: self.shard,
            checkpoint_shard_root: self.checkpoint_shard_root,
            checkpoint_state_version: self.checkpoint_state_version,
            checkpoint_state_version_is_trusted: self.checkpoint_state_version_is_trusted,
            progress_timeout: self.progress_timeout,
            version_proofs: Some(validator),
        }
    }

    /// Rejects a stream that goes `timeout` without committing a version or proving one. A zero timeout sets no
    /// limit.
    pub fn with_progress_timeout(mut self, timeout: Duration) -> Self {
        self.progress_timeout = Some(timeout);
        self
    }

    /// Rejects any batch above the checkpoint's state version if `is_trusted`. The checkpoint's quorum signatures
    /// must cover that version, and it must be the version of this shard rather than another shard of the group.
    pub fn with_trusted_checkpoint_version(mut self, is_trusted: bool) -> Self {
        self.checkpoint_state_version_is_trusted = is_trusted;
        self
    }

    /// The proof schedule for a stream starting at `start_state_version`, if the stream carries proofs. A network
    /// launched at V1 or later has every version since genesis proven, so it is held to the schedule from the
    /// start.
    fn proof_schedule(&self, start_state_version: Version) -> Option<ProofSchedule> {
        self.version_proofs.map(|_| {
            ProofSchedule::new(
                ProtocolVersion::genesis(self.network) >= ProtocolVersion::V1,
                start_state_version,
                self.checkpoint_state_version,
            )
        })
    }

    /// Verifies that `proof` proves the shard at `proof.state_version` with the root of `written_version`, the last
    /// version the stream wrote, and keeps it to re-serve to peers that sync from this node.
    async fn apply_version_proof(
        &self,
        validator: &TValidator,
        proof: proto::StateVersionProof,
        written_version: Option<Version>,
    ) -> Result<Version, RpcStateSyncError> {
        let shard = self.shard;
        if proof.shard != shard.as_u32() {
            return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                "Received a state version proof for shard {} while syncing {shard}",
                proof.shard
            )));
        }
        let state_version = proof.state_version;
        if state_version > self.checkpoint_state_version {
            return Err(RpcStateSyncError::ConflictsWithCheckpoint(anyhow!(
                "Received a proof of v{state_version} for {shard}, above the checkpoint's v{}",
                self.checkpoint_state_version
            )));
        }
        if state_version < written_version.unwrap_or(0) {
            return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                "Received a proof of v{state_version} for {shard}, outside v{}..=v{}",
                written_version.unwrap_or(0),
                self.checkpoint_state_version
            )));
        }
        let commit_proof = CommittedBlockProof::from_bytes(&proof.commit_proof)
            .map_err(|e| RpcStateSyncError::InvalidResponse(anyhow!("Undecodable commit proof: {e}")))?;
        let shard_root_proof =
            tari_bor::serde_codec::from_slice_with_max_depth(&proof.shard_root_proof, MAX_CBOR_NESTING_DEPTH)
                .map_err(|e| RpcStateSyncError::InvalidResponse(anyhow!("Undecodable shard root proof: {e}")))?;

        let shard_root = self.local_state_root(written_version)?;
        verify_state_version_leaf(
            self.network,
            &commit_proof,
            shard,
            state_version,
            &shard_root,
            &shard_root_proof,
        )
        .map_err(|e| RpcStateSyncError::InvalidResponse(e.into()))?;
        validator.validate(&commit_proof).await?;

        self.store.with_write_tx(|tx| {
            tx.state_version_proofs_insert(&StateVersionProof {
                shard,
                state_version,
                source: StateVersionProofSource::Received {
                    commit_proof: proof.commit_proof,
                },
                shard_root_proof,
            })?;
            // The state through the written version is now proven, so a later failure rewinds no further back.
            tx.state_sync_rewind_point_remove(shard)
        })?;
        debug!(target: LOG_TARGET, "🛜 ✅ {shard} proven at v{state_version}");
        Ok(state_version)
    }

    /// Advances the shard from `verified_version`, whose root matches the checkpoint, to the checkpoint's state
    /// version, and returns the version the shard is then at.
    ///
    /// A shard's trailing versions can change its tree without changing any substate, so the last version a stream
    /// writes can lie below the checkpoint's version while already holding the checkpoint's root. Every node that
    /// ran consensus is at the checkpoint's version, and from [`ProtocolVersion::V1`] the state merkle root commits
    /// to each shard's version, so a node left below it computes a different root for every block.
    ///
    /// [`ProtocolVersion::V1`]: tari_engine_types::ProtocolVersion::V1
    pub fn align_to_checkpoint_version(
        &self,
        verified_version: Option<Version>,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let target = self.checkpoint_state_version;
        if verified_version.unwrap_or(0) >= target {
            return Ok(verified_version);
        }
        self.store.with_write_tx(|tx| {
            let mut store = ShardScopedTreeStoreWriter::new(tx, self.shard);
            SpreadPrefixStateTree::new(&mut store).batch_put_substate_changes(verified_version, target, [])?;
            store.set_state_version(target)?;
            Ok::<_, RpcStateSyncError>(())
        })?;
        info!(
            target: LOG_TARGET,
            "🛜 Advanced {} from v{} to the checkpoint's v{target}",
            self.shard,
            verified_version.unwrap_or(0),
        );
        Ok(Some(target))
    }

    /// The first version to stream on top of `persisted_version`.
    ///
    /// The stream is inclusive of it, so it must be the first version not yet persisted. A persisted version must
    /// never be written a second time: JMT nodes are keyed by (version, nibble_path), so rewriting a version
    /// overwrites live nodes and records those very keys as stale at that version, and the stale-node GC then
    /// deletes them from under the current tree. Bootstrapped genesis state is committed at version 0 and is never
    /// synced - every node bootstraps it - so a freshly bootstrapped node starts at version 1, which is also the
    /// minimum the peer accepts.
    pub fn start_state_version(&self, persisted_version: Option<Version>) -> Result<Version, RpcStateSyncError> {
        match persisted_version {
            Some(version) => version.checked_add(1).ok_or_else(|| RpcStateSyncError::InvariantError {
                details: format!(
                    "{} is persisted at v{version}, the last representable state version, yet differs from the \
                     checkpoint",
                    self.shard
                ),
            }),
            None => Ok(1),
        }
    }

    pub fn local_state_root(&self, version: Option<Version>) -> Result<TreeHash, RpcStateSyncError> {
        self.store
            .with_read_tx(|tx| calculate_state_root_for_shard(tx, self.shard, version))
    }

    /// Rewinds the versions of the shard that an earlier sync committed but never verified against its checkpoint,
    /// and returns the shard's latest verified version.
    pub fn discard_unverified_state(&self) -> Result<Option<Version>, RpcStateSyncError> {
        let shard = self.shard;
        self.store.with_write_tx(|tx| {
            if let Some(rewind_point) = tx.state_sync_rewind_point_get(shard)? {
                rewind_shard(tx, shard, rewind_point)?;
            }
            Ok::<_, RpcStateSyncError>(tx.state_tree_versions_get_latest(shard)?)
        })
    }

    /// Syncs the shard from `state_stream` on top of `verified_version`, its latest verified version. Versions are
    /// committed as they arrive and discarded again unless the stream completes with the shard matching the
    /// checkpoint root, so only verified state outlives a sync.
    pub async fn sync_from_stream<S, E>(
        &self,
        stats: &mut StateSyncStats,
        verified_version: Option<Version>,
        state_stream: S,
    ) -> Result<Option<Version>, RpcStateSyncError>
    where
        S: Stream<Item = Result<SyncStateResponse, E>> + Unpin,
        RpcStateSyncError: From<E>,
    {
        let result = self.apply_stream(stats, verified_version, state_stream).await;
        let Err(err) = result else {
            return result;
        };
        let has_unverified_state = matches!(
            self.store.with_read_tx(|tx| tx.state_sync_rewind_point_get(self.shard)),
            Ok(Some(_))
        );
        if let Err(discard_err) = self.discard_unverified_state() {
            // The rewind point is still stored, so the next sync attempt discards the state first.
            error!(
                target: LOG_TARGET,
                "❌ Failed to discard unverified synced state for {}: {discard_err}",
                self.shard,
            );
        }
        // A peer whose stream fails once it has made this node commit unverified state has cost the work of
        // committing and discarding it, whatever the cause.
        if has_unverified_state && matches!(err, RpcStateSyncError::RpcError(_)) {
            return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                "Stream failed after committing unverified state: {err}"
            )));
        }
        Err(err)
    }

    #[expect(clippy::too_many_lines)]
    async fn apply_stream<S, E>(
        &self,
        stats: &mut StateSyncStats,
        mut maybe_persisted_state_version: Option<Version>,
        mut state_stream: S,
    ) -> Result<Option<Version>, RpcStateSyncError>
    where
        S: Stream<Item = Result<SyncStateResponse, E>> + Unpin,
        RpcStateSyncError: From<E>,
    {
        let shard = self.shard;
        let mut rewind_point = maybe_persisted_state_version.unwrap_or(0);
        let mut has_unverified_state = false;
        let start_state_version = self.start_state_version(maybe_persisted_state_version)?;
        let mut last_state_version = start_state_version;
        let mut tree_changes = vec![];
        let mut updates = vec![];
        let mut expected_state_version = None;
        let mut buffered = VersionBuffer::default();
        let mut proof_schedule = self.proof_schedule(start_state_version);
        // The last version this stream committed or proved. A new version must lie above it: re-committing a version
        // fails only inside the write transaction, after the whole version is buffered.
        let mut last_committed_version = None;
        let mut last_proven_version = None;
        let mut unverified_versions = 0u64;
        let mut progress_deadline = next_progress_deadline(self.progress_timeout);

        // syncing states
        loop {
            let next = match progress_deadline {
                Some(deadline) => tokio::time::timeout_at(deadline, state_stream.next())
                    .await
                    .map_err(|_| {
                        RpcStateSyncError::InvalidResponse(anyhow!(
                            "Peer made no progress syncing {shard} within {:?}",
                            self.progress_timeout.unwrap_or_default()
                        ))
                    })?,
                None => state_stream.next().await,
            };
            let Some(result) = next else {
                break;
            };
            let msg = result?;
            let batch = match msg.response {
                Some(sync_state_response::Response::Batch(batch)) => batch,
                Some(sync_state_response::Response::VersionProof(proof)) => {
                    let (Some(validator), Some(schedule)) = (self.version_proofs, proof_schedule.as_mut()) else {
                        return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                            "Peer sent a state version proof that was not requested"
                        )));
                    };
                    if let Some(version) = expected_state_version {
                        return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                            "Peer sent a state version proof in the middle of v{version}"
                        )));
                    }
                    if let Some(proven) = last_proven_version &&
                        proof.state_version <= proven
                    {
                        return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                            "Received a proof of v{} after this stream already proved v{proven}",
                            proof.state_version
                        )));
                    }
                    let proven = self
                        .apply_version_proof(validator, proof, maybe_persisted_state_version)
                        .await?;
                    schedule.proven(proven);
                    last_proven_version = Some(proven);
                    last_committed_version = Some(proven);
                    progress_deadline = next_progress_deadline(self.progress_timeout);
                    unverified_versions = 0;
                    has_unverified_state = false;
                    rewind_point = maybe_persisted_state_version.unwrap_or(0);
                    // Every version up to the proven one is settled, so the stream must continue above it.
                    last_state_version = last_state_version.max(proven.saturating_add(1));
                    continue;
                },
                Some(sync_state_response::Response::Complete(complete)) => {
                    if let Some(schedule) = &proof_schedule {
                        schedule.check_complete()?;
                    }
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
                    let local_state_root = self.local_state_root(maybe_persisted_state_version)?;
                    if local_state_root != self.checkpoint_shard_root {
                        error!(
                            target: LOG_TARGET,
                            "❌ State root mismatch for {shard}. Checkpoint {expected} but got {actual}.",
                            expected = self.checkpoint_shard_root,
                            actual = local_state_root,
                        );
                        return Err(RpcStateSyncError::StateRootMismatch {
                            expected: self.checkpoint_shard_root,
                            actual: local_state_root,
                        });
                    }
                    if has_unverified_state {
                        self.store
                            .with_write_tx(|tx| tx.state_sync_rewind_point_remove(shard))?;
                    }
                    info!(
                        target: LOG_TARGET,
                        "🛜 ✅ State root for {shard} matches checkpoint: {local_state_root} (v{})",
                        maybe_persisted_state_version.unwrap_or(0),
                    );
                    return self.align_to_checkpoint_version(maybe_persisted_state_version);
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
            if self.checkpoint_state_version_is_trusted && batch.state_version > self.checkpoint_state_version {
                return Err(RpcStateSyncError::ConflictsWithCheckpoint(anyhow!(
                    "Received state version {} that is above the checkpoint state version {}.",
                    batch.state_version,
                    self.checkpoint_state_version
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

            if expected_state_version.is_none() {
                if let Some(committed) = last_committed_version &&
                    state_version <= committed
                {
                    return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                        "Received a new state version v{state_version} that is not above the last committed state \
                         version v{committed}."
                    )));
                }
                let budget_applies = proof_schedule.as_ref().is_none_or(|s| s.is_enforced());
                if budget_applies && unverified_versions >= MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT {
                    return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                        "Peer exceeded unverified version budget: {MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT} state \
                         versions without a state version proof"
                    )));
                }
            }

            if let Some(schedule) = &proof_schedule {
                schedule.check_batch(state_version)?;
            }
            last_state_version = state_version;
            buffered.charge(state_version, batch.encoded_len())?;

            stats.total_transitions += batch.updates.len() as u64;

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
                let id = update.to_versioned_substate_id();
                let update_shard = id.to_shard(self.num_preshards);
                if update_shard != shard {
                    return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                        "Peer streamed an update to {id} in {update_shard} while syncing {shard}"
                    )));
                }
                let tree_change = extract_tree_change(self.network, &update, msg_epoch);

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
                expected_state_version = Some(state_version);
                continue;
            }

            expected_state_version = None;
            buffered = VersionBuffer::default();

            // Commit the buffered changes for this state version. The shard root is verified once, on
            // the terminal SyncComplete, against the trusted checkpoint. Until then the rewind point
            // marks every version committed above it as unverified, including across a restart.
            self.store.with_write_tx(|tx| {
                info!(
                    target: LOG_TARGET,
                    "🛜 Next state updates batch of size {} from v{}",
                    updates.len(),
                    state_version
                );

                check_updates_apply(&**tx, state_version, &updates)?;
                if !has_unverified_state {
                    tx.state_sync_rewind_point_set(shard, rewind_point)?;
                }

                let mut store = ShardScopedTreeStoreWriter::new(tx, shard);

                info!(target: LOG_TARGET, "🛜 {} state update(s) for v{}", updates.len(), state_version);
                commit_updates(
                    self.network,
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
            has_unverified_state = true;
            last_committed_version = Some(state_version);
            unverified_versions += 1;
            progress_deadline = next_progress_deadline(self.progress_timeout);
        }

        // The stream ended without a SyncComplete - the peer closed early, so the sync is unverified.
        Err(RpcStateSyncError::InvalidResponse(anyhow!(
            "State sync stream for {shard} ended without a completion marker"
        )))
    }
}

/// The instant by which a stream must next make progress, if `timeout` sets a limit. A zero timeout, or one too long
/// to represent, sets none.
fn next_progress_deadline(timeout: Option<Duration>) -> Option<tokio::time::Instant> {
    timeout
        .filter(|timeout| !timeout.is_zero())
        .and_then(|timeout| tokio::time::Instant::now().checked_add(timeout))
}

/// Rejects a state version with a transition that does not apply cleanly to local state: creating a substate that
/// already exists, or destroying one that is not up. A rewind inverts each committed transition, which restores the
/// prior state only if every transition applied cleanly.
fn check_updates_apply<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    state_version: Version,
    updates: &[SubstateUpdateProof],
) -> Result<(), RpcStateSyncError> {
    let mut created = HashSet::new();
    let mut destroyed = HashSet::new();
    for update in updates {
        let id = update.to_versioned_substate_id();
        let address = id.to_substate_address();
        match update {
            SubstateUpdateProof::Create(_) => {
                if !created.insert(address) || tx.substates_get(&address).optional()?.is_some() {
                    return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                        "Peer streamed the creation of {id} at v{state_version}, but {id} already exists"
                    )));
                }
            },
            SubstateUpdateProof::Destroy(_) => {
                let is_up =
                    created.contains(&address) || tx.substates_get(&address).optional()?.is_some_and(|s| s.is_up());
                if !is_up || !destroyed.insert(address) {
                    return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                        "Peer streamed the destruction of {id} at v{state_version}, but {id} is not up"
                    )));
                }
            },
        }
    }
    Ok(())
}

pub(crate) fn calculate_state_root_for_shard<TTx: StateStoreReadTransaction>(
    tx: &TTx,
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

/// Commits one state version's updates and records a down proof for each substate it destroys, where this node holds
/// a proof of a version the substate was up at.
fn commit_updates<TTx, I>(
    network: Network,
    tx: &mut TTx,
    shard: Shard,
    epoch: Epoch,
    state_version: Version,
    updates: I,
) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
    I: IntoIterator<Item = SubstateUpdateProof>,
{
    let mut batch = SubstateUpdateBatch::new(network, epoch);

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

    index_substate_down_proofs(tx, batch.downed_iter(), committed_block_commit_proof)?;
    SubstateRecord::commit_batch(tx, batch)?;

    Ok(())
}

fn extract_tree_change(network: Network, update: &SubstateUpdateProof, epoch: Epoch) -> SubstateTreeChange {
    match update {
        SubstateUpdateProof::Create(create) => {
            let id = create.substate.as_versioned_substate_id_ref();
            SubstateTreeChange::Up {
                id: id.to_owned(),
                value_hash: create.substate.to_value_hash(network, epoch),
            }
        },
        SubstateUpdateProof::Destroy(destroy) => SubstateTreeChange::Down {
            id: destroy.to_versioned_substate_id(),
        },
    }
}

/// The encoded bytes buffered for the state version being streamed, held to
/// [`MAX_BUFFERED_VERSION_BYTES`].
#[derive(Debug, Default)]
struct VersionBuffer {
    bytes: usize,
}

impl VersionBuffer {
    fn charge(&mut self, state_version: Version, bytes: usize) -> Result<(), RpcStateSyncError> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > MAX_BUFFERED_VERSION_BYTES {
            return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                "Peer streamed more than {MAX_BUFFERED_VERSION_BYTES} bytes for v{state_version} without completing it"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use futures::{FutureExt, stream};
    use tari_engine_types::substate::SubstateId;
    use tari_ootle_common_types::{SubstateAddress, SubstateVersion, optional::Optional};
    use tari_ootle_p2p::proto::rpc::{SubstateBatch, SyncComplete};
    use tari_ootle_storage::consensus_models::{SubstateCreate, SubstateData, SubstateDestroy, SubstateValueOrHash};
    use tari_rpc_framework::RpcStatus;
    use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
    use tari_state_tree::memory_store::MemoryTreeStore;
    use tari_template_lib_types::{ComponentAddress, Hash32, ObjectKey};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_version_is_buffered_up_to_the_byte_budget() {
        let mut buffer = VersionBuffer::default();
        let chunk = 6 * 1024 * 1024;
        for _ in 0..MAX_BUFFERED_VERSION_BYTES / chunk {
            buffer.charge(1, chunk).unwrap();
        }
        buffer.charge(1, MAX_BUFFERED_VERSION_BYTES % chunk).unwrap();
        assert!(matches!(
            buffer.charge(1, 1),
            Err(RpcStateSyncError::InvalidResponse(_))
        ));
    }

    const NETWORK: Network = Network::LocalNet;
    const NUM_PRESHARDS: NumPreshards = NumPreshards::P256;
    const EPOCH: Epoch = Epoch(1);
    const HONEST: u8 = 1;
    const POISON: u8 = 0xBA;

    type Versions = Vec<(Version, Vec<SubstateUpdateProof>)>;

    fn shard() -> Shard {
        Shard::from(3u32)
    }

    fn other_shard() -> Shard {
        Shard::from(4u32)
    }

    fn create_store() -> (RocksDbStateStore<String>, TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::open(tmp.path().join("rocksdb"), DatabaseOptions::default()).unwrap();
        (store, tmp)
    }

    /// A substate that lands in `shard`: with 256 preshards the first address byte selects shard `byte + 1`.
    fn substate_id_in(shard: Shard, seed: u8) -> SubstateId {
        let mut bytes = [seed; ObjectKey::LENGTH];
        bytes[0] = u8::try_from(shard.as_u32() - 1).unwrap();
        SubstateId::Component(ComponentAddress::from_array(bytes))
    }

    fn address_in(shard: Shard, seed: u8) -> SubstateAddress {
        SubstateAddress::from_substate_id(&substate_id_in(shard, seed), SubstateVersion::ZERO)
    }

    fn address(seed: u8) -> SubstateAddress {
        address_in(shard(), seed)
    }

    fn create(seed: u8) -> SubstateUpdateProof {
        create_with_value(seed, seed)
    }

    fn create_with_value(seed: u8, value: u8) -> SubstateUpdateProof {
        create_in(shard(), seed, value)
    }

    fn create_in(shard: Shard, seed: u8, value: u8) -> SubstateUpdateProof {
        SubstateUpdateProof::Create(Box::new(SubstateCreate {
            substate: SubstateData {
                substate_id: substate_id_in(shard, seed),
                version: SubstateVersion::ZERO,
                value: SubstateValueOrHash::Hash(Hash32::from_array([value; 32])),
                template_metadata: None,
            },
        }))
    }

    fn destroy(seed: u8) -> SubstateUpdateProof {
        destroy_in(shard(), seed)
    }

    fn destroy_in(shard: Shard, seed: u8) -> SubstateUpdateProof {
        SubstateUpdateProof::Destroy(SubstateDestroy {
            substate_id: substate_id_in(shard, seed),
            version: SubstateVersion::ZERO,
        })
    }

    fn batch(state_version: Version, updates: Vec<SubstateUpdateProof>) -> Result<SyncStateResponse, RpcStatus> {
        batch_in(shard(), state_version, updates)
    }

    fn batch_in(
        shard: Shard,
        state_version: Version,
        updates: Vec<SubstateUpdateProof>,
    ) -> Result<SyncStateResponse, RpcStatus> {
        Ok(SyncStateResponse {
            response: Some(sync_state_response::Response::Batch(SubstateBatch {
                state_version,
                updates: updates.into_iter().map(Into::into).collect(),
                has_more: false,
                epoch: Some(EPOCH.into()),
                shard: shard.as_u32(),
            })),
        })
    }

    fn batch_with_more(
        state_version: Version,
        updates: Vec<SubstateUpdateProof>,
    ) -> Result<SyncStateResponse, RpcStatus> {
        let mut response = batch(state_version, updates)?;
        if let Some(sync_state_response::Response::Batch(batch)) = response.response.as_mut() {
            batch.has_more = true;
        }
        Ok(response)
    }

    fn stream_of(versions: &Versions) -> Vec<Result<SyncStateResponse, RpcStatus>> {
        versions
            .iter()
            .map(|(version, updates)| batch(*version, updates.clone()))
            .collect()
    }

    fn complete(synced_to_version: Version) -> Result<SyncStateResponse, RpcStatus> {
        complete_in(shard(), synced_to_version)
    }

    fn complete_in(shard: Shard, synced_to_version: Version) -> Result<SyncStateResponse, RpcStatus> {
        Ok(SyncStateResponse {
            response: Some(sync_state_response::Response::Complete(SyncComplete {
                synced_to_version,
                epoch: Some(EPOCH.into()),
                shard: shard.as_u32(),
                is_final: true,
            })),
        })
    }

    /// The shard root a checkpoint commits to once `versions` are applied in order.
    fn root_after(versions: &Versions) -> TreeHash {
        let mut store = MemoryTreeStore::new();
        let mut tree = SpreadPrefixStateTree::new(&mut store);
        let mut prev = None;
        let mut root = SPARSE_MERKLE_PLACEHOLDER_HASH;
        for (version, updates) in versions {
            root = tree
                .put_substate_changes(
                    prev,
                    *version,
                    updates.iter().map(|u| extract_tree_change(NETWORK, u, EPOCH)),
                )
                .unwrap();
            prev = Some(*version);
        }
        root
    }

    /// The shard state version a checkpoint records once `versions` are applied in order.
    fn version_after(versions: &Versions) -> Version {
        versions.last().map_or(0, |(version, _)| *version)
    }

    fn local_version<TStore: StateStore>(store: &TStore) -> Option<Version> {
        store
            .with_read_tx(|tx| tx.state_tree_versions_get_latest(shard()))
            .unwrap()
    }

    fn rewind_point<TStore: StateStore>(store: &TStore) -> Option<Version> {
        store
            .with_read_tx(|tx| tx.state_sync_rewind_point_get(shard()))
            .unwrap()
    }

    fn substate<TStore: StateStore>(store: &TStore, seed: u8) -> Option<SubstateRecord> {
        store
            .with_read_tx(|tx| tx.substates_get(&address(seed)).optional())
            .unwrap()
    }

    async fn sync<TStore: StateStore>(
        store: &TStore,
        checkpoint: &Versions,
        responses: Vec<Result<SyncStateResponse, RpcStatus>>,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let sync = ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            store,
            shard(),
            root_after(checkpoint),
            version_after(checkpoint),
        );
        let verified_version = sync.discard_unverified_state()?;
        sync.sync_from_stream(
            &mut StateSyncStats::default(),
            verified_version,
            stream::iter(responses),
        )
        .await
    }

    /// Syncs `versions` from an honest peer, leaving them as the store's verified state.
    async fn sync_honestly<TStore: StateStore>(store: &TStore, versions: &Versions) {
        let mut responses = stream_of(versions);
        responses.push(complete(versions.last().unwrap().0));
        sync(store, versions, responses).await.unwrap();
    }

    #[tokio::test]
    async fn a_stream_that_matches_the_checkpoint_is_kept() {
        let (store, _tmp) = create_store();
        let honest = vec![(1, vec![create(HONEST)]), (2, vec![create(2)])];

        sync_honestly(&store, &honest).await;

        assert_eq!(local_version(&store), Some(2));
        assert!(substate(&store, HONEST).is_some_and(|s| s.is_up()));
        assert!(substate(&store, 2).is_some_and(|s| s.is_up()));
        assert_eq!(rewind_point(&store), None);
    }

    #[tokio::test]
    async fn a_shard_whose_trailing_versions_change_no_substate_ends_at_the_checkpoint_version() {
        let (store, _tmp) = create_store();
        // v3 changes the tree without changing a substate, so the peer has nothing to stream for it.
        let checkpoint = vec![(1, vec![create(HONEST)]), (2, vec![create(2)]), (3, vec![])];

        let synced = sync(&store, &checkpoint, vec![
            batch(1, vec![create(HONEST)]),
            batch(2, vec![create(2)]),
            complete(3),
        ])
        .await
        .unwrap();

        assert_eq!(synced, Some(3));
        assert_eq!(local_version(&store), Some(3));
        let sync = ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            &store,
            shard(),
            root_after(&checkpoint),
            version_after(&checkpoint),
        );
        assert_eq!(sync.local_state_root(Some(3)).unwrap(), root_after(&checkpoint));
        assert_eq!(rewind_point(&store), None);
    }

    #[tokio::test]
    async fn a_stream_that_closes_early_leaves_no_state_behind() {
        let (store, _tmp) = create_store();
        let honest = vec![(1, vec![create(HONEST)])];

        let err = sync(&store, &honest, vec![batch(1, vec![create(HONEST), create(POISON)])])
            .await
            .unwrap_err();

        assert!(matches!(err, RpcStateSyncError::InvalidResponse(_)), "{err}");
        assert_eq!(local_version(&store), None);
        assert!(substate(&store, POISON).is_none());
        assert!(substate(&store, HONEST).is_none());
        assert_eq!(rewind_point(&store), None);
    }

    #[tokio::test]
    async fn a_stream_that_misses_the_checkpoint_root_leaves_no_state_behind() {
        let (store, _tmp) = create_store();
        let honest = vec![(1, vec![create(HONEST)])];

        let err = sync(&store, &honest, vec![
            batch(1, vec![create(HONEST), create(POISON)]),
            complete(1),
        ])
        .await
        .unwrap_err();

        assert!(matches!(err, RpcStateSyncError::StateRootMismatch { .. }), "{err}");
        assert_eq!(local_version(&store), None);
        assert!(substate(&store, POISON).is_none());
        assert_eq!(rewind_point(&store), None);
    }

    #[tokio::test]
    async fn a_failed_sync_restores_the_verified_state_it_started_from() {
        let (store, _tmp) = create_store();
        let verified = vec![(1, vec![create(HONEST)])];
        sync_honestly(&store, &verified).await;

        let mut honest = verified.clone();
        honest.push((2, vec![create(2)]));
        sync(&store, &honest, vec![batch(2, vec![destroy(HONEST), create(POISON)])])
            .await
            .unwrap_err();

        assert_eq!(local_version(&store), Some(1));
        assert!(substate(&store, HONEST).is_some_and(|s| s.is_up()));
        assert!(substate(&store, POISON).is_none());
        let shard_sync = ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            &store,
            shard(),
            root_after(&verified),
            version_after(&verified),
        );
        assert_eq!(shard_sync.local_state_root(Some(1)).unwrap(), root_after(&verified));

        sync(&store, &honest, vec![batch(2, vec![create(2)]), complete(2)])
            .await
            .unwrap();
        assert_eq!(local_version(&store), Some(2));
    }

    #[tokio::test]
    async fn a_version_that_overwrites_local_state_is_rejected() {
        let (store, _tmp) = create_store();
        let verified = vec![(1, vec![create(HONEST)])];
        sync_honestly(&store, &verified).await;
        let mut checkpoint = verified.clone();
        checkpoint.push((2, vec![create(2)]));

        let err = sync(&store, &checkpoint, vec![batch(2, vec![create_with_value(
            HONEST, POISON,
        )])])
        .await
        .unwrap_err();
        assert!(matches!(err, RpcStateSyncError::InvalidResponse(_)), "{err}");

        let err = sync(&store, &checkpoint, vec![batch(2, vec![
            destroy(HONEST),
            destroy(HONEST),
        ])])
        .await
        .unwrap_err();
        assert!(matches!(err, RpcStateSyncError::InvalidResponse(_)), "{err}");

        let record = substate(&store, HONEST).unwrap();
        assert!(record.is_up());
        assert_eq!(*record.state_hash(), Hash32::from_array([HONEST; 32]));
        assert_eq!(local_version(&store), Some(1));
    }

    #[test]
    fn the_last_representable_version_has_no_successor() {
        let (store, _tmp) = create_store();
        let sync = ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            &store,
            shard(),
            SPARSE_MERKLE_PLACEHOLDER_HASH,
            0,
        );
        assert_eq!(sync.start_state_version(None).unwrap(), 1);
        assert_eq!(sync.start_state_version(Some(41)).unwrap(), 42);
        assert!(matches!(
            sync.start_state_version(Some(Version::MAX)),
            Err(RpcStateSyncError::InvariantError { .. })
        ));
    }

    #[tokio::test]
    async fn an_interrupted_sync_is_discarded_before_the_next_attempt() {
        let (store, _tmp) = create_store();
        let honest = vec![(1, vec![create(HONEST)])];
        let sync = ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            &store,
            shard(),
            root_after(&honest),
            version_after(&honest),
        );

        // The peer stalls after one version and the sync is dropped mid-stream, as on shutdown.
        let stalled = stream::iter(vec![batch(1, vec![create(POISON)])]).chain(stream::pending());
        let interrupted = sync
            .sync_from_stream(&mut StateSyncStats::default(), None, stalled)
            .now_or_never();
        assert!(interrupted.is_none());
        assert!(substate(&store, POISON).is_some());
        assert_eq!(rewind_point(&store), Some(0));

        assert_eq!(sync.discard_unverified_state().unwrap(), None);
        assert_eq!(local_version(&store), None);
        assert!(substate(&store, POISON).is_none());
        assert_eq!(rewind_point(&store), None);
    }

    /// Starts a sync of `shard` that streams `updates` at v1 and is then dropped mid-stream.
    fn interrupt_sync<TStore: StateStore>(
        store: &TStore,
        shard: Shard,
        updates: Vec<SubstateUpdateProof>,
    ) -> Option<Result<Option<Version>, RpcStateSyncError>> {
        let sync = ShardSync::new(NETWORK, NUM_PRESHARDS, store, shard, SPARSE_MERKLE_PLACEHOLDER_HASH, 1);
        let stalled = stream::iter(vec![batch_in(shard, 1, updates)]).chain(stream::pending());
        sync.sync_from_stream(&mut StateSyncStats::default(), None, stalled)
            .now_or_never()
    }

    #[tokio::test]
    async fn an_update_outside_the_synced_shard_is_rejected() {
        let (store, _tmp) = create_store();
        let other_verified = vec![(1, vec![create_in(other_shard(), HONEST, HONEST)])];
        ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            &store,
            other_shard(),
            root_after(&other_verified),
            version_after(&other_verified),
        )
        .sync_from_stream(
            &mut StateSyncStats::default(),
            None,
            stream::iter(vec![
                batch_in(other_shard(), 1, other_verified[0].1.clone()),
                complete_in(other_shard(), 1),
            ]),
        )
        .await
        .unwrap();

        let result = interrupt_sync(&store, shard(), vec![destroy_in(other_shard(), HONEST)]);

        assert!(
            matches!(result, Some(Err(RpcStateSyncError::InvalidResponse(_)))),
            "{result:?}"
        );
        let other = store
            .with_read_tx(|tx| tx.substates_get(&address_in(other_shard(), HONEST)))
            .unwrap();
        assert!(other.is_up());
        assert_eq!(rewind_point(&store), None);
    }

    #[tokio::test]
    async fn discarding_all_unverified_state_rewinds_every_shard() {
        let (store, _tmp) = create_store();
        assert!(interrupt_sync(&store, shard(), vec![create(POISON)]).is_none());
        assert!(interrupt_sync(&store, other_shard(), vec![create_in(other_shard(), POISON, POISON)]).is_none());
        assert_eq!(
            store
                .with_read_tx(|tx| tx.state_sync_rewind_points_get_all())
                .unwrap()
                .len(),
            2
        );

        discard_all_unverified_state(&store).unwrap();

        let points = store.with_read_tx(|tx| tx.state_sync_rewind_points_get_all()).unwrap();
        assert!(points.is_empty(), "{points:?}");
        for shard in [shard(), other_shard()] {
            let poison = store
                .with_read_tx(|tx| tx.substates_get(&address_in(shard, POISON)).optional())
                .unwrap();
            assert!(poison.is_none(), "{shard}");
            let version = store
                .with_read_tx(|tx| tx.state_tree_versions_get_latest(shard))
                .unwrap();
            assert_eq!(version, None, "{shard}");
        }
    }

    /// Accepts every commit proof, standing in for a committee's signatures.
    struct AcceptAll;

    impl CommitProofValidator for AcceptAll {
        async fn validate(&self, _commit_proof: &CommittedBlockProof) -> Result<(), RpcStateSyncError> {
            Ok(())
        }
    }

    const N: Version = tari_ootle_storage::consensus_models::STATE_VERSION_PROOF_INTERVAL;

    /// `N` versions, each creating one substate, so that the last is a proof point.
    fn versions_to_proof_point() -> Versions {
        (1..=N)
            .map(|version| (version, vec![create(u8::try_from(version).unwrap())]))
            .collect()
    }

    /// A proof that the shard was at `state_version` with `shard_root`, in a block whose state root holds that leaf.
    fn version_proof(state_version: Version, shard_root: TreeHash) -> Result<SyncStateResponse, RpcStatus> {
        use tari_common_types::types::FixedHash;
        use tari_sidechain::{SidechainBlockCommitProof, SidechainBlockHeader};
        use tari_state_tree::ShardGroupRootTree;

        let protocol_version = ProtocolVersion::at(NETWORK, EPOCH);
        let root_tree = ShardGroupRootTree::build(protocol_version, [
            (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
            (shard(), shard_root, state_version),
        ])
        .unwrap();
        let state_merkle_root = root_tree.root();
        let (_, shard_root_proof) = root_tree.get_proof(shard()).unwrap();
        let header = SidechainBlockHeader {
            network: NETWORK.as_byte(),
            protocol_version: protocol_version.as_u32(),
            parent_id: FixedHash::zero(),
            justify_id: FixedHash::zero(),
            height: 1,
            epoch: EPOCH.as_u64(),
            epoch_hash: FixedHash::zero(),
            shard_group: tari_sidechain::ShardGroup {
                start: 1,
                end_inclusive: 256,
            },
            proposed_by: Default::default(),
            state_merkle_root: FixedHash::from(state_merkle_root.into_array()),
            command_merkle_root: FixedHash::zero(),
            transaction_merkle_root: None,
            signature: Default::default(),
            accumulated_data: Default::default(),
            metadata_hash: FixedHash::zero(),
        };
        let commit_proof = CommittedBlockProof::new(SidechainBlockCommitProof {
            header,
            proof_elements: vec![],
        });
        Ok(SyncStateResponse {
            response: Some(sync_state_response::Response::VersionProof(proto::StateVersionProof {
                shard: shard().as_u32(),
                state_version,
                commit_proof: commit_proof.to_bytes(),
                shard_root_proof: tari_bor::serde_codec::to_vec(&shard_root_proof).unwrap(),
            })),
        })
    }

    async fn sync_with_proofs<TStore: StateStore>(
        store: &TStore,
        checkpoint: &Versions,
        responses: Vec<Result<SyncStateResponse, RpcStatus>>,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let validator = AcceptAll;
        let sync = ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            store,
            shard(),
            root_after(checkpoint),
            version_after(checkpoint),
        )
        .with_version_proofs(&validator);
        let verified_version = sync.discard_unverified_state()?;
        sync.sync_from_stream(
            &mut StateSyncStats::default(),
            verified_version,
            stream::iter(responses),
        )
        .await
    }

    fn down_proof_record<TStore: StateStore>(
        store: &TStore,
        seed: u8,
    ) -> Option<tari_ootle_storage::consensus_models::SubstateDownProofRecord> {
        let id = VersionedSubstateId::new(substate_id_in(shard(), seed), SubstateVersion::ZERO);
        store
            .with_read_tx(|tx| tx.substate_down_proofs_get(shard(), &id))
            .unwrap()
    }

    /// A synced node that holds no proof of a version the substate was up at cannot prove it went down, so it records
    /// nothing and serves the Down unproven.
    #[tokio::test]
    async fn a_destruction_synced_without_a_proof_point_records_no_down_proof() {
        let (store, _tmp) = create_store();
        let versions = vec![(1, vec![create(HONEST), create(2)]), (2, vec![destroy(HONEST)])];
        sync_honestly(&store, &versions).await;

        assert!(held_proofs(&store).is_empty());
        assert!(substate(&store, HONEST).unwrap().is_destroyed());
        assert!(down_proof_record(&store, HONEST).is_none());
    }

    /// With a proof of a version the substate was up at, the same destruction is recorded with it.
    #[tokio::test]
    async fn a_destruction_synced_after_a_proof_point_records_a_down_proof() {
        let (store, _tmp) = create_store();
        let v1 = vec![(1, vec![create(HONEST), create(2)])];
        let versions = vec![v1[0].clone(), (2, vec![destroy(HONEST)])];
        let responses = vec![
            batch(1, v1[0].1.clone()),
            version_proof(1, root_after(&v1)),
            batch(2, vec![destroy(HONEST)]),
            complete(2),
        ];
        sync_with_proofs(&store, &versions, responses).await.unwrap();

        assert_eq!(held_proofs(&store), vec![1]);
        let record = down_proof_record(&store, HONEST).unwrap();
        assert_eq!(record.state_version, 1);
    }

    fn held_proofs<TStore: StateStore>(store: &TStore) -> Vec<Version> {
        store
            .with_read_tx(|tx| tx.state_version_proofs_get_range(shard(), 0, Version::MAX - 1))
            .unwrap()
            .into_iter()
            .map(|proof| proof.state_version)
            .collect()
    }

    #[tokio::test]
    async fn a_proven_stream_is_kept_with_its_proofs() {
        let (store, _tmp) = create_store();
        let checkpoint = versions_to_proof_point();
        let mut responses = stream_of(&checkpoint);
        responses.push(version_proof(N, root_after(&checkpoint)));
        responses.push(complete(N));

        sync_with_proofs(&store, &checkpoint, responses).await.unwrap();

        assert_eq!(local_version(&store), Some(N));
        assert_eq!(held_proofs(&store), vec![N]);
        assert_eq!(rewind_point(&store), None);
    }

    #[tokio::test]
    async fn a_stream_that_skips_a_proof_point_is_rejected() {
        let (store, _tmp) = create_store();
        let checkpoint = versions_to_proof_point();
        let mut responses = stream_of(&checkpoint);
        responses.push(complete(N));

        let err = sync_with_proofs(&store, &checkpoint, responses).await.unwrap_err();

        assert!(matches!(err, RpcStateSyncError::InvalidResponse(_)), "{err}");
        assert_eq!(local_version(&store), None);
        assert!(held_proofs(&store).is_empty());
    }

    #[tokio::test]
    async fn a_proof_of_a_root_the_stream_did_not_produce_is_rejected() {
        let (store, _tmp) = create_store();
        let checkpoint = versions_to_proof_point();
        let mut responses = stream_of(&checkpoint);
        responses.push(version_proof(N, TreeHash::new([POISON; 32])));
        responses.push(complete(N));

        let err = sync_with_proofs(&store, &checkpoint, responses).await.unwrap_err();

        assert!(matches!(err, RpcStateSyncError::InvalidResponse(_)), "{err}");
        assert_eq!(local_version(&store), None);
    }

    #[tokio::test]
    async fn a_failure_after_a_proof_keeps_the_proven_state() {
        let (store, _tmp) = create_store();
        let proven = versions_to_proof_point();
        let mut checkpoint = proven.clone();
        checkpoint.push((N + 1, vec![create(200)]));
        let mut responses = stream_of(&proven);
        responses.push(version_proof(N, root_after(&proven)));
        responses.push(batch(N + 1, vec![create(POISON)]));
        responses.push(complete(N + 1));

        let err = sync_with_proofs(&store, &checkpoint, responses).await.unwrap_err();

        assert!(matches!(err, RpcStateSyncError::StateRootMismatch { .. }), "{err}");
        assert_eq!(local_version(&store), Some(N));
        assert!(substate(&store, POISON).is_none());
    }

    #[tokio::test]
    async fn a_second_terminal_batch_at_a_committed_version_is_rejected_before_it_is_buffered() {
        let (store, _tmp) = create_store();
        let checkpoint = vec![(1, vec![create(HONEST)])];
        let sync = ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            &store,
            shard(),
            root_after(&checkpoint),
            version_after(&checkpoint),
        );
        let mut stats = StateSyncStats::default();

        let err = sync
            .apply_stream(
                &mut stats,
                None,
                stream::iter(vec![
                    batch(1, vec![create(HONEST)]),
                    batch(1, vec![create(POISON)]),
                    complete(1),
                ]),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(&err, RpcStateSyncError::InvalidResponse(e) if e.to_string().contains("not above the last committed state version v1")),
            "{err}"
        );
        assert_eq!(stats.total_transitions, 1);
        assert_eq!(local_version(&store), Some(1));
        assert!(substate(&store, HONEST).is_some_and(|s| s.is_up()));
        assert!(substate(&store, POISON).is_none());
        assert_eq!(rewind_point(&store), Some(0));
    }

    #[tokio::test]
    async fn a_version_streamed_over_several_batches_is_committed_whole() {
        let (store, _tmp) = create_store();
        let checkpoint = vec![(1, vec![create(HONEST), create(2)])];

        let synced = sync(&store, &checkpoint, vec![
            batch_with_more(1, vec![create(HONEST)]),
            batch(1, vec![create(2)]),
            complete(1),
        ])
        .await
        .unwrap();

        assert_eq!(synced, Some(1));
        assert!(substate(&store, HONEST).is_some_and(|s| s.is_up()));
        assert!(substate(&store, 2).is_some_and(|s| s.is_up()));
        assert_eq!(rewind_point(&store), None);
    }

    /// `count` versions, each creating one substate.
    fn unproven_versions(count: u64) -> Versions {
        (1..=count)
            .map(|version| (version, vec![create(u8::try_from(version).unwrap())]))
            .collect()
    }

    #[tokio::test]
    async fn a_stream_within_the_unverified_version_budget_is_kept() {
        let (store, _tmp) = create_store();
        let checkpoint = unproven_versions(MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT);
        let mut responses = stream_of(&checkpoint);
        responses.push(complete(MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT));

        sync(&store, &checkpoint, responses).await.unwrap();

        assert_eq!(local_version(&store), Some(MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT));
    }

    #[tokio::test]
    async fn a_stream_beyond_the_unverified_version_budget_is_rejected() {
        let (store, _tmp) = create_store();
        let checkpoint = unproven_versions(MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT + 1);
        let mut responses = stream_of(&checkpoint);
        responses.push(complete(MAX_UNVERIFIED_VERSIONS_PER_ATTEMPT + 1));

        let err = sync(&store, &checkpoint, responses).await.unwrap_err();

        assert!(
            matches!(&err, RpcStateSyncError::InvalidResponse(e) if e.to_string().contains("unverified version budget")),
            "{err}"
        );
        assert_eq!(local_version(&store), None);
        assert_eq!(rewind_point(&store), None);
    }

    #[tokio::test]
    async fn a_batch_above_the_checkpoint_is_rejected_before_it_is_buffered() {
        // A network launched before V1 holds a stream to the proof schedule only from its first proof.
        const PRE_V1_NETWORK: Network = Network::Esmeralda;
        assert!(ProtocolVersion::genesis(PRE_V1_NETWORK) < ProtocolVersion::V1);
        let (store, _tmp) = create_store();
        let checkpoint = vec![(1, vec![create(HONEST)])];
        let validator = AcceptAll;
        let sync = ShardSync::new(
            PRE_V1_NETWORK,
            NUM_PRESHARDS,
            &store,
            shard(),
            root_after(&checkpoint),
            version_after(&checkpoint),
        )
        .with_version_proofs(&validator)
        .with_trusted_checkpoint_version(true);
        let mut stats = StateSyncStats::default();

        let err = sync
            .sync_from_stream(
                &mut stats,
                None,
                stream::iter(vec![
                    batch(1, vec![create(HONEST)]),
                    batch(2, vec![create(POISON)]),
                    complete(2),
                ]),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(&err, RpcStateSyncError::ConflictsWithCheckpoint(e) if e.to_string().contains("above the checkpoint state version 1")),
            "{err}"
        );
        assert_eq!(stats.total_transitions, 1);
        assert!(substate(&store, POISON).is_none());
        assert_eq!(local_version(&store), None);
    }

    #[tokio::test]
    async fn an_untrusted_checkpoint_version_does_not_bound_the_stream() {
        const PRE_V1_NETWORK: Network = Network::Esmeralda;
        let (store, _tmp) = create_store();
        let checkpoint = vec![(1, vec![create(HONEST)])];
        let validator = AcceptAll;
        let sync = ShardSync::new(
            PRE_V1_NETWORK,
            NUM_PRESHARDS,
            &store,
            shard(),
            root_after(&checkpoint),
            version_after(&checkpoint),
        )
        .with_version_proofs(&validator)
        .with_trusted_checkpoint_version(false);
        let mut stats = StateSyncStats::default();

        let err = sync
            .sync_from_stream(
                &mut stats,
                None,
                stream::iter(vec![
                    batch(1, vec![create(HONEST)]),
                    batch(2, vec![create(POISON)]),
                    complete(2),
                ]),
            )
            .await
            .unwrap_err();

        assert!(matches!(err, RpcStateSyncError::StateRootMismatch { .. }), "{err}");
        assert_eq!(stats.total_transitions, 2);
        assert_eq!(local_version(&store), None);
    }

    #[tokio::test]
    async fn a_replayed_proof_is_rejected() {
        let (store, _tmp) = create_store();
        let checkpoint = versions_to_proof_point();
        let mut responses = stream_of(&checkpoint);
        responses.push(version_proof(N, root_after(&checkpoint)));
        responses.push(version_proof(N, root_after(&checkpoint)));
        responses.push(complete(N));

        let err = sync_with_proofs(&store, &checkpoint, responses).await.unwrap_err();

        assert!(
            matches!(&err, RpcStateSyncError::InvalidResponse(e) if e.to_string().contains("already proved")),
            "{err}"
        );
    }

    fn sync_with_timeout<'a, TStore: StateStore>(
        store: &'a TStore,
        checkpoint: &Versions,
        timeout: Duration,
    ) -> ShardSync<'a, TStore> {
        ShardSync::new(
            NETWORK,
            NUM_PRESHARDS,
            store,
            shard(),
            root_after(checkpoint),
            version_after(checkpoint),
        )
        .with_progress_timeout(timeout)
    }

    fn is_no_progress(err: &RpcStateSyncError) -> bool {
        matches!(err, RpcStateSyncError::InvalidResponse(e) if e.to_string().contains("made no progress"))
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_stream_is_abandoned() {
        let (store, _tmp) = create_store();
        let checkpoint = vec![(1, vec![create(HONEST), create(2)])];
        let stalled = stream::iter(vec![batch_with_more(1, vec![create(HONEST)])]).chain(stream::pending());

        let err = sync_with_timeout(&store, &checkpoint, Duration::from_millis(50))
            .sync_from_stream(&mut StateSyncStats::default(), None, stalled)
            .await
            .unwrap_err();

        assert!(is_no_progress(&err), "{err}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_that_trickles_out_one_version_is_abandoned() {
        let (store, _tmp) = create_store();
        let checkpoint = vec![(1, vec![create(HONEST)])];
        let trickle = stream::repeat_with(|| batch_with_more(1, vec![create(HONEST)])).then(|response| async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            response
        });

        let err = sync_with_timeout(&store, &checkpoint, Duration::from_millis(100))
            .sync_from_stream(&mut StateSyncStats::default(), None, Box::pin(trickle))
            .await
            .unwrap_err();

        assert!(is_no_progress(&err), "{err}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_that_keeps_committing_versions_outlasts_the_progress_timeout() {
        let (store, _tmp) = create_store();
        let checkpoint = unproven_versions(5);
        let mut responses = stream_of(&checkpoint);
        responses.push(complete(5));
        let paced = stream::iter(responses).then(|response| async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            response
        });

        sync_with_timeout(&store, &checkpoint, Duration::from_millis(80))
            .sync_from_stream(&mut StateSyncStats::default(), None, Box::pin(paced))
            .await
            .unwrap();

        assert_eq!(local_version(&store), Some(5));
    }

    #[tokio::test]
    async fn a_transport_failure_after_committing_a_version_is_the_peer_s_fault() {
        let (store, _tmp) = create_store();
        let checkpoint = vec![(1, vec![create(HONEST)]), (2, vec![create(2)])];

        let err = sync(&store, &checkpoint, vec![
            batch(1, vec![create(HONEST)]),
            Err(RpcStatus::general("connection lost")),
        ])
        .await
        .unwrap_err();

        assert!(
            matches!(&err, RpcStateSyncError::InvalidResponse(e) if e.to_string().contains("after committing unverified state")),
            "{err}"
        );
        assert_eq!(local_version(&store), None);
    }

    #[tokio::test]
    async fn a_transport_failure_before_committing_a_version_stays_a_transport_failure() {
        let (store, _tmp) = create_store();
        let checkpoint = vec![(1, vec![create(HONEST), create(2)])];

        let err = sync(&store, &checkpoint, vec![
            batch_with_more(1, vec![create(HONEST)]),
            Err(RpcStatus::general("connection lost")),
        ])
        .await
        .unwrap_err();

        assert!(matches!(err, RpcStateSyncError::RpcError(_)), "{err}");
    }

    #[test]
    fn a_zero_or_unrepresentable_progress_timeout_sets_no_deadline() {
        assert_eq!(next_progress_deadline(None), None);
        assert_eq!(next_progress_deadline(Some(Duration::ZERO)), None);
        assert_eq!(next_progress_deadline(Some(Duration::MAX)), None);
        assert!(next_progress_deadline(Some(Duration::from_secs(1))).is_some());
    }
}
