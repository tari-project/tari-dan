//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{num::NonZeroUsize, sync::Arc};

use log::*;
use tari_consensus::hotstuff::{HotstuffEvent, commit_proofs::state_version_commit_proof};
use tari_engine_types::published_template::MAX_TEMPLATE_BLOB_WIRE_BYTES;
use tari_epoch_manager::{EpochManagerReader, service::EpochManagerHandle};
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    ShardGroup,
    committee::CommitteeInfo,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_p2p::{PeerAddress, proto::rpc};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{
        EpochCheckpoint,
        StateTransition,
        StateVersionProof,
        StateVersionTransitions,
        SubstateValueFilterFlags,
        is_state_version_proof_point,
    },
};
use tari_rpc_framework::RpcStatus;
use tari_state_tree::Version;
use tokio::sync::{Semaphore, broadcast, mpsc};

use crate::{consensus::ConsensusHandle, p2p::rpc::CONSENSUS_NOT_RUNNING};

const LOG_TARGET: &str = "tari::ootle::rpc::sync_task";
/// The most encoded update bytes a single `SubstateBatch` carries. The rest of the response payload limit is left for
/// the batch's other fields and the response envelope.
const MAX_BATCH_UPDATE_BYTES: usize = 4 * 1024 * 1024;
/// The field number of `SubstateBatch.updates`, which frames each update in the encoded batch.
const SUBSTATE_BATCH_UPDATES_TAG: u32 = 2;

const _: () = assert!(
    MAX_BATCH_UPDATE_BYTES + 1024 <= tari_rpc_framework::max_response_payload_size(),
    "a full state sync batch must fit one RPC response"
);
const _: () = assert!(
    MAX_TEMPLATE_BLOB_WIRE_BYTES + 1024 * 1024 <= MAX_BATCH_UPDATE_BYTES,
    "the largest substate must fit a state sync batch on its own"
);

/// Where a shard's stream stands in proving the versions it streams.
#[derive(Debug, Clone, Copy)]
struct ProofProgress {
    /// The first version not yet considered for a proof.
    next: Version,
    /// The last version the stream covers: the shard's version in the checkpoint it syncs to.
    last: Version,
}

/// A validated resume point for a single shard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardCursor {
    pub shard: Shard,
    pub start_state_version: Version,
}

impl ShardCursor {
    /// True if `tip` holds a version this cursor has yet to stream.
    fn is_behind(&self, tip: Option<Version>) -> bool {
        tip.is_some_and(|tip| tip >= self.start_state_version)
    }

    /// Moves the resume point past `synced_to_version`. A node behind the caller's cursor reports a
    /// tip below it; the cursor only ever advances.
    fn advance_past(&mut self, synced_to_version: Version) {
        self.start_state_version = self.start_state_version.max(synced_to_version + 1);
    }

    /// Validates a caller-supplied cursor list.
    ///
    /// Ascending order rules out duplicates and lets the responder stream each shard contiguously,
    /// which is what allows a consumer to finalise a shard the moment its completion marker arrives.
    pub fn validate_all(cursors: Vec<rpc::ShardCursor>) -> Result<Vec<Self>, RpcStatus> {
        if cursors.is_empty() {
            return Err(RpcStatus::bad_request("At least one shard cursor must be provided"));
        }
        // The global shard is streamable alongside every preshard, so one more than the preshard count.
        let max_cursors = NumPreshards::MAX.num_shards() + 1;
        if cursors.len() > max_cursors {
            return Err(RpcStatus::bad_request(format!(
                "Too many shard cursors: {}. At most {max_cursors} may be requested",
                cursors.len(),
            )));
        }

        let mut validated = Vec::with_capacity(cursors.len());
        let mut prev_shard = None::<Shard>;
        for cursor in cursors {
            let shard = Shard::from_u32(cursor.shard);
            if shard > NumPreshards::MAX_SHARD {
                return Err(RpcStatus::bad_request(format!(
                    "Shard {shard} out of range. Maximum shard is {}",
                    NumPreshards::MAX_SHARD
                )));
            }
            if prev_shard.is_some_and(|prev| shard <= prev) {
                return Err(RpcStatus::bad_request(
                    "Shard cursors must be ordered by strictly ascending shard",
                ));
            }
            // Genesis is committed at version 0 and is never synced - every node bootstraps it.
            if cursor.start_state_version == 0 {
                return Err(RpcStatus::bad_request("start_state_version must be greater than 0"));
            }
            prev_shard = Some(shard);
            validated.push(Self {
                shard,
                start_state_version: cursor.start_state_version,
            });
        }

        Ok(validated)
    }

    /// Rejects any cursor for a shard that `committee_info` does not cover.
    pub fn ensure_all_stored(cursors: &[Self], committee_info: &CommitteeInfo) -> Result<(), RpcStatus> {
        cursors
            .iter()
            .try_for_each(|cursor| ensure_shard_is_stored(cursor.shard, committee_info))
    }
}

/// The shards whose history through an epoch this node holds, final and complete. It is final once the committee
/// storing the shard committed the epoch, which a stored checkpoint for that epoch attests. It is complete if this
/// node committed the epoch for the shard itself, as a member of the committee storing it at that epoch, or
/// state-synced the shard from the checkpoint, as a member of the committee storing it at the next epoch. State
/// sync stores a checkpoint only once every shard it takes from it matches it, the global shard first.
#[derive(Debug, Clone)]
pub struct HeldHistory {
    /// The shard groups of the checkpoints stored for the epoch.
    pub checkpoint_shard_groups: Vec<ShardGroup>,
    /// This node's shard group at the epoch, if it was registered.
    pub committed_as: Option<ShardGroup>,
    /// This node's shard group at the next epoch, if it was registered.
    pub synced_as: Option<ShardGroup>,
}

impl HeldHistory {
    pub fn holds(&self, shard: Shard) -> bool {
        let is_member = [self.committed_as, self.synced_as]
            .into_iter()
            .flatten()
            .any(|sg| sg.contains_or_global(&shard));
        is_member &&
            self.checkpoint_shard_groups
                .iter()
                .any(|sg| sg.contains_or_global(&shard))
    }

    /// Rejects the request unless this node holds the history of every requested shard.
    pub fn ensure_holds_all(&self, cursors: &[ShardCursor], epoch: Epoch) -> Result<(), RpcStatus> {
        match cursors.iter().find(|cursor| !self.holds(cursor.shard)) {
            Some(cursor) => Err(RpcStatus::unavailable(format!(
                "This node does not hold committed state for {} through epoch {epoch}",
                cursor.shard
            ))),
            None => Ok(()),
        }
    }
}

/// Rejects a bounded request for history through an epoch this node has not reached, which it cannot hold. A
/// node lagging the caller's view of the base layer answers as unavailable so that the caller tries another.
pub fn ensure_epoch_reached(until_epoch: Epoch, current_epoch: Epoch) -> Result<(), RpcStatus> {
    if until_epoch > current_epoch {
        return Err(RpcStatus::unavailable(format!(
            "This node has not reached epoch {until_epoch} (current epoch is {current_epoch})"
        )));
    }
    Ok(())
}

/// Rejects a bounded request for history through `until_epoch` once this node's epoch GC may have pruned a value the
/// stream must carry. GC prunes the value of a substate destroyed at epoch `d` once the store reaches epoch
/// `d + epoch_history_length`. Past `until_epoch + epoch_history_length`, a substate that is live at `until_epoch` and
/// destroyed after it can therefore be left with only its hash, and the stream ends before the destruction that would
/// account for the missing value.
pub fn ensure_history_values_held(
    until_epoch: Epoch,
    store_epoch: Epoch,
    epoch_history_length: u64,
) -> Result<(), RpcStatus> {
    let horizon = until_epoch.checked_add(Epoch(epoch_history_length));
    if horizon.is_some_and(|horizon| store_epoch > horizon) {
        return Err(RpcStatus::unavailable(format!(
            "This node has pruned substate values that history through epoch {until_epoch} needs (current epoch is \
             {store_epoch})"
        )));
    }
    Ok(())
}

/// Rejects a shard that `committee_info` does not cover. Every committee holds the global shard, so it
/// is always in range.
fn ensure_shard_is_stored(shard: Shard, committee_info: &CommitteeInfo) -> Result<(), RpcStatus> {
    let shard_group = committee_info.shard_group();
    if shard_group.contains_or_global(&shard) {
        Ok(())
    } else {
        Err(RpcStatus::bad_request(format!(
            "This node in {shard_group} does not store {shard}"
        )))
    }
}

/// This node's warrant to tell a caller that it is level with the committee: the epoch the warrant was
/// established at, and the committee that epoch placed this node in.
///
/// It covers a shard only for as long as that committee stores it, so it carries the committee rather
/// than the fact that some check once passed - a group can shrink or reshuffle at an epoch boundary
/// while still containing the shard the boundary was noticed on.
#[derive(Debug, Clone)]
pub struct TipAuthority {
    epoch: Epoch,
    committee_info: CommitteeInfo,
}

impl TipAuthority {
    pub fn new(epoch: Epoch, committee_info: CommitteeInfo) -> Self {
        Self { epoch, committee_info }
    }

    /// True once `epoch` has moved on from the one this warrant was established at, leaving it saying
    /// nothing about the committee now.
    fn is_stale_at(&self, epoch: Epoch) -> bool {
        self.epoch != epoch
    }

    fn ensure_stores(&self, shard: Shard) -> Result<(), RpcStatus> {
        ensure_shard_is_stored(shard, &self.committee_info)
    }
}

pub struct StateSyncTask<TStateStore: StateStore> {
    store: TStateStore,
    sender: mpsc::Sender<Result<rpc::SyncStateResponse, RpcStatus>>,
    cursors: Vec<ShardCursor>,
    end_epoch: Option<Epoch>,
    consensus: ConsensusHandle,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    /// This node's warrant to serve its tip, for a stream that claims to. `None` for a bounded stream,
    /// which makes no such claim.
    tip_authority: Option<TipAuthority>,
    /// Whether to hold the stream open at the tip and keep streaming as this node commits.
    follow: bool,
    batch_size: NonZeroUsize,
    value_filters: SubstateValueFilterFlags,
    /// Whether to interleave state version proofs, which only a bounded stream does.
    include_version_proofs: bool,
    /// The state version reads every stream on this node shares. A permit is held for one version's read and
    /// released before its batches are sent, so a stream waiting on its peer holds none.
    reads: Arc<Semaphore>,
}

impl<TStateStore: StateStore> StateSyncTask<TStateStore> {
    pub fn new(
        store: TStateStore,
        sender: mpsc::Sender<Result<rpc::SyncStateResponse, RpcStatus>>,
        cursors: Vec<ShardCursor>,
        end_epoch: Option<Epoch>,
        consensus: ConsensusHandle,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        tip_authority: Option<TipAuthority>,
        follow: bool,
        batch_size: NonZeroUsize,
        value_filters: SubstateValueFilterFlags,
        include_version_proofs: bool,
        reads: Arc<Semaphore>,
    ) -> Self {
        Self {
            store,
            sender,
            cursors,
            end_epoch,
            consensus,
            epoch_manager,
            tip_authority,
            follow,
            batch_size,
            value_filters,
            include_version_proofs,
            reads,
        }
    }

    pub async fn run(mut self) -> Result<(), ()> {
        // Each shard's updates are streamed contiguously, in the order the caller listed them, so a
        // consumer can finalise a shard the moment its completion marker arrives.
        let mut cursors = std::mem::take(&mut self.cursors);
        let Some(last_index) = cursors.len().checked_sub(1) else {
            // Every stream must carry a final marker, so an empty request cannot be answered with an
            // empty stream. `ShardCursor::validate_all` rejects one before it reaches here.
            self.send(Err(RpcStatus::bad_request("No shard cursors were provided")))
                .await?;
            return Err(());
        };

        // Subscribed before the catch-up so that a commit landing during it is not missed.
        let mut events = if self.follow {
            match self.consensus.subscribe_to_hotstuff_events() {
                Ok(events) => Some(events),
                Err(err) => {
                    error!(target: LOG_TARGET, "🌍 Failed to subscribe to consensus events: {}", err);
                    self.send(Err(RpcStatus::general("Consensus events are unavailable")))
                        .await?;
                    return Err(());
                },
            }
        } else {
            None
        };

        for (i, cursor) in cursors.iter_mut().enumerate() {
            let is_final = events.is_none() && i == last_index;
            self.run_for_shard(cursor, is_final).await?;
        }

        match events.as_mut() {
            Some(events) => self.follow_tip(&mut cursors, events).await,
            None => Ok(()),
        }
    }

    /// Keeps streaming past the tip: each commit is followed by a pass over every cursor, and a
    /// shard that moved is streamed and closed off with a further marker.
    ///
    /// The commit event is published from inside the committing transaction, so a pass may run
    /// before the transitions it announces are visible; the next commit's pass picks them up, and
    /// the pacemaker commits on a timer so that pass is never far off. An epoch change forces a
    /// marker for every shard whether or not it moved: the marker re-establishes this node's
    /// warrant at the new epoch, which is what ends the stream promptly for a shard this node no
    /// longer stores, and tells the caller the epoch it is now level as of.
    async fn follow_tip(
        &mut self,
        cursors: &mut [ShardCursor],
        events: &mut broadcast::Receiver<HotstuffEvent>,
    ) -> Result<(), ()> {
        loop {
            // The client going away is otherwise only noticed by the next send, a block time later.
            let event = tokio::select! {
                event = events.recv() => event,
                _ = self.sender.closed() => {
                    debug!(target: LOG_TARGET, "Peer stream closed by client. Ending followed stream");
                    return Err(());
                },
            };
            let force_marker = match event {
                Ok(HotstuffEvent::BlockCommitted { .. }) => false,
                Ok(HotstuffEvent::EpochChanged { .. }) => true,
                Ok(_) => continue,
                // Dropped events are only ever commits or an epoch change, both answered by a pass.
                Err(broadcast::error::RecvError::Lagged(_)) => true,
                Err(broadcast::error::RecvError::Closed) => {
                    debug!(target: LOG_TARGET, "🌍 Consensus stopped. Ending followed stream");
                    return Ok(());
                },
            };
            for cursor in cursors.iter_mut() {
                self.follow_shard(cursor, force_marker).await?;
            }
        }
    }

    async fn follow_shard(&mut self, cursor: &mut ShardCursor, force_marker: bool) -> Result<(), ()> {
        let tip_at_start = self.snapshot_tip(cursor.shard).await?;
        if !cursor.is_behind(tip_at_start) && !force_marker {
            return Ok(());
        }
        self.stream_shard(cursor, tip_at_start, false).await
    }

    async fn run_for_shard(&mut self, cursor: &mut ShardCursor, is_final: bool) -> Result<(), ()> {
        // For an unbounded (sync-to-tip) request, snapshot the committed tree tip before scanning. The
        // completion marker advances the client over trailing versions that stream no updates (all
        // filtered out for its subscription). Snapshotting first ensures the marker never reports
        // beyond what we streamed: anything committed after this point is left for the next round.
        let tip_at_start = if self.end_epoch.is_none() {
            self.snapshot_tip(cursor.shard).await?
        } else {
            None
        };
        self.stream_shard(cursor, tip_at_start, is_final).await
    }

    async fn snapshot_tip(&mut self, shard: Shard) -> Result<Option<Version>, ()> {
        match self.read_latest_tree_version(shard) {
            Ok(version) => Ok(version),
            Err(err) => {
                error!(target: LOG_TARGET, "🌍 Error reading latest tree version for {}: {}", shard, err);
                self.send(Err(RpcStatus::log_internal_error(LOG_TARGET)(err))).await?;
                Err(())
            },
        }
    }

    /// Streams `cursor`'s shard from its resume point and closes it off with a marker, advancing the
    /// cursor past the version the marker reports.
    async fn stream_shard(
        &mut self,
        cursor: &mut ShardCursor,
        tip_at_start: Option<Version>,
        is_final: bool,
    ) -> Result<(), ()> {
        let shard = cursor.shard;
        let mut current_state_version = cursor.start_state_version;
        let mut counter = 0usize;
        let mut last_sent_version: Option<Version> = None;
        let mut proofs = match self.proof_progress(cursor) {
            Ok(proofs) => proofs,
            Err(err) => {
                error!(target: LOG_TARGET, "🌍 Error reading the checkpoint version of {}: {}", shard, err);
                self.send(Err(RpcStatus::log_internal_error(LOG_TARGET)(err))).await?;
                return Err(());
            },
        };
        loop {
            // A version whose updates are all filtered out sends nothing, so the scan checks for its peer before
            // every version it reads.
            let read = tokio::select! {
                biased;
                _ = self.sender.closed() => {
                    debug!(target: LOG_TARGET, "Peer stream closed by client during the scan of {}. Aborting", shard);
                    return Err(());
                },
                read = self.reads.acquire() => read.map_err(|_| ())?,
            };
            let fetched = self.fetch_next_batch(shard, current_state_version);
            drop(read);
            match fetched {
                Ok(Some(transitions)) => {
                    if let Some(end_epoch) = self.end_epoch {
                        // TODO(perf): might be better to not load in the first place, however also might incur the cost
                        // of a db index, more complex keys or loading from db anyway
                        if transitions.epoch > end_epoch {
                            info!(target: LOG_TARGET, "🌍 Reached end of requested epoch: {}", end_epoch);
                            break;
                        }
                    }
                    if !transitions.updates.is_empty() {
                        debug!(target: LOG_TARGET, "🌍 Fetched {} state transition(s) for {} up to v{}", transitions.updates.len(), shard, transitions.state_version);
                    }

                    current_state_version = transitions.state_version + 1;
                    counter += transitions.updates.len();

                    let state_version = transitions.state_version;
                    let has_updates = !transitions.updates.is_empty();
                    if let Some(proofs) = proofs.as_mut() {
                        self.send_proof_points_before(shard, proofs, state_version).await?;
                    }
                    self.send_batches(transitions).await?;
                    if let Some(proofs) = proofs.as_mut() {
                        self.send_proof_after_version(shard, proofs, state_version).await?;
                    }
                    // A version whose updates are all filtered out streams no batch, so only versions we
                    // actually sent count towards the client's recorded progress.
                    if has_updates {
                        last_sent_version = Some(state_version);
                    }
                    // Reads are synchronous and a filtered-out version awaits nothing, so the scan yields after every
                    // version to share its runtime worker.
                    tokio::task::yield_now().await;
                },
                Ok(None) => {
                    // TODO: differentiate between not found and end of stream
                    debug!(target: LOG_TARGET, "🌍sync complete for {} ({}). {} update(s) sent.", shard, current_state_version, counter);
                    break;
                },
                Err(err) => {
                    error!(target: LOG_TARGET, "🌍 Error fetching state transitions: {}", err);
                    self.send(Err(RpcStatus::log_internal_error(LOG_TARGET)(err))).await?;
                    return Err(());
                },
            }
        }

        if let Some(proofs) = proofs.as_mut() {
            let end = proofs.last.saturating_add(1);
            self.send_proof_points_before(shard, proofs, end).await?;
        }

        let synced_to_version = self
            .send_complete(cursor, tip_at_start, last_sent_version, is_final)
            .await?;
        cursor.advance_past(synced_to_version);
        Ok(())
    }

    fn read_latest_tree_version(&self, shard: Shard) -> Result<Option<Version>, StorageError> {
        self.store.with_read_tx(|tx| tx.state_tree_versions_get_latest(shard))
    }

    /// Closes off a shard with a `SyncComplete` stating the version the client is now synced to, and
    /// returns that version.
    ///
    /// For an unbounded request this is the committed tree tip (capped to what we streamed), letting the
    /// client advance over trailing versions that streamed no updates - e.g. a shard whose latest
    /// transitions are all substate types the client filtered out. Such a shard otherwise streams no
    /// message at all, so the client could never observe that it has caught up and would re-scan it from
    /// scratch every round, leaving any version comparison against the committed version unsatisfiable.
    ///
    /// For a bounded request the consumer verifies against its own checkpoint, so the reported version is
    /// just our last streamed version - the consumer does not trust it as the sync target.
    ///
    /// The marker asserts that the caller is level with this node as of the epoch it names, so its epoch
    /// and this node's standing to make that claim are one fact, established together at send time by
    /// `authorise_tip`.
    async fn send_complete(
        &mut self,
        cursor: &ShardCursor,
        tip_at_start: Option<Version>,
        last_sent_version: Option<Version>,
        is_final: bool,
    ) -> Result<Version, ()> {
        let epoch = match self.authorise_tip(cursor.shard).await {
            Ok(epoch) => epoch,
            Err(status) => {
                self.send(Err(status)).await?;
                return Err(());
            },
        };

        let synced_to_version = match tip_at_start {
            // Unbounded: advance to the committed tip, but never past a version we actually streamed.
            Some(tip) => tip.max(last_sent_version.unwrap_or(0)),
            // Bounded, or an unbounded shard with no committed state: report the last streamed version.
            None => last_sent_version.unwrap_or_else(|| cursor.start_state_version.saturating_sub(1)),
        };

        self.send(Ok(rpc::SyncStateResponse {
            response: Some(rpc::sync_state_response::Response::Complete(rpc::SyncComplete {
                synced_to_version,
                epoch: Some(epoch.into()),
                shard: cursor.shard.as_u32(),
                is_final,
            })),
        }))
        .await?;
        Ok(synced_to_version)
    }

    /// Establishes this node's standing to tell the caller that it is level with the committee for
    /// `shard`, and returns the epoch that claim is made as of.
    ///
    /// Only an unbounded stream makes the claim: a bounded stream is answered out of history, which the
    /// caller verifies against its own checkpoint, so it needs nothing of this node's present standing.
    ///
    /// The claim is anchored to a single epoch - the one whose committee was checked to store these
    /// shards - and a stream can outlive it. When the epoch moves on beneath the stream the warrant is
    /// re-established at the new epoch, and every shard is checked against it, since a boundary can
    /// leave the committee holding some of the streamed shards and not others.
    async fn authorise_tip(&mut self, shard: Shard) -> Result<Epoch, RpcStatus> {
        let epoch = self.consensus.current_epoch();
        let Some(mut authority) = self.tip_authority.as_ref() else {
            return Ok(epoch);
        };

        // Only a node participating in consensus is receiving the transitions it is claiming to be
        // level on, and it can stop participating while the stream is open.
        if !self.consensus.is_running() {
            return Err(RpcStatus::unavailable(CONSENSUS_NOT_RUNNING));
        }

        if authority.is_stale_at(epoch) {
            let committee_info = self
                .epoch_manager
                .get_local_committee_info(epoch)
                .await
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            authority = self.tip_authority.insert(TipAuthority::new(epoch, committee_info));
        }

        authority.ensure_stores(shard)?;

        Ok(epoch)
    }

    fn fetch_next_batch(
        &self,
        shard: Shard,
        current_state_version: Version,
    ) -> Result<Option<StateVersionTransitions>, StorageError> {
        let transitions = self.store.with_read_tx(|tx| {
            StateTransition::get_for_shard(tx, shard, current_state_version, self.value_filters).optional()
        })?;
        Ok(transitions)
    }

    async fn send(&mut self, result: Result<rpc::SyncStateResponse, RpcStatus>) -> Result<(), ()> {
        if self.sender.send(result).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "Peer stream closed by client before completing. Aborting"
            );
            return Err(());
        }
        Ok(())
    }

    /// The proof progress of a bounded stream for `cursor`'s shard, or `None` if the caller did not ask for proofs.
    fn proof_progress(&self, cursor: &ShardCursor) -> Result<Option<ProofProgress>, StorageError> {
        let (true, Some(end_epoch)) = (self.include_version_proofs, self.end_epoch) else {
            return Ok(None);
        };
        let last = self.store.with_read_tx(|tx| {
            let checkpoint = EpochCheckpoint::get_all_for_epoch(tx, end_epoch)?
                .into_iter()
                .find(|checkpoint| {
                    checkpoint
                        .checked_shard_group()
                        .is_ok_and(|group| group.contains_or_global(&cursor.shard))
                })
                .ok_or_else(|| StorageError::NotFound {
                    item: "EpochCheckpoint",
                    key: format!("{} at epoch {end_epoch}", cursor.shard),
                })?;
            Ok::<_, StorageError>(checkpoint.get_shard_state_version(cursor.shard))
        })?;
        Ok(Some(ProofProgress {
            next: cursor.start_state_version,
            last,
        }))
    }

    /// Sends a proof for every proof point from `proofs.next` up to `until`, exclusive, and no further than the
    /// last version the stream covers. Versions in that range changed no substate, so the caller has already
    /// written the root each of them is proven against.
    async fn send_proof_points_before(
        &mut self,
        shard: Shard,
        proofs: &mut ProofProgress,
        until: Version,
    ) -> Result<(), ()> {
        let to = until.min(proofs.last.saturating_add(1));
        if proofs.next < to {
            let held = self.read_proofs(shard, proofs.next, to - 1).await?;
            for proof in held {
                if is_state_version_proof_point(proof.state_version) {
                    self.send_proof(proof).await?;
                }
            }
        }
        proofs.next = proofs.next.max(until);
        Ok(())
    }

    /// Proves `state_version`, whose updates were just sent, if it is a proof point.
    async fn send_proof_after_version(
        &mut self,
        shard: Shard,
        proofs: &mut ProofProgress,
        state_version: Version,
    ) -> Result<(), ()> {
        proofs.next = state_version.saturating_add(1);
        if state_version > proofs.last || !is_state_version_proof_point(state_version) {
            return Ok(());
        }
        if let Some(proof) = self.read_proofs(shard, state_version, state_version).await?.pop() {
            self.send_proof(proof).await?;
        }
        Ok(())
    }

    async fn read_proofs(&mut self, shard: Shard, from: Version, to: Version) -> Result<Vec<StateVersionProof>, ()> {
        match self
            .store
            .with_read_tx(|tx| tx.state_version_proofs_get_range(shard, from, to))
        {
            Ok(proofs) => Ok(proofs),
            Err(err) => {
                error!(target: LOG_TARGET, "🌍 Error reading state version proofs for {}: {}", shard, err);
                self.send(Err(RpcStatus::log_internal_error(LOG_TARGET)(err))).await?;
                Err(())
            },
        }
    }

    async fn send_proof(&mut self, proof: StateVersionProof) -> Result<(), ()> {
        let message = match self.store.with_read_tx(|tx| version_proof_message(tx, proof)) {
            Ok(message) => message,
            Err(err) => {
                error!(target: LOG_TARGET, "🌍 Error building a state version proof: {}", err);
                self.send(Err(RpcStatus::log_internal_error(LOG_TARGET)(err))).await?;
                return Err(());
            },
        };
        self.send(Ok(rpc::SyncStateResponse {
            response: Some(rpc::sync_state_response::Response::VersionProof(message)),
        }))
        .await
    }

    /// Sends `transitions` in batches.
    async fn send_batches(&mut self, transitions: StateVersionTransitions) -> Result<(), ()> {
        let batches = match into_batches(transitions, self.batch_size) {
            Ok(batches) => batches,
            Err(status) => {
                error!(target: LOG_TARGET, "🌍 {}", status);
                self.send(Err(status)).await?;
                return Err(());
            },
        };

        for batch in batches {
            self.send(Ok(rpc::SyncStateResponse {
                response: Some(rpc::sync_state_response::Response::Batch(batch)),
            }))
            .await?;
        }

        Ok(())
    }
}

/// Splits one state version's transitions into the batches that stream it, all but the last marked `has_more`.
///
/// Each batch is one RPC response, so it is bounded by encoded size as well as by `max_updates`: a single block can
/// commit far more substate bytes to a shard than one response carries. Fails if one update alone exceeds
/// `MAX_BATCH_UPDATE_BYTES`, which no substate the engine commits does.
fn into_batches(
    transitions: StateVersionTransitions,
    max_updates: NonZeroUsize,
) -> Result<Vec<rpc::SubstateBatch>, RpcStatus> {
    let StateVersionTransitions {
        epoch,
        shard,
        state_version,
        updates,
    } = transitions;
    let empty_batch = || rpc::SubstateBatch {
        state_version,
        updates: Vec::new(),
        has_more: true,
        epoch: Some(epoch.into()),
        shard: shard.as_u32(),
    };

    let mut batches = Vec::new();
    let mut batch = empty_batch();
    let mut batch_bytes = 0usize;
    for update in updates {
        let substate_id = update.substate_id().clone();
        let update = rpc::SubstateUpdate::from(update);
        let update_bytes = prost::encoding::message::encoded_len(SUBSTATE_BATCH_UPDATES_TAG, &update);
        if update_bytes > MAX_BATCH_UPDATE_BYTES {
            return Err(RpcStatus::general(format!(
                "Substate {substate_id} at state version {state_version} of {shard} encodes to {update_bytes} bytes, \
                 more than the {MAX_BATCH_UPDATE_BYTES} a state sync batch carries"
            )));
        }
        if batch.updates.len() >= max_updates.get() || batch_bytes + update_bytes > MAX_BATCH_UPDATE_BYTES {
            batches.push(std::mem::replace(&mut batch, empty_batch()));
            batch_bytes = 0;
        }
        batch.updates.push(update);
        batch_bytes += update_bytes;
    }
    if !batch.updates.is_empty() {
        batch.has_more = false;
        batches.push(batch);
    }
    Ok(batches)
}

/// The wire form of `proof`.
fn version_proof_message<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    proof: StateVersionProof,
) -> Result<rpc::StateVersionProof, StorageError> {
    let commit_proof = state_version_commit_proof(tx, proof.source).map_err(|e| StorageError::QueryError {
        reason: format!("commit proof of {} v{}: {e}", proof.shard, proof.state_version),
    })?;
    let shard_root_proof =
        tari_bor::serde_codec::to_vec(&proof.shard_root_proof).map_err(|e| StorageError::QueryError {
            reason: format!("encode shard root proof: {e}"),
        })?;
    Ok(rpc::StateVersionProof {
        shard: proof.shard.as_u32(),
        state_version: proof.state_version,
        commit_proof,
        shard_root_proof,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU64, AtomicUsize, Ordering},
        time::Duration,
    };

    use prost::Message;
    use tari_engine_types::{
        limits::ENGINE_LIMITS,
        published_template::{PublishedTemplate, PublishedTemplateAddress},
        substate::SubstateId,
    };
    use tari_epoch_manager::service::CommitteeCache;
    use tari_ootle_common_types::{SubstateVersion, VotePower};
    use tari_ootle_storage::{
        StateStoreWriteTransaction,
        consensus_models::{
            SubstateCreate,
            SubstateData,
            SubstateDestroy,
            SubstateTransition,
            SubstateUpdateBatch,
            SubstateUpdateProof,
            SubstateValueOrHash,
        },
    };
    use tari_ootle_transaction::Network;
    use tari_rpc_framework::max_response_payload_size;
    use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
    use tari_template_lib::types::{Hash32, crypto::RistrettoPublicKeyBytes};
    use tari_validator_node_rpc::STATE_SYNC_MAX_BATCH_SIZE;

    use super::*;

    fn max_updates() -> NonZeroUsize {
        NonZeroUsize::new(STATE_SYNC_MAX_BATCH_SIZE).unwrap()
    }

    fn template_id(seed: u16) -> SubstateId {
        let mut hash = [0u8; 32];
        hash[..2].copy_from_slice(&seed.to_le_bytes());
        SubstateId::Template(PublishedTemplateAddress::from(Hash32::from_array(hash)))
    }

    fn published_template(binary_len: usize) -> PublishedTemplate {
        PublishedTemplate {
            template_name: "test".try_into().unwrap(),
            author: RistrettoPublicKeyBytes::default(),
            binary: vec![0xAB; binary_len].try_into().unwrap(),
            at_epoch: 0,
            metadata_hash: None,
        }
    }

    fn create_template(seed: u16, binary_len: usize) -> SubstateUpdateProof {
        SubstateUpdateProof::Create(Box::new(SubstateCreate {
            substate: SubstateData {
                substate_id: template_id(seed),
                version: SubstateVersion::ZERO,
                value: SubstateValueOrHash::Value(Box::new(published_template(binary_len).into())),
                template_metadata: None,
            },
        }))
    }

    fn destroy(seed: u16) -> SubstateUpdateProof {
        SubstateUpdateProof::Destroy(SubstateDestroy {
            substate_id: template_id(seed),
            version: SubstateVersion::ZERO,
        })
    }

    fn transitions(updates: Vec<SubstateUpdateProof>) -> StateVersionTransitions {
        StateVersionTransitions {
            epoch: Epoch(1),
            shard: Shard::from_u32(1),
            state_version: 7,
            updates,
        }
    }

    fn response_len(batch: &rpc::SubstateBatch) -> usize {
        rpc::SyncStateResponse {
            response: Some(rpc::sync_state_response::Response::Batch(batch.clone())),
        }
        .encoded_len()
    }

    fn substate_ids(batches: &[rpc::SubstateBatch]) -> Vec<SubstateId> {
        batches
            .iter()
            .flat_map(|batch| &batch.updates)
            .map(|update| match update.update.as_ref().unwrap() {
                rpc::substate_update::Update::Create(create) => {
                    SubstateId::from_bytes(&create.substate.as_ref().unwrap().substate_id).unwrap()
                },
                rpc::substate_update::Update::Destroy(destroy) => SubstateId::from_bytes(&destroy.substate_id).unwrap(),
            })
            .collect()
    }

    fn assert_has_more_on_all_but_last(batches: &[rpc::SubstateBatch]) {
        let (last, rest) = batches.split_last().unwrap();
        assert!(rest.iter().all(|batch| batch.has_more));
        assert!(!last.has_more);
    }

    #[test]
    fn every_batch_of_a_version_fits_one_rpc_response() {
        // Fewer updates than the count limit, each of them a substate the engine admits, together far
        // larger than one response can carry.
        let updates = (0..16)
            .map(|seed| create_template(seed, ENGINE_LIMITS.max_template_binary_size_bytes))
            .collect::<Vec<_>>();
        let expected_ids = updates.iter().map(|u| u.substate_id().clone()).collect::<Vec<_>>();

        let batches = into_batches(transitions(updates), max_updates()).unwrap();

        assert!(batches.len() > 1);
        for batch in &batches {
            let len = response_len(batch);
            assert!(
                len <= max_response_payload_size(),
                "a batch of {} update(s) encodes to {len} bytes, more than the {} an RPC response carries",
                batch.updates.len(),
                max_response_payload_size()
            );
            assert_eq!(batch.state_version, 7);
            assert_eq!(batch.shard, 1);
        }
        assert_has_more_on_all_but_last(&batches);
        assert_eq!(substate_ids(&batches), expected_ids);
    }

    #[test]
    fn small_updates_are_batched_by_count() {
        let updates = (0..250).map(destroy).collect::<Vec<_>>();
        let expected_ids = updates.iter().map(|u| u.substate_id().clone()).collect::<Vec<_>>();

        let batches = into_batches(transitions(updates), max_updates()).unwrap();

        let sizes = batches.iter().map(|batch| batch.updates.len()).collect::<Vec<_>>();
        assert_eq!(sizes, vec![100, 100, 50]);
        assert_has_more_on_all_but_last(&batches);
        assert_eq!(substate_ids(&batches), expected_ids);
    }

    #[test]
    fn the_largest_template_a_substate_can_carry_streams_in_one_batch() {
        let updates = vec![
            destroy(1),
            create_template(2, MAX_TEMPLATE_BLOB_WIRE_BYTES),
            create_template(3, MAX_TEMPLATE_BLOB_WIRE_BYTES),
        ];

        let batches = into_batches(transitions(updates), max_updates()).unwrap();

        for batch in &batches {
            assert!(response_len(batch) <= max_response_payload_size());
        }
        assert_has_more_on_all_but_last(&batches);
        assert_eq!(substate_ids(&batches), vec![
            template_id(1),
            template_id(2),
            template_id(3)
        ]);
    }

    #[test]
    fn a_version_with_no_updates_has_no_batches() {
        assert!(into_batches(transitions(vec![]), max_updates()).unwrap().is_empty());
    }

    fn committee_info(start: u32, end_inclusive: u32) -> CommitteeInfo {
        CommitteeInfo::new(
            NumPreshards::P64,
            4,
            8,
            ShardGroup::new(start, end_inclusive),
            Epoch(1),
            VotePower::of(4),
        )
    }

    fn cursor(shard: u32, start_state_version: u64) -> rpc::ShardCursor {
        rpc::ShardCursor {
            shard,
            start_state_version,
        }
    }

    fn held(
        checkpoints: &[(u32, u32)],
        committed_as: Option<(u32, u32)>,
        synced_as: Option<(u32, u32)>,
    ) -> HeldHistory {
        HeldHistory {
            checkpoint_shard_groups: checkpoints.iter().map(|&(s, e)| ShardGroup::new(s, e)).collect(),
            committed_as: committed_as.map(|(s, e)| ShardGroup::new(s, e)),
            synced_as: synced_as.map(|(s, e)| ShardGroup::new(s, e)),
        }
    }

    #[test]
    fn it_rejects_history_through_an_epoch_not_yet_reached() {
        assert!(ensure_epoch_reached(Epoch(4), Epoch(5)).is_ok());
        assert!(ensure_epoch_reached(Epoch(5), Epoch(5)).is_ok());
        let err = ensure_epoch_reached(Epoch(6), Epoch(5)).unwrap_err();
        assert!(err.is_unavailable());
        let err = ensure_epoch_reached(Epoch::max(), Epoch(5)).unwrap_err();
        assert!(err.is_unavailable());
    }

    #[test]
    fn it_rejects_history_whose_values_epoch_gc_may_have_pruned() {
        assert!(ensure_history_values_held(Epoch(4), Epoch(5), 1).is_ok());
        let err = ensure_history_values_held(Epoch(4), Epoch(6), 1).unwrap_err();
        assert!(err.is_unavailable());
        assert!(ensure_history_values_held(Epoch(4), Epoch(6), 2).is_ok());
        assert!(ensure_history_values_held(Epoch::max(), Epoch::max(), 1).is_ok());
    }

    #[test]
    fn a_member_that_committed_the_epoch_holds_its_shards() {
        let history = held(&[(1, 32)], Some((1, 32)), None);
        assert!(history.holds(Shard::from(1u32)));
        assert!(history.holds(Shard::global()));
        assert!(!history.holds(Shard::from(33u32)));
    }

    #[test]
    fn a_synced_node_holds_only_its_own_slice_of_a_checkpoint() {
        // The checkpoint covers 1..=32, but a node joining 17..=32 synced only that slice of it.
        let history = held(&[(1, 32)], None, Some((17, 32)));
        assert!(history.holds(Shard::from(17u32)));
        assert!(!history.holds(Shard::from(1u32)));
    }

    #[test]
    fn membership_without_a_checkpoint_holds_nothing() {
        let history = held(&[], Some((1, 32)), Some((1, 32)));
        assert!(!history.holds(Shard::from(1u32)));
        assert!(!history.holds(Shard::global()));
    }

    #[test]
    fn it_names_the_first_shard_it_does_not_hold() {
        let history = held(&[(1, 32)], Some((1, 32)), None);
        let cursors = [
            ShardCursor {
                shard: Shard::from(1u32),
                start_state_version: 1,
            },
            ShardCursor {
                shard: Shard::from(40u32),
                start_state_version: 1,
            },
        ];
        let err = history.ensure_holds_all(&cursors, Epoch(3)).unwrap_err();
        assert!(err.is_unavailable());
        assert!(err.details().contains("Shard(40)"), "{err}");
    }

    #[test]
    fn it_accepts_ascending_cursors_including_the_global_shard() {
        let validated = ShardCursor::validate_all(vec![cursor(0, 1), cursor(1, 5), cursor(256, 9)]).unwrap();
        assert_eq!(validated, vec![
            ShardCursor {
                shard: Shard::global(),
                start_state_version: 1
            },
            ShardCursor {
                shard: Shard::from_u32(1),
                start_state_version: 5
            },
            ShardCursor {
                shard: Shard::from_u32(256),
                start_state_version: 9
            },
        ]);
    }

    #[test]
    fn it_accepts_a_cursor_for_every_shard_plus_global() {
        let cursors = (0..=NumPreshards::MAX.as_u32())
            .map(|s| cursor(s, 1))
            .collect::<Vec<_>>();
        assert_eq!(cursors.len(), NumPreshards::MAX.num_shards() + 1);
        assert!(ShardCursor::validate_all(cursors).is_ok());
    }

    #[test]
    fn it_rejects_an_empty_cursor_list() {
        assert!(ShardCursor::validate_all(vec![]).is_err());
    }

    #[test]
    fn it_rejects_more_cursors_than_there_are_shards() {
        let cursors = (0..=NumPreshards::MAX.as_u32() + 1)
            .map(|s| cursor(s, 1))
            .collect::<Vec<_>>();
        assert!(ShardCursor::validate_all(cursors).is_err());
    }

    #[test]
    fn it_rejects_an_out_of_range_shard() {
        assert!(ShardCursor::validate_all(vec![cursor(NumPreshards::MAX.as_u32() + 1, 1)]).is_err());
    }

    #[test]
    fn it_rejects_duplicate_and_out_of_order_shards() {
        assert!(ShardCursor::validate_all(vec![cursor(1, 1), cursor(1, 2)]).is_err());
        assert!(ShardCursor::validate_all(vec![cursor(2, 1), cursor(1, 1)]).is_err());
    }

    #[test]
    fn it_rejects_a_zero_start_state_version() {
        assert!(ShardCursor::validate_all(vec![cursor(1, 1), cursor(2, 0)]).is_err());
    }

    #[test]
    fn it_accepts_cursors_for_stored_shards_and_the_global_shard() {
        let cursors = ShardCursor::validate_all(vec![cursor(0, 1), cursor(9, 1), cursor(16, 1)]).unwrap();
        ShardCursor::ensure_all_stored(&cursors, &committee_info(9, 16)).unwrap();
    }

    #[test]
    fn a_warrant_covers_every_shard_its_committee_stores() {
        let authority = TipAuthority::new(Epoch(1), committee_info(9, 16));
        authority.ensure_stores(Shard::global()).unwrap();
        authority.ensure_stores(Shard::from_u32(9)).unwrap();
        authority.ensure_stores(Shard::from_u32(16)).unwrap();
    }

    #[test]
    fn a_warrant_does_not_cover_a_shard_its_epoch_dropped_from_the_group() {
        // A boundary that shrinks the group still leaves it holding the shards the boundary is first
        // noticed on, so every shard is measured against the committee rather than against the
        // boundary having been handled.
        let authority = TipAuthority::new(Epoch(2), committee_info(9, 12));
        authority.ensure_stores(Shard::from_u32(9)).unwrap();
        let err = authority.ensure_stores(Shard::from_u32(13)).unwrap_err();
        assert!(err.details().contains("does not store Shard(13)"), "{err}");
    }

    #[test]
    fn a_warrant_is_stale_once_the_epoch_moves_on() {
        let authority = TipAuthority::new(Epoch(1), committee_info(9, 16));
        assert!(!authority.is_stale_at(Epoch(1)));
        assert!(authority.is_stale_at(Epoch(2)));
    }

    #[test]
    fn a_cursor_is_behind_a_tip_at_or_past_its_resume_point() {
        let cursor = ShardCursor {
            shard: Shard::from_u32(1),
            start_state_version: 5,
        };
        assert!(cursor.is_behind(Some(5)));
        assert!(cursor.is_behind(Some(9)));
        assert!(!cursor.is_behind(Some(4)));
        assert!(!cursor.is_behind(None));
    }

    #[test]
    fn a_cursor_advances_past_a_marker_and_never_moves_back() {
        let mut cursor = ShardCursor {
            shard: Shard::from_u32(1),
            start_state_version: 5,
        };
        cursor.advance_past(9);
        assert_eq!(cursor.start_state_version, 10);
        // A node behind the cursor reports a lower tip.
        cursor.advance_past(3);
        assert_eq!(cursor.start_state_version, 10);
        assert!(!cursor.is_behind(Some(9)));
        assert!(cursor.is_behind(Some(10)));
    }

    #[test]
    fn it_rejects_a_cursor_for_a_shard_this_node_does_not_store() {
        let cursors = ShardCursor::validate_all(vec![cursor(9, 1), cursor(17, 1)]).unwrap();
        let err = ShardCursor::ensure_all_stored(&cursors, &committee_info(9, 16)).unwrap_err();
        assert!(err.details().contains("does not store Shard(17)"), "{err}");
    }

    /// A store that counts the read transactions opened on it. A sync reads each state version it scans in a read
    /// transaction of its own.
    #[derive(Clone)]
    struct CountingStore<T> {
        inner: T,
        reads: Arc<AtomicUsize>,
    }

    impl<T: StateStore> StateStore for CountingStore<T> {
        type Addr = T::Addr;
        type ReadTransaction<'a>
            = T::ReadTransaction<'a>
        where Self: 'a;
        type WriteTransaction<'a>
            = T::WriteTransaction<'a>
        where Self: 'a;

        fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.inner.create_read_tx()
        }

        fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError> {
            self.inner.create_write_tx()
        }

        fn create_durable_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError> {
            self.inner.create_durable_write_tx()
        }
    }

    /// A store holding `num_versions` state versions of the global shard, each bringing up one template.
    fn store_with_history(num_versions: u16) -> (CountingStore<RocksDbStateStore<String>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::<String>::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();
        store
            .with_write_tx(|tx| {
                let mut batch = SubstateUpdateBatch::new(Network::LocalNet, Epoch::zero());
                for seed in 1..=num_versions {
                    batch
                        .with_transition(Shard::global(), u64::from(seed))
                        .push(SubstateTransition::Up {
                            id: template_id(seed),
                            version: SubstateVersion::ZERO,
                            substate_or_hash: SubstateValueOrHash::Value(Box::new(published_template(1).into())),
                        });
                }
                tx.substates_commit_batch(batch)
            })
            .unwrap();
        let store = CountingStore {
            inner: store,
            reads: Arc::new(AtomicUsize::new(0)),
        };
        (store, dir)
    }

    fn dummy_epoch_manager() -> EpochManagerHandle<PeerAddress> {
        let (tx_request, _) = mpsc::channel(1);
        let (tx_events, _) = broadcast::channel(1);
        EpochManagerHandle::new(
            tx_request,
            tx_events.downgrade(),
            Arc::new(AtomicU64::new(0)),
            CommitteeCache::new(),
        )
    }

    /// A task syncing the global shard from its first version through epoch 1.
    fn sync_task<TStore: StateStore>(
        store: TStore,
        sender: mpsc::Sender<Result<rpc::SyncStateResponse, RpcStatus>>,
        value_filters: SubstateValueFilterFlags,
        reads: Arc<Semaphore>,
    ) -> StateSyncTask<TStore> {
        let (events, _) = broadcast::channel(1);
        StateSyncTask::new(
            store,
            sender,
            vec![ShardCursor {
                shard: Shard::global(),
                start_state_version: 1,
            }],
            Some(Epoch(1)),
            ConsensusHandle::for_test(&events),
            dummy_epoch_manager(),
            None,
            false,
            max_updates(),
            value_filters,
            false,
            reads,
        )
    }

    #[tokio::test]
    async fn a_scan_that_streams_nothing_stops_once_the_peer_is_gone() {
        const VERSIONS: u16 = 500;
        let (store, _dir) = store_with_history(VERSIONS);
        let reads = store.reads.clone();
        let (sender, receiver) = mpsc::channel(10);
        // Every substate in the history is a template, so a component filter selects none of them.
        let task = sync_task(
            store,
            sender,
            SubstateValueFilterFlags::COMPONENT,
            Arc::new(Semaphore::new(1)),
        );

        let handle = tokio::spawn(task.run());
        // The scan runs until it first yields this thread, and the peer goes away in the meantime.
        tokio::task::yield_now().await;
        drop(receiver);

        assert!(handle.await.unwrap().is_err());
        let reads = reads.load(Ordering::SeqCst);
        assert!(
            reads < usize::from(VERSIONS / 10),
            "the scan read {reads} state version(s) of {VERSIONS} for a peer that went away"
        );
    }

    #[tokio::test]
    async fn a_scan_waiting_for_a_read_stops_once_the_peer_is_gone() {
        let (store, _dir) = store_with_history(10);
        let reads = store.reads.clone();
        let (sender, receiver) = mpsc::channel(10);
        let task = sync_task(
            store,
            sender,
            SubstateValueFilterFlags::all_substates(),
            Arc::new(Semaphore::new(0)),
        );

        let handle = tokio::spawn(task.run());
        tokio::task::yield_now().await;
        drop(receiver);

        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("the scan kept waiting for a read after its peer went away");
        assert!(result.unwrap().is_err());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_stream_waiting_on_its_peer_holds_no_read() {
        let (store, _dir) = store_with_history(30);
        let shared_reads = Arc::new(Semaphore::new(1));
        // This peer takes nothing, so its stream stalls once the channel is full.
        let (stalled_sender, stalled_receiver) = mpsc::channel(1);
        let stalled = sync_task(
            store.clone(),
            stalled_sender,
            SubstateValueFilterFlags::all_substates(),
            shared_reads.clone(),
        );
        let _stalled = tokio::spawn(stalled.run());
        while stalled_receiver.is_empty() {
            tokio::task::yield_now().await;
        }

        let (sender, mut receiver) = mpsc::channel(10);
        let task = sync_task(store, sender, SubstateValueFilterFlags::COMPONENT, shared_reads);
        let _handle = tokio::spawn(task.run());

        let response = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await
            .expect("a stream waiting on its peer kept every read");
        assert!(matches!(
            response.unwrap().unwrap().response,
            Some(rpc::sync_state_response::Response::Complete(_))
        ));
    }
}
