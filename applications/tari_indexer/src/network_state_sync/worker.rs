//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    convert::Infallible,
    pin::pin,
    sync::Arc,
    time::Duration,
};

use futures::{StreamExt, future::Either, stream::FuturesUnordered};
use log::*;
use ootle_network::Network;
use prost::Message;
use tari_engine_types::{
    published_template::PublishedTemplateMetadata,
    substate::{SubstateId, SubstateValue},
    transaction_receipt::TransactionReceipt,
};
use tari_epoch_manager::{
    EpochManagerEvent,
    EpochManagerReader,
    service::{EpochManagerHandle, NetworkDescription},
};
use tari_indexer_client::event::{IndexerEvent, NewEpochEvent, TransactionEvent, TransactionFinalizedEvent};
use tari_networking::NetworkingHandle;
use tari_ootle_common_types::{Epoch, ShardGroup, StateVersion, ValidatorSetHash, optional::Optional, shard::Shard};
use tari_ootle_p2p::{PeerAddress, TariMessagingSpec, proto::rpc};
use tari_ootle_storage::{
    StorageError,
    consensus_models::{
        EpochCheckpoint,
        SubstateData,
        SubstateUpdateProof,
        SubstateValueFilterFlags,
        VerifiedBlockTip,
    },
};
use tari_rpc_framework::RpcRequestOptions;
use tari_shutdown::ShutdownSignal;
use tari_template_lib_types::{Amount, TemplateAddress, TransactionReceiptAddress, UtxoAddress};
use tokio::{
    sync::{broadcast, watch},
    time,
};
use tokio_util::sync::CancellationToken;

#[cfg(feature = "metrics")]
use crate::{
    exhaust_burn_rate::resolve_exhaust_burn_rate_for_epoch,
    network_state_sync::NetworkStateMetrics,
    store::ReadOnlyStore,
    substate_cache::SubstateCacheMetrics,
    substate_manager::SubstateManager,
};
use crate::{
    network_state_sync::{
        EventFilter,
        committee_client::{ValidatorCommitteeRpcPool, ValidatorRpcSession},
        config::NetworkWideStateSyncConfig,
        consensus_epoch::ConsensusEpoch,
        error::NetworkStateSyncError,
        shard_watermarks::ShardWatermarks,
        stats::SyncStats,
        sync_plan::SyncPlan,
        sync_progress::{SharedSyncProgress, SyncProgress},
        validator_status::ValidatorStatusMonitor,
    },
    notify::Notify,
    storage_sqlite::{
        SqliteIndexerStore,
        SqliteStoreWriteTransaction,
        models::{Key, SubstateCacheInvalidation, UtxoSpent, UtxoUnspent, UtxoUpdateRecord, VerifiedStateRoot},
    },
    store::{
        IndexerStore,
        IndexerStoreReadTransaction,
        IndexerStoreReader,
        IndexerStoreWriteTransaction,
        InsertedEvent,
    },
};

const LOG_TARGET: &str = "tari::indexer::network_state_sync::worker";
/// How often every shard group's committed tip is probed and recorded.
///
/// The latest recorded tip is what a verified read's anchor must be recent against, so this bounds
/// how far that reference trails the chain when no read refreshes it.
const COMMITTED_TIP_PROBE_INTERVAL: Duration = Duration::from_secs(10);
/// The most a peer may stream, in encoded bytes, for one state version before completing it. A
/// version is buffered whole and committed at once, so this bounds the memory a peer can hold to a
/// constant multiple of it: the buffered updates are held decoded, alongside what is derived from
/// them.
const MAX_BUFFERED_VERSION_BYTES: usize = 256 * 1024 * 1024;
// A state version is one block's writes to the shard, which consensus bounds by
// `MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES`. The stream carries each update with its metadata, so the
// buffer leaves twice that.
const _: () =
    assert!(MAX_BUFFERED_VERSION_BYTES >= 2 * tari_ootle_common_types::MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES);

#[derive(Clone)]
pub struct NetworkWideStateSync {
    epoch_manager: EpochManagerHandle<PeerAddress>,
    networking: NetworkingHandle<TariMessagingSpec>,
    store: SqliteIndexerStore,
    stats: SyncStats,
    config: NetworkWideStateSyncConfig,
    notify: Notify<IndexerEvent>,
    transaction_event_notify: Notify<TransactionEvent>,
    validator_status: ValidatorStatusMonitor,
    shard_watermarks: Arc<ShardWatermarks>,
    consensus_epoch: ConsensusEpoch,
    network: Network,
    #[cfg(feature = "metrics")]
    metrics: NetworkStateMetrics,
    #[cfg(feature = "metrics")]
    substate_cache_metrics: SubstateCacheMetrics,
    #[cfg(feature = "metrics")]
    substate_manager: SubstateManager,
}

impl NetworkWideStateSync {
    pub fn new(
        epoch_manager: EpochManagerHandle<PeerAddress>,
        networking: NetworkingHandle<TariMessagingSpec>,
        storage: SqliteIndexerStore,
        config: NetworkWideStateSyncConfig,
        notify: Notify<IndexerEvent>,
        transaction_event_notify: Notify<TransactionEvent>,
        validator_status: ValidatorStatusMonitor,
        shard_watermarks: Arc<ShardWatermarks>,
        consensus_epoch: ConsensusEpoch,
        network: Network,
        #[cfg(feature = "metrics")] metrics: NetworkStateMetrics,
        #[cfg(feature = "metrics")] substate_cache_metrics: SubstateCacheMetrics,
        #[cfg(feature = "metrics")] substate_manager: SubstateManager,
    ) -> Self {
        Self {
            epoch_manager,
            networking,
            store: storage,
            stats: SyncStats::new(),
            config,
            notify,
            transaction_event_notify,
            validator_status,
            shard_watermarks,
            consensus_epoch,
            network,
            #[cfg(feature = "metrics")]
            metrics,
            #[cfg(feature = "metrics")]
            substate_cache_metrics,
            #[cfg(feature = "metrics")]
            substate_manager,
        }
    }

    pub fn spawn(mut self, shutdown_signal: ShutdownSignal) -> tokio::task::JoinHandle<()> {
        let mut epoch_events = self.epoch_manager.subscribe();
        tokio::spawn(async move {
            loop {
                let config = self.config.clone();
                let task = self.start(&mut epoch_events);
                let task = pin!(task);
                match shutdown_signal.clone().select(task).await {
                    Either::Left(_) => {
                        info!(target: LOG_TARGET, "🌍️ Network-wide state sync was shutdown.");
                        break;
                    },
                    Either::Right((Ok(()), _)) => {
                        info!(target: LOG_TARGET, "🌍️ Network-wide state sync completed successfully.");
                    },
                    Either::Right((Err(e), _)) => {
                        error!(target: LOG_TARGET, "⚠️ Network-wide state sync failed: {}", e);
                        // Restart after cooldown
                        time::sleep(config.work_interval).await;
                    },
                }
            }
        })
    }

    async fn start(
        &mut self,
        epoch_events: &mut broadcast::Receiver<EpochManagerEvent>,
    ) -> Result<(), NetworkStateSyncError> {
        self.epoch_manager.wait_for_initial_scanning_to_complete().await?;

        // Publish last-known totals immediately so a freshly restarted indexer reports them before its
        // first sync round completes.
        #[cfg(feature = "metrics")]
        self.update_metrics().await;

        let mut report_interval = time::interval(self.config.work_interval);
        report_interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        report_interval.reset();

        loop {
            let sync_plan = self.initialize_sync_plan().await?;
            let plan_epoch = sync_plan.network_description().epoch();
            let partition = sync_plan
                .network_description()
                .shard_groups_iter()
                .collect::<BTreeSet<_>>();
            self.consensus_epoch.track_groups(partition.iter().copied());
            let (epoch_tx, epoch_rx) = watch::channel(plan_epoch);
            // A plan is drawn against one partition of the shards into groups. An epoch that keeps
            // the partition is handled by each group on its own: it winds its stream down, syncs its
            // checkpoints, re-resolves its committee and reopens from the cursor it holds. Only a
            // change to the partition itself - which shards each group serves - draws a new plan,
            // and then every stream is wound down first; the cursors survive in the persisted
            // progress, so nothing is re-streamed.
            //
            // Wound down, not dropped: a write transaction runs to its commit on a blocking thread
            // whether or not the future awaiting it survives, and a plan drawn from progress read
            // before that commit would carry a cursor the commit has moved past. Once persisted
            // over the committed one, that cursor re-streams a version whose economic totals were
            // already folded in. So every stream is told to stop, and the plan is awaited to the
            // end before the next is read.
            let cancel = CancellationToken::new();
            let mut sync = pin!(self.clone().sync_plan(sync_plan, cancel.clone(), epoch_rx));
            loop {
                tokio::select! {
                    event = epoch_events.recv() => {
                        // Every way out of here winds the plan down first, for the reason above.
                        let outcome: Result<(), NetworkStateSyncError> = match event {
                            Ok(EpochManagerEvent::EpochChanged { epoch, .. }) => {
                                info!(target: LOG_TARGET, "🌍️ Epoch changed to {}.", epoch);
                                self.notify.notify(NewEpochEvent { epoch });
                                match self.epoch_manager.get_network_description().await {
                                    Ok(network_desc) if plan_absorbs_epoch(plan_epoch, &partition, &network_desc) => {
                                        let epoch = network_desc.epoch();
                                        epoch_tx.send_if_modified(|current| {
                                            let moved = *current != epoch;
                                            *current = epoch;
                                            moved
                                        });
                                        continue;
                                    },
                                    Ok(_) => {
                                        info!(target: LOG_TARGET, "🌍️ Re-planning the state sync at epoch {}", epoch);
                                        Ok(())
                                    },
                                    // The plan is re-drawn from a fresh description a work interval
                                    // later rather than every stream being torn down for good.
                                    Err(err) => {
                                        warn!(target: LOG_TARGET, "⚠️ Failed to read the network description at epoch {}: {}. Re-planning the state sync", epoch, err);
                                        Ok(())
                                    },
                                }
                            },
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(target: LOG_TARGET, "⚠️ Missed {n} epoch event(s). Re-planning the state sync");
                                Ok(())
                            },
                            Err(broadcast::error::RecvError::Closed) => Err(NetworkStateSyncError::InvariantError {
                                details: "Epoch manager stopped publishing events".to_string(),
                            }),
                        };
                        cancel.cancel();
                        sync.await?;
                        outcome?;
                        break;
                    },
                    result = &mut sync => {
                        result?;
                        time::sleep(self.config.work_interval).await;
                        break;
                    },
                    _ = report_interval.tick() => {
                        self.stats.log_stats();
                        self.stats.reset();
                        #[cfg(feature = "metrics")]
                        self.update_metrics().await;
                    },
                }
            }
        }
    }

    /// Reads the persisted economic totals and publishes them to the Prometheus gauges. Metrics are
    /// observability only, so a read failure is logged rather than propagated into the sync loop.
    #[cfg(feature = "metrics")]
    async fn update_metrics(&self) {
        match ReadOnlyStore::new(self.store.clone()).get_tari_economics().await {
            Ok(economics) => {
                let current_epoch = self.epoch_manager.get_current_epoch();
                let target_burn_rate_bps =
                    resolve_exhaust_burn_rate_for_epoch(&self.substate_manager, self.network, current_epoch)
                        .await
                        .as_bps();
                self.metrics.update(&economics, target_burn_rate_bps);
            },
            Err(err) => {
                warn!(target: LOG_TARGET, "⚠️ Failed to update network economics metrics: {err}");
            },
        }
    }

    async fn initialize_sync_plan(&self) -> Result<SyncPlan, NetworkStateSyncError> {
        let network_desc = self.epoch_manager.get_network_description().await?;
        let sync_progress = self
            .store
            .with_read_tx(|tx| tx.key_value_get_value::<_, SyncProgress>(Key::SyncProgress))
            .await
            .optional()?
            .unwrap_or_default();

        let mut committee_pools = HashMap::with_capacity(network_desc.num_committees());
        for shard_group in network_desc.shard_groups_iter() {
            let pool = ValidatorCommitteeRpcPool::new(shard_group, self.networking.clone(), self.epoch_manager.clone());
            committee_pools.insert(shard_group, pool);
        }

        Ok(SyncPlan::new(
            network_desc,
            SharedSyncProgress::new(sync_progress),
            committee_pools,
        ))
    }

    /// Follows every shard group's tip until `cancel` is triggered, at which point it returns once
    /// every stream has stopped between messages. `epoch` carries the epoch each group is to serve;
    /// a group answers a change to it on its own. Returns early only on an error that is not one
    /// shard group's alone.
    async fn sync_plan(
        self,
        sync_plan: SyncPlan,
        cancel: CancellationToken,
        epoch: watch::Receiver<Epoch>,
    ) -> Result<(), NetworkStateSyncError> {
        if sync_plan.network_description().epoch.is_zero() {
            info!(target: LOG_TARGET, "🌍️ Current epoch is zero, nothing to sync.");
            cancel.cancelled().await;
            return Ok(());
        }
        info!(target: LOG_TARGET, "🌍️ Starting network-wide state sync...");
        self.follow_state(&sync_plan, &cancel, &epoch).await
    }

    /// Syncs `shard_group`'s checkpoints up to the epoch before `epoch`, from wherever it left off.
    /// Nothing to do once they are recorded, so this is run before every stream the group opens.
    ///
    /// Returns whether the group's checkpoints reach the epoch before `epoch`. They fall short when
    /// the committee has not yet completed that epoch: consensus ends an epoch some time after the
    /// base layer moves past it. They fall short for all of `epoch` when the epoch before it has no
    /// checkpoint at all, as in a network's first epoch or after an epoch consensus never ran. They
    /// also fall short when `cancel` stops the backfill between pages.
    async fn sync_group_checkpoints(
        &self,
        shard_group: ShardGroup,
        pool: &mut ValidatorCommitteeRpcPool,
        epoch: Epoch,
        progress: &SharedSyncProgress,
        cancel: &CancellationToken,
    ) -> Result<bool, NetworkStateSyncError> {
        let Some(prev_epoch) = epoch.checked_sub(Epoch(1)) else {
            return Ok(true);
        };
        let checkpoint_epoch = || async { progress.lock().await.checkpoint_epoch(shard_group) };
        let mut from_epoch = checkpoint_epoch().await.unwrap_or_else(Epoch::zero);
        if from_epoch >= prev_epoch {
            debug!(target: LOG_TARGET, "🌍️ No checkpoints to sync for shard group {shard_group} from epoch {from_epoch}");
            return Ok(true);
        }
        if is_backfill(from_epoch, prev_epoch) {
            match self.sync_head_checkpoint(shard_group, pool, prev_epoch).await {
                Ok(()) => {},
                Err(err) if err.is_peer_fault() => {
                    warn!(target: LOG_TARGET, "⚠️ Syncing the checkpoint of {prev_epoch} for shard group {shard_group} ahead of the backfill failed: {err}");
                },
                Err(err) => return Err(err),
            }
        }
        // Each page is fetched as soon as the one before it is stored, so a wallet walking from an older set waits
        // only as long as the pages take to verify. A cancelled plan stops between pages; progress is recorded per
        // checkpoint, so the next plan resumes where this one stopped.
        loop {
            info!(target: LOG_TARGET, "🌍️ Syncing checkpoints from {from_epoch} for shard group {shard_group}");
            let checkpoints = self
                .fetch_group_checkpoints(shard_group, pool, from_epoch, prev_epoch)
                .await?;

            let level = if checkpoints.is_empty() {
                Level::Debug
            } else {
                Level::Info
            };
            log!(target: LOG_TARGET, level, "🌍️ Found {} checkpoints for shard group {shard_group} from epoch {from_epoch} (prev_epoch {prev_epoch})", checkpoints.len());

            for checkpoint in checkpoints {
                self.store_checkpoint(shard_group, checkpoint, prev_epoch, Some(progress))
                    .await?;
            }
            let reached = checkpoint_epoch().await;
            match reached {
                Some(reached) if reached >= prev_epoch => return Ok(true),
                Some(reached) if reached > from_epoch && !cancel.is_cancelled() => from_epoch = reached,
                _ => return Ok(false),
            }
        }
    }

    /// Syncs `shard_group`'s checkpoint of `prev_epoch` ahead of the backfill that leads up to it.
    ///
    /// A wallet verifies the indexer's reads at the current epoch only through this checkpoint, which hands over to the
    /// current epoch's validator set, so serving it first makes reads verifiable while older epochs are still being
    /// backfilled. The backfill's progress stays where it is.
    async fn sync_head_checkpoint(
        &self,
        shard_group: ShardGroup,
        pool: &mut ValidatorCommitteeRpcPool,
        prev_epoch: Epoch,
    ) -> Result<(), NetworkStateSyncError> {
        // The checkpoint of `prev_epoch` is of that epoch's shard groups, which differ from `shard_group` when the
        // number of committees changed, so it is found by the shard the group starts at.
        let stored = self
            .store
            .with_read_tx(move |tx| tx.epoch_checkpoint_get_all(prev_epoch, Some(shard_group.start()), 1))
            .await?
            .first()
            .is_some_and(|checkpoint| checkpoint.epoch() == prev_epoch);
        if stored {
            return Ok(());
        }
        info!(target: LOG_TARGET, "🌍️ Syncing the checkpoint of {prev_epoch} for shard group {shard_group} ahead of the backfill");
        let checkpoints = self
            .fetch_group_checkpoints(shard_group, pool, prev_epoch, prev_epoch)
            .await?;
        for checkpoint in checkpoints {
            self.store_checkpoint(shard_group, checkpoint, prev_epoch, None).await?;
        }
        Ok(())
    }

    /// `shard_group`'s checkpoints from `from_epoch` to `prev_epoch`, from a committee member whose committed tip
    /// verifies.
    async fn fetch_group_checkpoints(
        &self,
        shard_group: ShardGroup,
        pool: &mut ValidatorCommitteeRpcPool,
        from_epoch: Epoch,
        prev_epoch: Epoch,
    ) -> Result<Vec<EpochCheckpoint>, NetworkStateSyncError> {
        let validator_status = self.validator_status.clone();
        let consensus_epoch = self.consensus_epoch.clone();
        let mut checkpoints = pool
            .try_with_random_members(|mut session| {
                let validator_status = validator_status.clone();
                let consensus_epoch = consensus_epoch.clone();
                async move {
                    // Verify how far this peer has committed before trusting it as a sync source.
                    // `probe` only returns Err for a forged/malformed proof (other failures are
                    // logged internally and return Ok(None)), which disqualifies the peer so
                    // another committee member is tried.
                    match validator_status.probe(&mut session, shard_group).await {
                        Ok(Some(tip)) => consensus_epoch.observe(tip.shard_group, tip.epoch),
                        Ok(None) => {},
                        Err(e) => {
                            return Err(NetworkStateSyncError::InvalidCommitProof {
                                details: format!("shard group {shard_group}: {e}"),
                            });
                        },
                    }
                    fetch_checkpoints(&mut session, shard_group, from_epoch).await
                }
            })
            .await?;
        retain_checkpoints_in_sync_range(&mut checkpoints, from_epoch, prev_epoch);
        Ok(checkpoints)
    }

    /// Verifies `checkpoint` and stores it, advancing `shard_group`'s checkpoint progress to its epoch when `progress`
    /// is given.
    async fn store_checkpoint(
        &self,
        shard_group: ShardGroup,
        checkpoint: EpochCheckpoint,
        prev_epoch: Epoch,
        progress: Option<&SharedSyncProgress>,
    ) -> Result<(), NetworkStateSyncError> {
        info!(target: LOG_TARGET, "🌍️ Validating checkpoint for shard group {shard_group}: {}", checkpoint.header().calculate_hash());

        let checkpoint_shard_group =
            checkpoint
                .checked_shard_group()
                .map_err(|e| NetworkStateSyncError::InvalidCheckpoint {
                    details: format!("Checkpoint for shard group {} is not valid: {}", shard_group, e),
                })?;

        verify_checkpoint(
            self.network,
            &self.epoch_manager,
            &checkpoint,
            checkpoint_shard_group,
            prev_epoch,
        )
        .await?;

        info!(target: LOG_TARGET, "🌍️ Inserting checkpoint for {}, shard group {}", checkpoint.epoch(), checkpoint_shard_group);

        let xtr_exhausted = checkpoint_exhaust_burn(&checkpoint)?;
        self.stats.increment_checkpoints();
        let checkpoint_epoch = checkpoint.epoch();
        // The progress lock is held until the snapshot is written, as `SharedSyncProgress` requires.
        let mut progress = match progress {
            Some(progress) => Some(progress.lock().await),
            None => None,
        };
        let sync_progress_snapshot = progress.as_deref_mut().map(|progress| {
            progress.record_checkpoint(shard_group, checkpoint_epoch);
            progress.clone()
        });
        self.store
            .with_write_tx(move |tx| {
                write_checkpoint(
                    tx,
                    &checkpoint,
                    checkpoint_shard_group,
                    xtr_exhausted,
                    sync_progress_snapshot,
                )
            })
            .await?;
        Ok(())
    }

    /// Follows every shard group's tip at once, each on its own stream, until cancelled. Returns
    /// early only on an error that is not one shard group's alone; a group whose peer fails is
    /// retried on its own without disturbing the others.
    async fn follow_state(
        &self,
        sync_plan: &SyncPlan,
        cancel: &CancellationToken,
        epoch: &watch::Receiver<Epoch>,
    ) -> Result<(), NetworkStateSyncError> {
        let mut committee_pools = sync_plan.committee_pools().iter().collect::<Vec<_>>();
        committee_pools.sort_by_key(|(shard_group, _)| **shard_group);

        // Every committee holds the global shard, so it is claimed by exactly one group: the lowest.
        let mut groups = committee_pools
            .into_iter()
            .enumerate()
            .map(|(i, (shard_group, pool))| {
                self.clone().follow_shard_group(
                    *shard_group,
                    pool.clone(),
                    i == 0,
                    sync_plan.sync_progress().clone(),
                    cancel.clone(),
                    epoch.clone(),
                )
            })
            .collect::<FuturesUnordered<_>>();

        let followed = async {
            while let Some(result) = groups.next().await {
                result?;
            }
            Ok(())
        };
        // The groups wind down on `cancel`; the probe loop is dropped once they have.
        tokio::select! {
            result = followed => result,
            never = self.track_committed_tips(sync_plan) => match never {},
        }
    }

    /// Probes every shard group's committed tip each [`COMMITTED_TIP_PROBE_INTERVAL`], records it in
    /// the verified-root store, and observes its epoch.
    ///
    /// A followed stream only probes when it reopens, which on a quiet network is up to a stream
    /// deadline away and on a busy one may be never. Probing here keeps [`ConsensusEpoch`] following a
    /// committee into a new epoch soon after its end-of-epoch block, and keeps the latest recorded
    /// tip, which a verified read's anchor must be recent against, near the chain's.
    async fn track_committed_tips(&self, sync_plan: &SyncPlan) -> Infallible {
        let mut pools = sync_plan.committee_pools().clone();
        let mut interval = time::interval(COMMITTED_TIP_PROBE_INTERVAL);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            for (shard_group, pool) in &mut pools {
                let shard_group = *shard_group;
                let mut session = match pool.new_session().await {
                    Ok(session) => session,
                    Err(e) => {
                        debug!(target: LOG_TARGET, "No session to probe the committed tip of shard group {shard_group}: {e}");
                        continue;
                    },
                };
                match self.validator_status.probe(&mut session, shard_group).await {
                    Ok(Some(tip)) => {
                        self.consensus_epoch.observe(tip.shard_group, tip.epoch);
                        if let Err(e) = self.persist_verified_tip(tip).await {
                            warn!(target: LOG_TARGET, "⚠️ Failed to record verified state root for shard group {shard_group}: {e}");
                        }
                    },
                    Ok(None) => {},
                    Err(e) => {
                        warn!(target: LOG_TARGET, "⚠️ Validator {} for shard group {} served an INVALID commit proof: {}", session.peer_address(), shard_group, e);
                    },
                }
            }
        }
    }

    /// Keeps one shard group synced: syncs its checkpoints, opens a stream from a committee member,
    /// follows it until it ends, and goes round again, until cancelled.
    ///
    /// A stream that ends because the validator had nothing to send for the deadline is reopened at
    /// once - that is the ordinary end of a followed stream, and the reopen refreshes each shard's
    /// watermark. So is one wound down because the epoch moved: the next round syncs the new
    /// checkpoints and resolves the committee at the new epoch. One closed by a validator that does
    /// not follow, or failed by the peer, waits the work interval first: the former is polling, the
    /// latter wants a different peer.
    async fn follow_shard_group(
        mut self,
        shard_group: ShardGroup,
        mut pool: ValidatorCommitteeRpcPool,
        syncs_global_shard: bool,
        progress: SharedSyncProgress,
        cancel: CancellationToken,
        mut epoch: watch::Receiver<Epoch>,
    ) -> Result<(), NetworkStateSyncError> {
        while !cancel.is_cancelled() {
            let current_epoch = *epoch.borrow_and_update();
            let checkpoints_reached_epoch = match self
                .sync_group_checkpoints(shard_group, &mut pool, current_epoch, &progress, &cancel)
                .await
            {
                Ok(reached) => reached,
                Err(err) => {
                    if !err.is_peer_fault() {
                        return Err(err);
                    }
                    warn!(target: LOG_TARGET, "⚠️ Checkpoint sync for shard group {} failed: {}", shard_group, err);
                    self.pause(shard_group, "checkpoint sync failed", &cancel, &mut epoch)
                        .await;
                    continue;
                },
            };
            if cancel.is_cancelled() {
                break;
            }
            // Wallets verify the validator set of an epoch through the checkpoint that ends the epoch
            // before it, so a missing checkpoint is fetched again after the work interval.
            let checkpoints_due_at =
                (!checkpoints_reached_epoch).then(|| time::Instant::now() + self.config.work_interval);
            let mut session = match pool.new_stream_session().await {
                Ok(s) => s,
                Err(e) => {
                    warn!(target: LOG_TARGET, "⚠️ Failed to create session for shard group {}: {}", shard_group, e);
                    self.pause(shard_group, "no session", &cancel, &mut epoch).await;
                    continue;
                },
            };
            match self.validator_status.probe(&mut session, shard_group).await {
                Ok(Some(verified_tip)) => {
                    self.consensus_epoch
                        .observe(verified_tip.shard_group, verified_tip.epoch);
                    // Record the quorum-signed state root so the read path can skip re-validating
                    // commit proofs for this tip. A failure here must not abort the state sync.
                    if let Err(e) = self.persist_verified_tip(verified_tip).await {
                        warn!(target: LOG_TARGET, "⚠️ Failed to record verified state root for shard group {}: {}", shard_group, e);
                    }
                },
                Ok(None) => {},
                // probe only returns Err for an invalid (forged) commit proof.
                Err(e) => {
                    warn!(target: LOG_TARGET, "⚠️ Validator {} for shard group {} served an INVALID commit proof: {}", session.peer_address(), shard_group, e);
                    self.pause(shard_group, "invalid commit proof", &cancel, &mut epoch)
                        .await;
                    continue;
                },
            }

            // Shard 0 sorts before every preshard, which keeps the cursor list ascending as the
            // responder requires.
            let shards = syncs_global_shard
                .then_some(Shard::global())
                .into_iter()
                .chain(shard_group.shard_iter());

            // A responder that cannot serve these shards - it left the committee, or the epoch this
            // indexer resolved its committees at has moved on - costs this shard group a retry and
            // no more.
            match self
                .sync_shard_group_state(
                    shards,
                    &progress,
                    shard_group,
                    &mut session,
                    &cancel,
                    &mut epoch,
                    checkpoints_due_at,
                )
                .await
            {
                Ok(StreamEnd::Cancelled) => return Ok(()),
                Ok(StreamEnd::TimedOut) => {
                    debug!(target: LOG_TARGET, "🌍️ State sync stream for shard group {shard_group} from {} had nothing to send for the deadline. Reopening", session.peer_address());
                },
                Ok(StreamEnd::EpochAdvanced) => {
                    info!(target: LOG_TARGET, "🌍️ Epoch advanced to {}. Reopening state sync for shard group {shard_group}", *epoch.borrow());
                },
                Ok(StreamEnd::CheckpointsDue) => {
                    info!(target: LOG_TARGET, "🌍️ Shard group {shard_group} has no checkpoint for the epoch before {current_epoch} yet. Syncing checkpoints again");
                },
                Ok(StreamEnd::Final) => {
                    self.pause(
                        shard_group,
                        "the validator closed the stream at its tip and does not follow",
                        &cancel,
                        &mut epoch,
                    )
                    .await;
                },
                Err(err) if err.is_peer_fault() => {
                    warn!(target: LOG_TARGET, "⚠️ State sync for shard group {} from {} failed: {}", shard_group, session.peer_address(), err);
                    self.pause(shard_group, "the peer failed", &cancel, &mut epoch).await;
                },
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    /// Waits the work interval before a shard group tries again, or less if the plan is wound down
    /// or the epoch moves - the new epoch wants its checkpoints synced and its committee resolved
    /// before anything is retried.
    async fn pause(
        &self,
        shard_group: ShardGroup,
        reason: &str,
        cancel: &CancellationToken,
        epoch: &mut watch::Receiver<Epoch>,
    ) {
        let interval = self.config.work_interval;
        debug!(target: LOG_TARGET, "🌍️ Shard group {shard_group}: {reason}. Retrying in {interval:.0?}");
        tokio::select! {
            _ = cancel.cancelled() => {},
            Ok(()) = epoch.changed() => {},
            _ = time::sleep(interval) => {},
        }
    }

    /// Syncs every given shard from `session`, which serves them all over a single stream.
    ///
    /// A shard that has never been synced wants only the current head state rather than its full
    /// history, which is expressed by the `UP_ONLY` filter. Filters apply to the whole request, so
    /// such shards are streamed separately, on a stream that runs to its tip and closes. Every shard
    /// then joins the followed stream: one the head fetch found nothing for follows from version one,
    /// since its first transition is its head state, and it has to arrive while the stream is open
    /// rather than on the reopen after the deadline. The head fetch advances a shard only to the
    /// latest version it delivered, so the followed stream picks up the versions after it, which
    /// hold only substates since destroyed. A shard whose live substates all fall outside the value
    /// filter delivers nothing to the head fetch, so its first sync replays its whole history as ids
    /// and hashes.
    #[expect(clippy::too_many_arguments)]
    async fn sync_shard_group_state(
        &mut self,
        shards: impl Iterator<Item = Shard>,
        progress: &SharedSyncProgress,
        shard_group: ShardGroup,
        session: &mut ValidatorRpcSession,
        cancel: &CancellationToken,
        epoch: &mut watch::Receiver<Epoch>,
        checkpoints_due_at: Option<time::Instant>,
    ) -> Result<StreamEnd, NetworkStateSyncError> {
        let value_filters = SubstateValueFilterFlags::UTXO |
            SubstateValueFilterFlags::VALIDATOR_FEE_POOL |
            SubstateValueFilterFlags::CLAIMED_OUTPUT_TOMBSTONE |
            SubstateValueFilterFlags::TRANSACTION_RECEIPT |
            SubstateValueFilterFlags::TEMPLATE_METADATA;

        let shards = shards.collect::<Vec<_>>();
        let from_scratch = cursors_for(&shards, &*progress.lock().await)
            .into_iter()
            .filter(|cursor| cursor.start_state_version == 1)
            .collect::<Vec<_>>();
        if !from_scratch.is_empty() {
            info!(
                target: LOG_TARGET,
                "🌍️ Syncing {} shard(s) in shard group {shard_group} from scratch. Only fetching the head state.",
                from_scratch.len()
            );
            let end = self
                .stream_shard_state(
                    from_scratch,
                    value_filters | SubstateValueFilterFlags::UP_ONLY,
                    false,
                    progress,
                    shard_group,
                    session,
                    cancel,
                    epoch,
                    checkpoints_due_at,
                )
                .await?;
            if matches!(
                end,
                StreamEnd::Cancelled | StreamEnd::EpochAdvanced | StreamEnd::CheckpointsDue
            ) {
                return Ok(end);
            }
        }

        let cursors = cursors_for(&shards, &*progress.lock().await);
        if cursors.is_empty() {
            warn!(target: LOG_TARGET, "⚠️ Every shard in shard group {shard_group} is recorded at the last representable state version. Nothing to follow");
            return Ok(StreamEnd::Final);
        }
        // ALL_HASHES adds an id and a version for every substate outside the value filter, which is
        // what lets the substate cache tell a superseded or destroyed entry from a current one. It
        // is pointless on the from-scratch stream: those shards have no cached entries to retire.
        self.stream_shard_state(
            cursors,
            value_filters | SubstateValueFilterFlags::ALL_HASHES,
            true,
            progress,
            shard_group,
            session,
            cancel,
            epoch,
            checkpoints_due_at,
        )
        .await
    }

    /// Records a committee-validated tip into the verified-root store, after a fail-open epoch
    /// continuity check: the committee's quorum-signed `epoch_hash` must match the epoch hash the
    /// indexer independently derives from the base layer. A mismatch is logged loudly but does not
    /// stop the tip being recorded - the read path is sound regardless, so this is anomaly detection
    /// (forged checkpoint / L1 reorg), not a gate.
    async fn persist_verified_tip(&self, tip: VerifiedBlockTip) -> Result<(), NetworkStateSyncError> {
        match self.epoch_manager.get_epoch_hash(tip.epoch).await {
            Ok(expected) if expected != tip.epoch_hash => {
                error!(
                    target: LOG_TARGET,
                    "⚠️ Epoch continuity mismatch for {} epoch {}: committee epoch_hash {} != base-layer-derived {}. Recording tip anyway.",
                    tip.shard_group, tip.epoch, tip.epoch_hash, expected
                );
            },
            Ok(_) => {},
            Err(e) => {
                // Not yet resolvable (e.g. the epoch just changed); skip the check and retry next round.
                debug!(target: LOG_TARGET, "Epoch hash for epoch {} unavailable for continuity check: {e}", tip.epoch);
            },
        }

        let root = VerifiedStateRoot::from_verified_tip(&tip);
        self.store
            .with_write_tx(move |tx| tx.upsert_verified_state_root(&root))
            .await?;
        Ok(())
    }

    /// Consumes a single `sync_state` stream covering `cursors`, following the responder's tip if
    /// `follow` is set.
    ///
    /// The responder streams each shard's updates contiguously and closes it off with a completion
    /// marker, so progress is recorded per shard as the stream advances - an interrupted stream keeps
    /// everything already committed and simply resumes from the recorded cursors when reopened. A
    /// followed stream keeps going past the tip, closing off each burst of a shard's new versions
    /// with a further marker; it ends when the responder has had nothing to send for the deadline,
    /// or can no longer serve it.
    ///
    /// Cancellation and an epoch change are honoured between messages, so a version being committed
    /// when either arrives is committed in full and the recorded progress reflects it.
    #[expect(clippy::too_many_lines, clippy::too_many_arguments)]
    async fn stream_shard_state(
        &mut self,
        cursors: Vec<rpc::ShardCursor>,
        value_filters: SubstateValueFilterFlags,
        follow: bool,
        progress: &SharedSyncProgress,
        shard_group: ShardGroup,
        session: &mut ValidatorRpcSession,
        cancel: &CancellationToken,
        epoch: &mut watch::Receiver<Epoch>,
        checkpoints_due_at: Option<time::Instant>,
    ) -> Result<StreamEnd, NetworkStateSyncError> {
        let mut order = StreamOrder::new(
            &cursors,
            value_filters.include_filtered_hashes() && !value_filters.is_up_only(),
        );

        info!(
            target: LOG_TARGET,
            "🌍️ Starting state sync for {} shard(s) in shard group {shard_group} from peer {} (follow: {follow})",
            cursors.len(),
            session.peer_address()
        );

        let options = RpcRequestOptions::new()
            .with_deadline(self.config.stream_deadline)
            .with_keepalive_interval(self.config.keepalive_interval);
        let mut stream = session
            .sync_state_with_options(
                rpc::SyncStateRequest {
                    cursors,
                    // Sync to latest epoch
                    until_epoch: None,
                    value_filters: value_filters.bits(),
                    follow,
                    include_version_proofs: false,
                },
                options,
            )
            .await?;

        // A keepalive says the responder is there and has nothing to send: for every shard it has
        // closed off on this stream, that is the claim a further marker would make, so each one
        // re-stamps those shards' watermarks. A shard not yet closed off is still being caught up
        // and is not level, keepalive or not.
        let mut keepalives = stream.keepalives();
        let mut keepalives_open = true;
        let mut level_shards = HashSet::new();

        // Buffers accumulate a single (shard, state version) at a time: the responder splits an
        // oversized version into chunks flagged `has_more`, and the last chunk flushes them.
        let mut num_updates = 0usize;
        let mut version_buf = VersionBuf::default();

        loop {
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(StreamEnd::Cancelled),
                Ok(()) = epoch.changed() => return Ok(StreamEnd::EpochAdvanced),
                () = time::sleep_until(checkpoints_due_at.unwrap_or_else(time::Instant::now)), if checkpoints_due_at.is_some() => {
                    return Ok(StreamEnd::CheckpointsDue);
                },
                next = stream.next() => match next {
                    Some(result) => result,
                    None => break,
                },
                changed = keepalives.changed(), if keepalives_open => {
                    match changed {
                        Ok(()) => {
                            for shard in &level_shards {
                                self.shard_watermarks.refresh(*shard);
                            }
                        },
                        Err(_) => keepalives_open = false,
                    }
                    continue;
                },
            };
            let msg = match result {
                Ok(msg) => msg,
                // A followed stream with nothing to send for the deadline is simply abandoned by the
                // responder, which the client reports as a timeout.
                Err(status) if follow && status.as_status_code().is_timeout() => {
                    debug!(target: LOG_TARGET, "🌍️ State sync stream for shard group {shard_group} timed out: {status}");
                    return Ok(StreamEnd::TimedOut);
                },
                Err(status) => return Err(status.into()),
            };
            let batch = match msg.response {
                Some(rpc::sync_state_response::Response::Batch(batch)) => batch,
                Some(rpc::sync_state_response::Response::VersionProof(_)) => {
                    return Err(NetworkStateSyncError::InvalidStateUpdate {
                        details: "Received a state version proof that was not requested".to_string(),
                    });
                },
                Some(rpc::sync_state_response::Response::Complete(complete)) => {
                    let shard = Shard::from(complete.shard);
                    let synced_to = StateVersion::new(complete.synced_to_version);
                    let msg_epoch =
                        complete
                            .epoch
                            .map(Epoch::from)
                            .ok_or_else(|| NetworkStateSyncError::InvalidStateUpdate {
                                details: "Received sync completion without epoch".to_string(),
                            })?;
                    // Recorded progress only ever advances with a committed batch, never to the
                    // version a marker claims: a claim past what was streamed would skip every
                    // version in between for good.
                    let level_at = order
                        .accept_marker(shard, synced_to)
                        .map_err(|details| NetworkStateSyncError::InvalidStateUpdate { details })?;
                    // The completion marker is the only point at which this shard is known to be
                    // level with the committee, which is what the substate cache needs: mid-stream
                    // the indexer holds every transition up to some version while the chain is
                    // arbitrarily far ahead of it. Confirmed even when the watermark did not move -
                    // a quiet shard is still one this indexer is keeping up with.
                    if let Some(level_at) = level_at {
                        self.shard_watermarks.confirm(shard, level_at);
                        level_shards.insert(shard);
                    }
                    debug!(target: LOG_TARGET, "🌍️ Completed state sync for shard {shard} in shard group {shard_group} at epoch {msg_epoch} (peer synced to v{synced_to})");
                    if complete.is_final {
                        return Ok(StreamEnd::Final);
                    }
                    continue;
                },
                None => {
                    return Err(NetworkStateSyncError::InvalidStateUpdate {
                        details: "Received sync state response with no variant set".to_string(),
                    });
                },
            };

            let shard = Shard::from(batch.shard);
            let state_version = StateVersion::new(batch.state_version);
            let msg_epoch = batch
                .epoch
                .map(Epoch::from)
                .ok_or_else(|| NetworkStateSyncError::InvalidStateUpdate {
                    details: "Received state update without epoch".to_string(),
                })?;
            order
                .accept_batch(
                    shard,
                    state_version,
                    batch.has_more,
                    batch.updates.len(),
                    batch.encoded_len(),
                )
                .map_err(|details| NetworkStateSyncError::InvalidStateUpdate { details })?;

            num_updates += batch.updates.len();
            for update in batch.updates {
                let update =
                    SubstateUpdateProof::try_from(update).map_err(|e| NetworkStateSyncError::InvalidStateUpdate {
                        details: format!("Failed to convert substate update: {}", e),
                    })?;

                version_buf.extend_from_substate_update(shard, state_version, update, value_filters);
            }
            if batch.has_more {
                debug!(target: LOG_TARGET, "🌍️ more updates for shard {shard} (epoch: {msg_epoch}, state version: {state_version})");
                continue;
            }

            debug!(target: LOG_TARGET, "🌍️ Received {num_updates} updates for shard {shard} (epoch: {msg_epoch}, state version: {state_version})");

            self.stats.increase_state_updates(std::mem::take(&mut num_updates));
            // The stream flushes (has_more == false) once per state version, so each commit must fold
            // only that version's delta.
            let version = std::mem::take(&mut version_buf);

            let mut progress = progress.lock().await;
            progress.record_state_version(shard, state_version, msg_epoch);
            let sync_progress_snapshot = progress.clone();

            let event_filters = self.config.event_filters.clone();
            let watched_templates = self.config.watched_templates.clone();

            let committed = self
                .store
                .clone()
                .with_write_tx(move |tx| {
                    version.commit(
                        tx,
                        shard,
                        state_version,
                        msg_epoch,
                        sync_progress_snapshot,
                        &event_filters,
                        &watched_templates,
                    )
                })
                .await?;
            drop(progress);
            self.stats.increase_events(committed.num_receipt_events);
            if committed.retired_cache_entries > 0 {
                debug!(target: LOG_TARGET, "Retired {} cached substates for shard {shard} at state version {state_version}", committed.retired_cache_entries);
                #[cfg(feature = "metrics")]
                self.substate_cache_metrics
                    .add_invalidations(committed.retired_cache_entries);
            }
            for address in &committed.skipped_receipts {
                warn!(target: LOG_TARGET, "⚠️ Peer {} streamed receipt {address} at v{state_version} of shard {shard}, which is already indexed. Kept the indexed receipt", session.peer_address());
            }
            for address in &committed.skipped_utxos {
                warn!(target: LOG_TARGET, "⚠️ Peer {} streamed unspent output {address} at v{state_version} of shard {shard}, whose nonce and tag an indexed unspent output already holds. Kept the indexed output", session.peer_address());
            }
            for address in &committed.skipped_templates {
                warn!(target: LOG_TARGET, "⚠️ Peer {} streamed template {address} at v{state_version} of shard {shard}, which is already catalogued. Kept the catalogued entry", session.peer_address());
            }

            for finalized in committed.finalized {
                self.notify.notify(finalized);
            }
            for inserted in committed.inserted_events {
                self.transaction_event_notify.notify(TransactionEvent {
                    id: inserted.id,
                    transaction_id: inserted.transaction_id,
                    event: inserted.event,
                    encoded: inserted.encoded,
                });
            }
        }

        // A followed stream is only ever ended by the responder for want of its warrant, which it
        // reports, or of consensus. Ending silently is the peer's failing either way.
        Err(NetworkStateSyncError::InvalidStateUpdate {
            details: if follow {
                format!("Followed state sync stream for shard group {shard_group} was closed by the responder")
            } else {
                format!("State sync stream for shard group {shard_group} ended without a final completion marker")
            },
        })
    }
}

/// Whether a plan drawn at `plan_epoch` against `partition` carries on into the epoch `network_desc`
/// describes, with each shard group reopening on its own.
///
/// It does so only while the partition of shards into groups is unchanged: a group loop serves one
/// set of shards for its lifetime. A plan drawn at epoch zero has no group loops at all - it waits
/// for the network to start - so the first real epoch always draws a new plan, whatever partition
/// epoch zero reported.
fn plan_absorbs_epoch(plan_epoch: Epoch, partition: &BTreeSet<ShardGroup>, network_desc: &NetworkDescription) -> bool {
    !plan_epoch.is_zero() && network_desc.shard_groups_iter().collect::<BTreeSet<_>>() == *partition
}

/// How a `sync_state` stream ended without failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamEnd {
    /// The responder closed it with a final completion marker: it streamed to its tip and does not
    /// follow.
    Final,
    /// A followed stream was abandoned by the responder after it had nothing to send for the
    /// deadline.
    TimedOut,
    /// The epoch moved while the stream was open. The group syncs the new checkpoints and reopens
    /// from a committee member at the new epoch.
    EpochAdvanced,
    /// The group's checkpoint for the epoch before the current one was missing when the stream
    /// opened, and the work interval has passed. The group syncs checkpoints again and reopens.
    CheckpointsDue,
    /// The plan is being wound down.
    Cancelled,
}

/// A cursor per shard resuming after the version recorded for it, in the order of `shards`. A shard
/// never synced resumes from version one. A shard recorded at the last representable version has no
/// version to resume from, and gets no cursor.
fn cursors_for(shards: &[Shard], progress: &SyncProgress) -> Vec<rpc::ShardCursor> {
    shards
        .iter()
        .filter_map(|&shard| {
            let start_state_version = match progress.last_state_version(shard) {
                Some(version) => {
                    let Some(next) = version.as_u64().checked_add(1) else {
                        warn!(target: LOG_TARGET, "⚠️ Shard {shard} is recorded at v{version}, the last representable state version, and is no longer synced. Resync this indexer from an empty data directory");
                        return None;
                    };
                    next
                },
                None => 1,
            };
            Some(rpc::ShardCursor {
                shard: shard.as_u32(),
                start_state_version,
            })
        })
        .collect()
}

/// Enforces the ordering a `sync_state` stream promises across the shards it carries: a shard's
/// versions strictly advance, and a version split across chunks is delivered whole before anything
/// else.
///
/// The consumer relies on both. Its buffers hold one `(shard, state version)` at a time, so a shard
/// interleaved mid-version would mix two shards' updates into one commit; and because the running
/// economic totals are read-modify-write, re-applying a version already committed would double-count
/// it. A completion marker closes off what was streamed so far for a shard and a followed stream
/// carries more for it after, so a marker does not end a shard.
///
/// A version split across chunks is also held to [`MAX_BUFFERED_VERSION_BYTES`], since it is
/// buffered whole until the chunk that flushes it.
struct StreamOrder {
    /// Highest version committed per requested shard, seeded from the cursor so a responder cannot
    /// replay versions the caller already holds.
    committed_versions: HashMap<Shard, StateVersion>,
    /// Set while a version is split across chunks, with the encoded bytes buffered for it, until the
    /// chunk that flushes it.
    pending_chunk: Option<(Shard, StateVersion, usize)>,
    /// Whether the stream carries every transition of its shards. Every committed state version of
    /// a shard holds at least one transition, so such a stream delivers each shard's versions
    /// contiguously from its cursor, and a marker claims no version beyond the last one delivered.
    complete: bool,
}

impl StreamOrder {
    fn new(cursors: &[rpc::ShardCursor], complete: bool) -> Self {
        Self {
            complete,
            committed_versions: cursors
                .iter()
                .map(|c| {
                    (
                        Shard::from(c.shard),
                        StateVersion::new(c.start_state_version.saturating_sub(1)),
                    )
                })
                .collect(),
            pending_chunk: None,
        }
    }

    fn accept_batch(
        &mut self,
        shard: Shard,
        state_version: StateVersion,
        has_more: bool,
        num_updates: usize,
        encoded_bytes: usize,
    ) -> Result<(), String> {
        let Some(committed_version) = self.committed_versions.get(&shard).copied() else {
            return Err(format!("Received batch for unrequested shard {shard}"));
        };
        // A responder splits a version into chunks of its updates, so every chunk carries one.
        if num_updates == 0 {
            return Err(format!("Received v{state_version} for shard {shard} with no updates"));
        }
        if state_version <= committed_version {
            return Err(format!(
                "Received v{state_version} for shard {shard}, which is not ahead of the committed v{committed_version}"
            ));
        }
        let buffered_bytes = match self.pending_chunk {
            Some((pending_shard, pending_version, bytes))
                if (pending_shard, pending_version) == (shard, state_version) =>
            {
                bytes
            },
            Some((pending_shard, pending_version, _)) => {
                return Err(format!(
                    "Received v{state_version} of shard {shard} while v{pending_version} of shard {pending_shard} is \
                     still incomplete"
                ));
            },
            None => {
                if self.complete && committed_version.as_u64().checked_add(1) != Some(state_version.as_u64()) {
                    return Err(format!(
                        "Received v{state_version} for shard {shard}, skipping the versions after the committed \
                         v{committed_version}"
                    ));
                }
                0
            },
        }
        .saturating_add(encoded_bytes);
        if buffered_bytes > MAX_BUFFERED_VERSION_BYTES {
            return Err(format!(
                "Received more than {MAX_BUFFERED_VERSION_BYTES} bytes for v{state_version} of shard {shard} without \
                 its completion"
            ));
        }

        if has_more {
            self.pending_chunk = Some((shard, state_version, buffered_bytes));
        } else {
            self.pending_chunk = None;
            self.committed_versions.insert(shard, state_version);
        }
        Ok(())
    }

    /// Accepts a completion marker, returning the version the shard is now level with the committee
    /// at: the last version delivered for it. A stream that leaves out transitions establishes no
    /// such version.
    fn accept_marker(&mut self, shard: Shard, synced_to: StateVersion) -> Result<Option<StateVersion>, String> {
        let Some(committed_version) = self.committed_versions.get(&shard).copied() else {
            return Err(format!("Received completion marker for unrequested shard {shard}"));
        };
        if let Some((pending_shard, pending_version, _)) = self.pending_chunk {
            return Err(format!(
                "Received completion marker for shard {shard} while v{pending_version} of shard {pending_shard} is \
                 still incomplete"
            ));
        }
        if !self.complete {
            return Ok(None);
        }
        if synced_to > committed_version {
            return Err(format!(
                "Received completion marker for shard {shard} at v{synced_to}, past the last version streamed, \
                 v{committed_version}"
            ));
        }
        Ok(Some(committed_version))
    }
}

/// Adds `amount` to the running total persisted under `key`.
fn add_to_total(tx: &mut SqliteStoreWriteTransaction<'_>, key: Key, amount: Amount) -> Result<(), StorageError> {
    use crate::store::IndexerStoreWriteTransaction;

    let total = tx
        .key_value_get_value::<_, Amount>(&key)
        .optional()?
        .unwrap_or_else(Amount::zero);
    let new_total = total
        .checked_add(amount)
        .ok_or_else(|| StorageError::DataInconsistency {
            details: format!(
                "Adding {amount} to the {} total of {total} overflows it",
                key.as_key_str()
            ),
        })?;
    tx.key_value_set(key, new_total)
}

/// The exhaust burn `checkpoint` reports for its shard group and epoch.
///
/// Every Tari amount is a `u64` of microtari and the whole supply fits in one, so no shard group can
/// burn more in an epoch. A checkpoint claiming more is rejected: bounding each one keeps the running
/// total, summed over every epoch and shard group, from approaching `Amount::MAX`.
fn checkpoint_exhaust_burn(checkpoint: &EpochCheckpoint) -> Result<Amount, NetworkStateSyncError> {
    let burn = checkpoint.header().accumulated_data().total_exhaust_burn;
    u64::try_from(burn)
        .map(Amount::from)
        .map_err(|_| NetworkStateSyncError::InvalidCheckpoint {
            details: format!(
                "Checkpoint for epoch {} claims an exhaust burn of {burn}, more than the total supply",
                checkpoint.epoch()
            ),
        })
}

/// Requests `shard_group`'s checkpoints from `from_epoch` on from `session`'s committee member.
async fn fetch_checkpoints(
    session: &mut ValidatorRpcSession,
    shard_group: ShardGroup,
    from_epoch: Epoch,
) -> Result<Vec<EpochCheckpoint>, NetworkStateSyncError> {
    let resp = session
        .get_checkpoints(rpc::GetCheckpointsRequest {
            from_epoch: Some(from_epoch.into()),
            num_to_return: 100,
            shard_group: None,
        })
        .await?;

    debug!(target: LOG_TARGET, "🌍️ Received {} checkpoints for shard group {} from peer {}", resp.checkpoints.len(), shard_group, session.peer_address());

    resp.checkpoints
        .into_iter()
        .map(|cp| {
            EpochCheckpoint::try_from(cp).map_err(|e| NetworkStateSyncError::InvalidCheckpoint {
                details: format!("Failed to convert checkpoint for shard group {}: {}", shard_group, e),
            })
        })
        .collect()
}

/// Verifies `checkpoint` against `epoch_manager`'s record of its epoch, before it is stored and served.
///
/// In an epoch the epoch manager has a record of, the checkpoint must be of one of that epoch's shard groups, and its
/// quorum certificate is checked against that shard group's committee. `shard_group` is the checkpoint's own, which
/// can differ from the shard group being synced because shard groups change with the number of committees. Only a
/// checkpoint of an epoch before the epoch manager's records is checked for form alone. `prev_epoch` is always
/// recorded.
///
/// The next epoch's validator set the checkpoint names, which the committee ratified, must be the one this indexer
/// assigned: an indexer that disagrees would serve committees the network does not run.
async fn verify_checkpoint<TEpochManager: EpochManagerReader>(
    network: Network,
    epoch_manager: &TEpochManager,
    checkpoint: &EpochCheckpoint,
    shard_group: ShardGroup,
    prev_epoch: Epoch,
) -> Result<(), NetworkStateSyncError> {
    let epoch = checkpoint.epoch();
    let recorded = epoch == prev_epoch || epoch_manager.get_epoch_hash(epoch).await.optional()?.is_some();
    if !recorded {
        debug!(target: LOG_TARGET, "🌍️ {epoch} is before this indexer's records: checking its checkpoint for form only");
        return checkpoint
            .validate_well_formed()
            .map_err(|e| NetworkStateSyncError::InvalidCheckpoint {
                details: format!("Checkpoint for {epoch}, shard group {shard_group} is not well formed: {e}"),
            });
    }
    let Some(committee) = epoch_manager
        .get_committee_by_shard_group(epoch, shard_group)
        .await
        .optional()?
    else {
        return Err(NetworkStateSyncError::InvalidCheckpoint {
            details: format!(
                "Checkpoint for {epoch} names shard group {shard_group}, which is not one of that epoch's shard \
                 groups as this indexer assigned them. If every committee member is refused this way, this indexer's \
                 committee assignment for {epoch} differs from the network's; check its epoch oracle and committee \
                 configuration."
            ),
        });
    };
    checkpoint
        .validate(network, epoch, committee.quorum_threshold(), &*committee)
        .map_err(|e| NetworkStateSyncError::InvalidCheckpoint {
            details: format!("Checkpoint for {epoch}, shard group {shard_group} is not valid: {e}"),
        })?;

    let Some(committed) = checkpoint.next_epoch_validator_set_hash() else {
        return Ok(());
    };
    let next_epoch = epoch + Epoch(1);
    let local = epoch_manager.get_validator_set_hash(next_epoch).await?;
    check_validator_set(next_epoch, local, committed)
}

fn check_validator_set(
    epoch: Epoch,
    local: ValidatorSetHash,
    committed: ValidatorSetHash,
) -> Result<(), NetworkStateSyncError> {
    if local != committed {
        return Err(NetworkStateSyncError::ValidatorSetDiverged {
            epoch,
            local,
            committed,
        });
    }
    Ok(())
}

/// Stores `checkpoint` of `shard_group`, adding `xtr_exhausted` to the accumulated exhaust burn the first time it is
/// stored, and `progress` when given.
fn write_checkpoint(
    tx: &mut SqliteStoreWriteTransaction<'_>,
    checkpoint: &EpochCheckpoint,
    shard_group: ShardGroup,
    xtr_exhausted: Amount,
    progress: Option<SyncProgress>,
) -> Result<(), StorageError> {
    if !tx.epoch_checkpoint_exists(shard_group, checkpoint.epoch())? {
        tx.insert_or_ignore_epoch_checkpoint(checkpoint)?;
        add_to_total(tx, Key::TariAccumulatedExhaustBurn, xtr_exhausted)?;
    }
    if let Some(progress) = progress {
        tx.key_value_set(Key::SyncProgress, progress)?;
    }
    Ok(())
}

/// Whether a checkpoint sync from `from_epoch` backfills epochs before `prev_epoch`, whose checkpoint is then synced
/// first.
fn is_backfill(from_epoch: Epoch, prev_epoch: Epoch) -> bool {
    from_epoch.as_u64() + 1 < prev_epoch.as_u64()
}

/// Keeps the checkpoints a sync from `from_epoch` can use: those from `from_epoch` up to `prev_epoch`.
/// Later ones are for an epoch that has not ended for this indexer, and earlier ones were already
/// synced.
fn retain_checkpoints_in_sync_range(checkpoints: &mut Vec<EpochCheckpoint>, from_epoch: Epoch, prev_epoch: Epoch) {
    checkpoints.retain(|checkpoint| (from_epoch..=prev_epoch).contains(&checkpoint.epoch()));
}

fn process_watched_substate_events(
    tx: &mut SqliteStoreWriteTransaction<'_>,
    events: &[InsertedEvent],
    watched_templates: &HashSet<TemplateAddress>,
) -> Result<(), StorageError> {
    use crate::store::IndexerStoreWriteTransaction;

    for inserted in events {
        let event = &inserted.event;
        match event.topic() {
            "std.component.created" => {
                if watched_templates.contains(event.template_address()) &&
                    let Some(substate_id) = event.substate_id()
                {
                    debug!(
                        target: LOG_TARGET,
                        "📌 Watched component created: {} (template: {})",
                        substate_id,
                        event.template_address()
                    );
                    tx.insert_watched_substate(substate_id, event.template_address())?;
                }
            },
            "std.component.template_update" => {
                if let Some(substate_id) = event.substate_id() {
                    let prev_template = event
                        .payload()
                        .get_as::<TemplateAddress>("prev_template")
                        .ok()
                        .flatten();

                    let prev_was_watched = prev_template.as_ref().is_some_and(|t| watched_templates.contains(t));
                    let new_is_watched = watched_templates.contains(event.template_address());

                    if prev_was_watched && !new_is_watched {
                        debug!(
                            target: LOG_TARGET,
                            "📌 Watched component removed (template update): {}",
                            substate_id
                        );
                        tx.delete_watched_substate(substate_id)?;
                    } else if new_is_watched {
                        debug!(
                            target: LOG_TARGET,
                            "📌 Watched component updated: {} (template: {})",
                            substate_id,
                            event.template_address()
                        );
                        tx.insert_watched_substate(substate_id, event.template_address())?;
                    } else {
                        // N/A
                    }
                }
            },
            _ => {},
        }
    }
    Ok(())
}

/// One streamed state version, buffered whole until the chunk that completes it and then committed
/// in a single write transaction.
#[derive(Default)]
struct VersionBuf {
    invalidations: Vec<SubstateCacheInvalidation>,
    utxos: Vec<UtxoUpdateRecord>,
    transactions: Vec<(TransactionReceiptAddress, TransactionReceipt)>,
    validator_fee_pools: Vec<SubstateData>,
    template_catalogue: Vec<(TemplateAddress, PublishedTemplateMetadata)>,
    xtr_claimed: Amount,
}

/// What committing a [`VersionBuf`] wrote, for the caller to announce once the write transaction
/// has committed.
struct CommittedVersion {
    retired_cache_entries: usize,
    /// Events carried by the inserted receipts, before the event filters.
    num_receipt_events: usize,
    /// One per receipt the commit inserted.
    finalized: Vec<TransactionFinalizedEvent>,
    inserted_events: Vec<InsertedEvent>,
    /// Receipts left out because their address already held one.
    skipped_receipts: Vec<TransactionReceiptAddress>,
    /// Unspent outputs left out because an unspent output with their nonce and tag is indexed.
    skipped_utxos: Vec<UtxoAddress>,
    /// Templates left out because the catalogue already held them.
    skipped_templates: Vec<TemplateAddress>,
}

impl VersionBuf {
    /// Writes the version, with `sync_progress` recording it, and folds what it inserted into the
    /// running totals.
    ///
    /// The receipt-sourced totals are taken from the receipts actually inserted, so a receipt whose
    /// address is already indexed is neither counted again nor announced.
    #[expect(clippy::too_many_arguments)]
    fn commit(
        self,
        tx: &mut SqliteStoreWriteTransaction<'_>,
        shard: Shard,
        state_version: StateVersion,
        epoch: Epoch,
        sync_progress: SyncProgress,
        event_filters: &[EventFilter],
        watched_templates: &HashSet<TemplateAddress>,
    ) -> Result<CommittedVersion, StorageError> {
        // Must commit with the watermark below: the substate cache serves an entry on the argument
        // that it holds every transition up to that watermark, which a reader seeing one of the two
        // without the other would break.
        let retired_cache_entries = tx.substate_cache_invalidate(self.invalidations, state_version)?;
        debug!(target: LOG_TARGET, "✅ Committing {} UTXOs for shard {shard} (epoch: {epoch})", self.utxos.len());
        let skipped_utxos = tx.batch_insert_utxo_updates(epoch, self.utxos)?;
        for substate_data in self.validator_fee_pools {
            tx.upsert_substate(&substate_data)?;
        }
        debug!(target: LOG_TARGET, "✅ Committing {} transactions for shard {shard} (epoch: {epoch})", self.transactions.len());
        let inserted = tx.batch_insert_transaction_receipts(self.transactions, event_filters)?;
        if !watched_templates.is_empty() {
            process_watched_substate_events(tx, &inserted.events, watched_templates)?;
        }

        let mut skipped_templates = Vec::new();
        if !self.template_catalogue.is_empty() {
            debug!(target: LOG_TARGET, "✅ Inserting {} template catalogue entries for shard {shard} (epoch: {epoch})", self.template_catalogue.len());
            for (template_addr, metadata) in self.template_catalogue {
                if !tx.insert_or_ignore_template_catalogue(&template_addr, &metadata)? {
                    skipped_templates.push(template_addr);
                }
            }
        }

        // Each receipt's realized-share pair: what the payer spent and the burn taken out of it, so
        // `burn / paid` recovers the share independent of the header-sourced burn total.
        let mut xtr_fees = Amount::zero();
        let mut xtr_receipt_burn = Amount::zero();
        for receipt in &inserted.receipts {
            xtr_fees += Amount::from(receipt.fee_receipt.total_fees_paid());
            xtr_receipt_burn += Amount::from(receipt.fee_receipt.exhaust_burn());
        }

        tx.key_value_set(Key::SyncProgress, sync_progress)?;
        add_to_total(tx, Key::TariAccumulatedClaimed, self.xtr_claimed)?;
        add_to_total(tx, Key::TariAccumulatedFees, xtr_fees)?;
        add_to_total(tx, Key::TariAccumulatedReceiptExhaustBurn, xtr_receipt_burn)?;

        Ok(CommittedVersion {
            retired_cache_entries,
            num_receipt_events: inserted.receipts.iter().map(|receipt| receipt.num_events).sum(),
            finalized: inserted
                .receipts
                .into_iter()
                .map(|receipt| TransactionFinalizedEvent {
                    transaction_id: receipt.transaction_id,
                    outcome: receipt.outcome,
                })
                .collect(),
            inserted_events: inserted.events,
            skipped_receipts: inserted.skipped,
            skipped_utxos,
            skipped_templates,
        })
    }

    /// Sorts one streamed transition into the buffers the commit is assembled from.
    ///
    /// A transition that retires anything cached reaches `invalidations`, which for a substate's first
    /// creation is the record that it did not exist. Only those whose substate `value_filters` selects
    /// carry a value; the rest arrive as an id and a version under `ALL_HASHES` and must reach nothing
    /// else - indexing one, counting it in the economic totals or emitting an event for it would all be
    /// reading a value that was never sent.
    fn extend_from_substate_update(
        &mut self,
        shard: Shard,
        state_version: StateVersion,
        update: SubstateUpdateProof,
        value_filters: SubstateValueFilterFlags,
    ) {
        self.invalidations.extend(match &update {
            SubstateUpdateProof::Create(create) => {
                SubstateCacheInvalidation::created(create.substate.substate_id(), create.substate.version)
            },
            SubstateUpdateProof::Destroy(destroy) => Some(SubstateCacheInvalidation::destroyed(
                destroy.substate_id.clone(),
                destroy.version,
            )),
        });

        if !value_filters.contains_substate(update.substate_id()) {
            return;
        }

        match &update {
            SubstateUpdateProof::Create(create) => {
                if create.substate.substate_id().is_template() {
                    if let Some(metadata) = &create.substate.template_metadata &&
                        let Some(template_addr) = create.substate.substate_id().as_template()
                    {
                        self.template_catalogue
                            .push((template_addr.as_template_address(), metadata.clone()));
                    }
                    return;
                }
                match create.substate.value().value() {
                    Some(SubstateValue::Utxo(utxo)) => {
                        if let Some(address) = create.substate.substate_id().as_utxo_address() {
                            let is_frozen = utxo.is_frozen();
                            if let Some(ref output) = utxo.output {
                                self.utxos.push(UtxoUpdateRecord::Unspent(Box::new(UtxoUnspent {
                                    address,
                                    version: update.version(),
                                    shard,
                                    state_version,
                                    utxo_output: output.clone(),
                                    is_frozen,
                                })));
                            }
                        } else {
                            warn!(target: LOG_TARGET, "⚠️ NEVER HAPPEN: Received UTXO substate with invalid address: {}", create.substate.substate_id());
                        };
                    },
                    Some(SubstateValue::TransactionReceipt(receipt)) => {
                        if let Some(address) = update.substate_id().as_transaction_receipt_address() {
                            self.transactions.push((address, receipt.clone()));
                        } else {
                            warn!(target: LOG_TARGET, "⚠️ NEVER HAPPEN: Received Transaction Receipt substate with invalid address: {}", create.substate.substate_id());
                        }
                    },
                    Some(SubstateValue::ValidatorFeePool(_)) => {
                        self.validator_fee_pools.push(SubstateData {
                            substate_id: create.substate.substate_id().clone(),
                            version: create.substate.version,
                            value: create.substate.value().clone(),
                            template_metadata: None,
                        });
                    },
                    Some(SubstateValue::ClaimedOutputTombstone(claim)) => {
                        self.xtr_claimed += Amount::from(claim.value);
                    },
                    Some(_) => {
                        warn!(target: LOG_TARGET, "⚠️ NEVER HAPPEN: Received unexpected substate value for created substate: {}", create.substate.substate_id());
                    },
                    None => {
                        let id = create.substate.substate_id();
                        if id.is_transaction_receipt() {
                            warn!(target: LOG_TARGET, "⚠️ Received tx receipt {id} update with no value, it may have been pruned and so will not be indexed");
                        }
                        if let Some(addr) = id.as_utxo_address() {
                            debug!(target: LOG_TARGET, "🌍️ Received UTXO substate {addr} creation with no value. Ignoring as this means it is spent later.");
                        }
                    },
                }
            },
            SubstateUpdateProof::Destroy(destroy) => match &destroy.substate_id {
                SubstateId::Utxo(address) => {
                    self.utxos.push(UtxoUpdateRecord::Spent(UtxoSpent {
                        address: address.clone(),
                        shard,
                        version: update.version(),
                        state_version,
                    }));
                },

                other if other.is_read_only() => {
                    warn!(target: LOG_TARGET, "⚠️ NEVER HAPPEN: Received destroy for read only substate: {}", destroy.substate_id);
                },
                _ => {},
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod running_totals {
        use super::*;
        use crate::store::{IndexerStore, IndexerStoreWriteTransaction};

        async fn store_with_total(key: Key, total: Amount) -> (tempfile::TempDir, SqliteIndexerStore) {
            let dir = tempfile::tempdir().unwrap();
            let store = SqliteIndexerStore::try_create(dir.path().join("indexer.db")).unwrap();
            store
                .with_write_tx(move |tx| tx.key_value_set(key, total))
                .await
                .unwrap();
            (dir, store)
        }

        async fn total(store: &SqliteIndexerStore, key: Key) -> Amount {
            store
                .with_read_tx(move |tx| tx.key_value_get_value::<_, Amount>(key))
                .await
                .unwrap()
        }

        #[tokio::test]
        async fn an_addition_that_overflows_the_total_is_refused() {
            let (_dir, store) = store_with_total(Key::TariAccumulatedExhaustBurn, Amount::MAX).await;

            let result = store
                .with_write_tx(|tx| add_to_total(tx, Key::TariAccumulatedExhaustBurn, Amount::from(1u64)))
                .await;

            assert!(
                matches!(result, Err(StorageError::DataInconsistency { .. })),
                "{result:?}"
            );
            assert_eq!(total(&store, Key::TariAccumulatedExhaustBurn).await, Amount::MAX);
        }

        #[tokio::test]
        async fn an_addition_is_folded_into_the_total() {
            let (_dir, store) = store_with_total(Key::TariAccumulatedFees, Amount::from(40u64)).await;

            store
                .with_write_tx(|tx| add_to_total(tx, Key::TariAccumulatedFees, Amount::from(2u64)))
                .await
                .unwrap();

            assert_eq!(total(&store, Key::TariAccumulatedFees).await, Amount::from(42u64));
        }
    }

    mod version_commit {
        use tari_engine_types::{fees::FeeReceiptBuilder, transaction_receipt::FinalizeOutcome};
        use tari_ootle_transaction::TransactionId;
        use tari_template_lib_types::{Hash32, crypto::RistrettoPublicKeyBytes};

        use super::*;
        use crate::store::IndexerStore;

        const SHARD: Shard = Shard::from_u32(1);

        fn receipt_address(n: u8) -> TransactionReceiptAddress {
            TransactionId::new([n; 32]).into_receipt_address()
        }

        fn receipt(outcome: FinalizeOutcome, total_fees_paid: u64) -> TransactionReceipt {
            TransactionReceipt {
                outcome,
                diff_summary: Default::default(),
                fee_withdrawals: [].into(),
                events: [].into(),
                fee_receipt: FeeReceiptBuilder::default()
                    .with_total_fees_paid(total_fees_paid)
                    .build(),
                epoch: Epoch(1),
                intent_commitment: Default::default(),
            }
        }

        fn template_metadata(name: &str) -> PublishedTemplateMetadata {
            PublishedTemplateMetadata {
                template_name: name.to_string(),
                author_public_key: RistrettoPublicKeyBytes::default(),
                binary_hash: Hash32::default(),
                at_epoch: 1,
                metadata_hash: None,
            }
        }

        async fn commit(store: &SqliteIndexerStore, version: u64, buf: VersionBuf) -> CommittedVersion {
            store
                .with_write_tx(move |tx| {
                    buf.commit(
                        tx,
                        SHARD,
                        StateVersion::new(version),
                        Epoch(1),
                        SyncProgress::default(),
                        &[],
                        &HashSet::new(),
                    )
                })
                .await
                .unwrap()
        }

        async fn total_fees(store: &SqliteIndexerStore) -> Amount {
            store
                .with_read_tx(|tx| tx.key_value_get_value::<_, Amount>(Key::TariAccumulatedFees))
                .await
                .unwrap()
        }

        /// A receipt address is written once on the network, so a conflict at one means some sync peer
        /// was faulty. The incoming receipt is kept out of the totals and the finalize events, and does
        /// not fail the commit: every peer would resend it from the same cursor, stalling the shard for
        /// good.
        #[tokio::test]
        async fn a_receipt_already_indexed_is_neither_counted_nor_announced_again() {
            let dir = tempfile::tempdir().unwrap();
            let store = SqliteIndexerStore::try_create(dir.path().join("indexer.db")).unwrap();

            let committed = commit(&store, 1, VersionBuf {
                transactions: vec![(receipt_address(1), receipt(FinalizeOutcome::FeeIntentCommit, 100))],
                ..Default::default()
            })
            .await;
            assert_eq!(committed.finalized.len(), 1);
            assert_eq!(total_fees(&store).await, Amount::from(100u64));

            let committed = commit(&store, 2, VersionBuf {
                transactions: vec![
                    (receipt_address(1), receipt(FinalizeOutcome::Commit, 999)),
                    (receipt_address(2), receipt(FinalizeOutcome::Commit, 5)),
                ],
                ..Default::default()
            })
            .await;

            let finalized = committed.finalized.iter().map(|f| f.transaction_id).collect::<Vec<_>>();
            assert_eq!(finalized, [TransactionId::new([2; 32])]);
            assert_eq!(committed.skipped_receipts, [receipt_address(1)]);
            assert_eq!(total_fees(&store).await, Amount::from(105u64));
            let stored = store
                .with_read_tx(|tx| tx.get_transaction_receipt(&receipt_address(1)))
                .await
                .unwrap();
            assert!(stored.decode().unwrap().outcome.is_fee_intent_commit());
        }

        #[tokio::test]
        async fn a_catalogued_template_is_not_rewritten() {
            let dir = tempfile::tempdir().unwrap();
            let store = SqliteIndexerStore::try_create(dir.path().join("indexer.db")).unwrap();
            let address = TemplateAddress::from_array([7; 32]);

            commit(&store, 1, VersionBuf {
                template_catalogue: vec![(address, template_metadata("original"))],
                ..Default::default()
            })
            .await;
            let committed = commit(&store, 2, VersionBuf {
                template_catalogue: vec![(address, template_metadata("rewritten"))],
                ..Default::default()
            })
            .await;

            assert_eq!(committed.skipped_templates, [address]);
            let entry = store
                .with_read_tx(move |tx| tx.get_template_catalogue_entry(&address))
                .await
                .unwrap();
            assert_eq!(entry.template_name, "original");
        }
    }

    mod served_checkpoints {
        use indexmap::IndexMap;
        use tari_ootle_storage::consensus_models::EndOfEpochCommand;
        use tari_sidechain::{CommandCommitProof, FixedHash, SidechainBlockCommitProof, SidechainBlockHeader};
        use tari_state_tree::{TreeHash, compute_proof_for_hashes};

        use super::*;

        fn checkpoint(epoch: u64, total_exhaust_burn: u128) -> EpochCheckpoint {
            let key = TreeHash::new([1; 32]);
            let (_, inclusion_proof) = compute_proof_for_hashes([key].into_iter(), key).unwrap();
            let commit_proof = SidechainBlockCommitProof {
                header: SidechainBlockHeader {
                    network: 0,
                    protocol_version: 0,
                    parent_id: FixedHash::zero(),
                    justify_id: FixedHash::zero(),
                    height: 0,
                    epoch,
                    epoch_hash: FixedHash::zero(),
                    shard_group: tari_sidechain::ShardGroup {
                        start: 0,
                        end_inclusive: 255,
                    },
                    proposed_by: Default::default(),
                    state_merkle_root: FixedHash::zero(),
                    command_merkle_root: FixedHash::zero(),
                    transaction_merkle_root: FixedHash::zero(),
                    signature: Default::default(),
                    accumulated_data: tari_sidechain::ShardGroupAccumulatedData { total_exhaust_burn },
                    metadata_hash: FixedHash::zero(),
                },
                proof_elements: vec![],
            };
            EpochCheckpoint::new(
                CommandCommitProof::new(EndOfEpochCommand::new(FixedHash::zero()), commit_proof, inclusion_proof),
                IndexMap::new(),
                tari_ootle_storage::consensus_models::BlockHeaderMetadata {
                    total_leader_fee: 0,
                    timestamp: 0,
                    extra_data: Default::default(),
                    timeout_certificate_id: None,
                    is_dummy: false,
                },
            )
        }

        #[test]
        fn a_burn_the_supply_could_pay_is_accepted() {
            let burn = checkpoint_exhaust_burn(&checkpoint(3, u128::from(u64::MAX))).unwrap();
            assert_eq!(burn, Amount::from(u64::MAX));
        }

        #[test]
        fn a_burn_beyond_the_supply_is_rejected() {
            let result = checkpoint_exhaust_burn(&checkpoint(3, u128::from(u64::MAX) + 1));
            assert!(
                matches!(result, Err(NetworkStateSyncError::InvalidCheckpoint { .. })),
                "{result:?}"
            );
        }

        /// The head checkpoint is stored ahead of the backfill without moving its progress, and the backfill that
        /// reaches it later records its progress without counting its exhaust burn a second time.
        #[tokio::test]
        async fn a_head_checkpoint_stored_ahead_of_the_backfill_is_counted_once() {
            use crate::store::IndexerStore;

            let dir = tempfile::tempdir().unwrap();
            let store = SqliteIndexerStore::try_create(dir.path().join("indexer.db")).unwrap();
            let head = checkpoint(250, 7);
            let shard_group = head.checked_shard_group().unwrap();
            let burn = checkpoint_exhaust_burn(&head).unwrap();

            let ahead = head.clone();
            store
                .with_write_tx(move |tx| write_checkpoint(tx, &ahead, shard_group, burn, None))
                .await
                .unwrap();
            let progress = store
                .with_read_tx(|tx| tx.key_value_get_value::<_, SyncProgress>(Key::SyncProgress))
                .await
                .optional()
                .unwrap();
            assert!(progress.is_none());

            let mut backfilled = SyncProgress::default();
            backfilled.record_checkpoint(shard_group, Epoch(250));
            store
                .with_write_tx(move |tx| write_checkpoint(tx, &head, shard_group, burn, Some(backfilled)))
                .await
                .unwrap();
            let progress = store
                .with_read_tx(|tx| tx.key_value_get_value::<_, SyncProgress>(Key::SyncProgress))
                .await
                .unwrap();
            assert_eq!(progress.checkpoint_epoch(shard_group), Some(Epoch(250)));
            let total = store
                .with_read_tx(|tx| tx.key_value_get_value::<_, Amount>(Key::TariAccumulatedExhaustBurn))
                .await
                .unwrap();
            assert_eq!(total, Amount::from(7u64));
        }

        #[test]
        fn the_previous_epochs_checkpoint_is_synced_first_only_when_earlier_ones_are_missing() {
            assert!(is_backfill(Epoch(0), Epoch(250)));
            assert!(is_backfill(Epoch(248), Epoch(250)));
            assert!(!is_backfill(Epoch(249), Epoch(250)));
            assert!(!is_backfill(Epoch(250), Epoch(250)));
        }

        #[test]
        fn only_checkpoints_from_the_synced_epoch_to_the_previous_epoch_are_kept() {
            let mut checkpoints = (1..=6).map(|epoch| checkpoint(epoch, 0)).collect();

            retain_checkpoints_in_sync_range(&mut checkpoints, Epoch(2), Epoch(4));

            let epochs = checkpoints.iter().map(|c| c.epoch().as_u64()).collect::<Vec<_>>();
            assert_eq!(epochs, [2, 3, 4]);
        }
    }

    mod plan_absorbs_epoch {
        use tari_epoch_manager::service::ShardGroupInfo;
        use tari_ootle_common_types::NumPreshards;

        use super::*;

        fn network(epoch: u64, groups: &[ShardGroup]) -> NetworkDescription {
            NetworkDescription {
                epoch: Epoch(epoch),
                shard_groups: groups.iter().map(|g| (*g, ShardGroupInfo { num_members: 1 })).collect(),
                num_preshards: NumPreshards::P256,
            }
        }

        fn partition(groups: &[ShardGroup]) -> BTreeSet<ShardGroup> {
            groups.iter().copied().collect()
        }

        fn whole() -> ShardGroup {
            ShardGroup::new(1u32, 256)
        }

        #[test]
        fn an_epoch_that_keeps_the_partition_is_absorbed() {
            assert!(plan_absorbs_epoch(
                Epoch(3),
                &partition(&[whole()]),
                &network(4, &[whole()])
            ));
        }

        #[test]
        fn a_changed_partition_draws_a_new_plan() {
            let split = [ShardGroup::new(1u32, 128), ShardGroup::new(129u32, 256)];
            assert!(!plan_absorbs_epoch(
                Epoch(3),
                &partition(&[whole()]),
                &network(4, &split)
            ));
        }

        #[test]
        fn a_plan_drawn_at_epoch_zero_never_absorbs_the_first_epoch() {
            // Epoch zero reports a partition, but the plan drawn against it runs no group loops.
            assert!(!plan_absorbs_epoch(
                Epoch(0),
                &partition(&[whole()]),
                &network(1, &[whole()])
            ));
        }
    }

    mod stream_order {
        use super::*;

        const S1: Shard = Shard::from_u32(1);
        const S2: Shard = Shard::from_u32(2);

        fn cursors(cursors: &[(u32, u64)]) -> Vec<rpc::ShardCursor> {
            cursors
                .iter()
                .map(|&(shard, start_state_version)| rpc::ShardCursor {
                    shard,
                    start_state_version,
                })
                .collect()
        }

        /// A followed stream, which carries every transition.
        fn order(c: &[(u32, u64)]) -> StreamOrder {
            StreamOrder::new(&cursors(c), true)
        }

        /// A head fetch, which leaves out transitions of substates since destroyed.
        fn head_order(c: &[(u32, u64)]) -> StreamOrder {
            StreamOrder::new(&cursors(c), false)
        }

        fn v(n: u64) -> StateVersion {
            StateVersion::new(n)
        }

        #[test]
        fn it_accepts_shards_streamed_one_after_another() {
            let mut o = order(&[(1, 1), (2, 5)]);
            o.accept_batch(S1, v(1), false, 1, 0).unwrap();
            o.accept_batch(S1, v(2), false, 1, 0).unwrap();
            assert_eq!(o.accept_marker(S1, v(2)).unwrap(), Some(v(2)));
            o.accept_batch(S2, v(5), false, 1, 0).unwrap();
            assert_eq!(o.accept_marker(S2, v(5)).unwrap(), Some(v(5)));
        }

        #[test]
        fn it_accepts_a_version_split_across_chunks() {
            let mut o = order(&[(1, 3)]);
            o.accept_batch(S1, v(3), true, 1, 0).unwrap();
            o.accept_batch(S1, v(3), true, 1, 0).unwrap();
            o.accept_batch(S1, v(3), false, 1, 0).unwrap();
            assert_eq!(o.accept_marker(S1, v(3)).unwrap(), Some(v(3)));
        }

        #[test]
        fn it_rejects_a_version_below_the_cursor() {
            // A cursor of 5 asks to resume at v5, so v5 is wanted and everything under it is already held.
            assert!(order(&[(1, 5)]).accept_batch(S1, v(3), false, 1, 0).is_err());
            assert!(order(&[(1, 5)]).accept_batch(S1, v(4), false, 1, 0).is_err());
            order(&[(1, 5)]).accept_batch(S1, v(5), false, 1, 0).unwrap();
        }

        #[test]
        fn it_rejects_a_replayed_version() {
            let mut o = order(&[(1, 9)]);
            o.accept_batch(S1, v(9), false, 1, 0).unwrap();
            assert!(o.accept_batch(S1, v(9), false, 1, 0).is_err());
        }

        #[test]
        fn it_rejects_a_regressing_version() {
            let mut o = head_order(&[(1, 1)]);
            o.accept_batch(S1, v(9), false, 1, 0).unwrap();
            assert!(o.accept_batch(S1, v(8), false, 1, 0).is_err());
        }

        #[test]
        fn a_followed_stream_rejects_a_skipped_version() {
            assert!(order(&[(1, 1)]).accept_batch(S1, v(2), false, 1, 0).is_err());
            assert!(order(&[(1, 1)]).accept_batch(S1, v(2), true, 1, 0).is_err());
            let mut o = order(&[(1, 1)]);
            o.accept_batch(S1, v(1), false, 1, 0).unwrap();
            assert!(o.accept_batch(S1, v(3), false, 1, 0).is_err());
        }

        #[test]
        fn a_followed_stream_rejects_a_marker_past_the_last_version_streamed() {
            let mut o = order(&[(1, 101)]);
            assert!(o.accept_marker(S1, v(100_000)).is_err());
            assert!(o.accept_marker(S1, v(101)).is_err());
            o.accept_batch(S1, v(101), false, 1, 0).unwrap();
            assert!(o.accept_marker(S1, v(102)).is_err());
            assert_eq!(o.accept_marker(S1, v(101)).unwrap(), Some(v(101)));
        }

        #[test]
        fn a_head_fetch_skips_versions_and_establishes_no_level() {
            let mut o = head_order(&[(1, 1)]);
            o.accept_batch(S1, v(4), false, 1, 0).unwrap();
            o.accept_batch(S1, v(9), false, 1, 0).unwrap();
            assert_eq!(o.accept_marker(S1, v(100_000)).unwrap(), None);
        }

        #[test]
        fn it_accepts_a_followed_shard_streamed_again_after_its_marker() {
            let mut o = order(&[(1, 2), (2, 1)]);
            o.accept_batch(S1, v(2), false, 1, 0).unwrap();
            assert_eq!(o.accept_marker(S1, v(2)).unwrap(), Some(v(2)));
            assert_eq!(o.accept_marker(S2, v(0)).unwrap(), Some(v(0)));
            o.accept_batch(S1, v(3), false, 1, 0).unwrap();
            o.accept_marker(S1, v(3)).unwrap();
            // A forced marker on an epoch change closes off nothing new.
            assert_eq!(o.accept_marker(S1, v(3)).unwrap(), Some(v(3)));
            assert!(o.accept_batch(S1, v(3), false, 1, 0).is_err());
        }

        #[test]
        fn it_rejects_an_interleaved_shard_mid_version() {
            let mut o = order(&[(1, 3), (2, 1)]);
            o.accept_batch(S1, v(3), true, 1, 0).unwrap();
            assert!(o.accept_batch(S2, v(1), false, 1, 0).is_err());
        }

        #[test]
        fn it_rejects_a_marker_while_a_version_is_incomplete() {
            let mut o = order(&[(1, 3)]);
            o.accept_batch(S1, v(3), true, 1, 0).unwrap();
            assert!(o.accept_marker(S1, v(3)).is_err());
        }

        #[test]
        fn it_rejects_unrequested_shards() {
            assert!(order(&[(1, 1)]).accept_batch(S2, v(1), false, 1, 0).is_err());
            assert!(order(&[(1, 1)]).accept_marker(S2, v(1)).is_err());
        }

        #[test]
        fn it_rejects_a_batch_with_no_updates() {
            assert!(order(&[(1, 1)]).accept_batch(S1, v(1), false, 0, 0).is_err());
            assert!(order(&[(1, 1)]).accept_batch(S1, v(1), true, 0, 0).is_err());
            assert!(head_order(&[(1, 1)]).accept_batch(S1, v(9), false, 0, 0).is_err());
            // Empty versions cannot walk a followed stream's cursor forward ahead of a marker.
            let mut o = order(&[(1, 1)]);
            o.accept_batch(S1, v(1), false, 1, 0).unwrap();
            assert!(o.accept_batch(S1, v(2), false, 0, 0).is_err());
            assert_eq!(o.accept_marker(S1, v(1)).unwrap(), Some(v(1)));
            assert!(o.accept_marker(S1, v(2)).is_err());
        }

        #[test]
        fn it_accepts_versions_across_the_whole_range() {
            let mut o = head_order(&[(1, 1)]);
            o.accept_batch(S1, v(i64::MAX as u64 + 1), false, 1, 0).unwrap();
            o.accept_batch(S1, v(u64::MAX), false, 1, 0).unwrap();
            assert!(o.accept_batch(S1, v(u64::MAX), false, 1, 0).is_err());

            let mut o = order(&[(1, u64::MAX)]);
            o.accept_batch(S1, v(u64::MAX), false, 1, 0).unwrap();
            assert_eq!(o.accept_marker(S1, v(u64::MAX)).unwrap(), Some(v(u64::MAX)));
            assert!(o.accept_batch(S1, v(u64::MAX), false, 1, 0).is_err());
        }

        #[test]
        fn it_rejects_a_version_streamed_past_the_byte_budget() {
            const CHUNK: usize = 6 * 1024 * 1024;
            let mut o = order(&[(1, 3)]);
            for _ in 0..MAX_BUFFERED_VERSION_BYTES / CHUNK {
                o.accept_batch(S1, v(3), true, 1, CHUNK).unwrap();
            }
            assert!(o.accept_batch(S1, v(3), true, 1, CHUNK).is_err());
        }

        #[test]
        fn the_byte_budget_applies_to_each_version_afresh() {
            let mut o = order(&[(1, 3)]);
            o.accept_batch(S1, v(3), true, 1, MAX_BUFFERED_VERSION_BYTES - 1)
                .unwrap();
            o.accept_batch(S1, v(3), false, 1, 1).unwrap();
            o.accept_batch(S1, v(4), true, 1, MAX_BUFFERED_VERSION_BYTES - 1)
                .unwrap();
            assert!(o.accept_batch(S1, v(4), false, 1, 2).is_err());
        }
    }

    mod cursors_for {
        use super::*;

        const S1: Shard = Shard::from_u32(1);

        fn progress_at(version: u64) -> SyncProgress {
            let mut progress = SyncProgress::default();
            progress.record_state_version(S1, StateVersion::new(version), Epoch(1));
            progress
        }

        #[test]
        fn it_resumes_after_the_recorded_version() {
            assert_eq!(cursors_for(&[S1], &progress_at(12_345))[0].start_state_version, 12_346);
            assert_eq!(cursors_for(&[S1], &SyncProgress::default())[0].start_state_version, 1);
        }

        #[test]
        fn a_shard_at_the_largest_version_gets_no_cursor() {
            let mut progress = progress_at(u64::MAX);
            progress.record_state_version(Shard::from_u32(2), StateVersion::new(u64::MAX - 1), Epoch(1));
            let cursors = cursors_for(&[S1, Shard::from_u32(2)], &progress);
            assert_eq!(cursors.len(), 1);
            assert_eq!(cursors[0].shard, 2);
            assert_eq!(cursors[0].start_state_version, u64::MAX);
        }
    }

    mod checkpoint_verification {
        use std::{
            collections::{HashMap, HashSet},
            sync::Arc,
        };

        use tari_epoch_manager::EpochManagerError;
        use tari_ootle_common_types::{
            SubstateAddress,
            VotePower,
            committee::{Committee, CommitteeInfo, CommitteeMember},
        };
        use tari_ootle_storage::{
            consensus_models::{BlockHeaderMetadata, EndOfEpochCommand},
            global::models::ValidatorNode,
        };
        use tari_sidechain::{CommandCommitProof, FixedHash, SidechainBlockCommitProof, SidechainBlockHeader};
        use tari_state_tree::{TreeHash, compute_proof_for_hashes};
        use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
        use tokio::sync::broadcast;

        use super::*;

        type Addr = String;

        fn low() -> ShardGroup {
            ShardGroup::new(1u32, 127u32)
        }

        fn high() -> ShardGroup {
            ShardGroup::new(128u32, 255u32)
        }

        /// A well-formed checkpoint that no committee signed.
        fn checkpoint(epoch: u64, shard_group: ShardGroup) -> EpochCheckpoint {
            let state_root = checkpoint_with_state_root(epoch, shard_group, FixedHash::zero())
                .compute_state_merkle_root()
                .unwrap();
            checkpoint_with_state_root(epoch, shard_group, FixedHash::from(state_root.into_array()))
        }

        fn checkpoint_with_state_root(
            epoch: u64,
            shard_group: ShardGroup,
            state_merkle_root: FixedHash,
        ) -> EpochCheckpoint {
            let header_metadata = BlockHeaderMetadata {
                total_leader_fee: 0,
                timestamp: epoch,
                extra_data: Default::default(),
                timeout_certificate_id: None,
                is_dummy: false,
            };
            let key = TreeHash::new([1; 32]);
            let (_, inclusion_proof) = compute_proof_for_hashes([key].into_iter(), key).unwrap();
            let commit_proof = SidechainBlockCommitProof {
                header: SidechainBlockHeader {
                    network: 0,
                    protocol_version: 0,
                    parent_id: FixedHash::zero(),
                    justify_id: FixedHash::zero(),
                    height: 0,
                    epoch,
                    epoch_hash: FixedHash::zero(),
                    shard_group: tari_sidechain::ShardGroup {
                        start: shard_group.start().as_u32(),
                        end_inclusive: shard_group.end().as_u32(),
                    },
                    proposed_by: Default::default(),
                    state_merkle_root,
                    command_merkle_root: FixedHash::zero(),
                    transaction_merkle_root: FixedHash::zero(),
                    signature: Default::default(),
                    accumulated_data: Default::default(),
                    metadata_hash: header_metadata.calculate_hash(),
                },
                proof_elements: vec![],
            };
            EpochCheckpoint::new(
                CommandCommitProof::new(EndOfEpochCommand::new(FixedHash::zero()), commit_proof, inclusion_proof),
                Default::default(),
                header_metadata,
            )
        }

        fn committee() -> Arc<Committee<Addr>> {
            Arc::new(Committee::new(vec![CommitteeMember {
                address: "member".to_string(),
                public_key: RistrettoPublicKeyBytes::default(),
                vote_power: VotePower::of(1),
            }]))
        }

        /// The epochs an epoch manager has a record of, each with the committees of its shard groups.
        #[derive(Default)]
        struct EpochRecords {
            committees: HashMap<Epoch, Vec<ShardGroup>>,
            validator_set_hash: Option<ValidatorSetHash>,
        }

        impl EpochRecords {
            fn with(mut self, epoch: u64, shard_groups: &[ShardGroup]) -> Self {
                self.committees.insert(Epoch(epoch), shard_groups.to_vec());
                self
            }
        }

        impl EpochManagerReader for EpochRecords {
            type Addr = Addr;

            fn subscribe(&self) -> broadcast::Receiver<EpochManagerEvent> {
                unimplemented!()
            }

            async fn wait_for_initial_scanning_to_complete(&self) -> Result<(), EpochManagerError> {
                unimplemented!()
            }

            async fn get_all_validator_nodes(&self, _: Epoch) -> Result<Vec<ValidatorNode<Addr>>, EpochManagerError> {
                unimplemented!()
            }

            async fn get_committee_info_by_validator_address(
                &self,
                _: Epoch,
                _: &Addr,
            ) -> Result<CommitteeInfo, EpochManagerError> {
                unimplemented!()
            }

            async fn get_committee_for_substate(
                &self,
                _: Epoch,
                _: SubstateAddress,
            ) -> Result<Arc<Committee<Addr>>, EpochManagerError> {
                unimplemented!()
            }

            async fn get_validator_node_by_public_key(
                &self,
                _: Epoch,
                _: RistrettoPublicKeyBytes,
            ) -> Result<ValidatorNode<Addr>, EpochManagerError> {
                unimplemented!()
            }

            async fn get_our_validator_node(&self, _: Epoch) -> Result<ValidatorNode<Addr>, EpochManagerError> {
                unimplemented!()
            }

            async fn get_local_committee_info(&self, _: Epoch) -> Result<CommitteeInfo, EpochManagerError> {
                unimplemented!()
            }

            async fn get_committee_info(&self, _: Epoch, _: ShardGroup) -> Result<CommitteeInfo, EpochManagerError> {
                unimplemented!()
            }

            async fn get_committee_info_for_substate(
                &self,
                _: Epoch,
                _: SubstateAddress,
            ) -> Result<CommitteeInfo, EpochManagerError> {
                unimplemented!()
            }

            async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
                unimplemented!()
            }

            async fn get_current_epoch_hash(&self) -> Result<FixedHash, EpochManagerError> {
                unimplemented!()
            }

            async fn get_epoch_hash(&self, epoch: Epoch) -> Result<FixedHash, EpochManagerError> {
                if self.committees.contains_key(&epoch) {
                    Ok(FixedHash::zero())
                } else {
                    Err(EpochManagerError::NoEpochFound(epoch))
                }
            }

            async fn get_validator_set_hash(&self, epoch: Epoch) -> Result<ValidatorSetHash, EpochManagerError> {
                self.validator_set_hash
                    .ok_or(EpochManagerError::NoValidatorSet { epoch })
            }

            async fn get_validator_set_committee_proof(
                &self,
                _: Epoch,
                _: ShardGroup,
            ) -> Result<tari_state_tree::ValidatorSetCommitteeProof, EpochManagerError> {
                unimplemented!()
            }

            async fn get_num_committees(&self, _: Epoch) -> Result<u32, EpochManagerError> {
                unimplemented!()
            }

            async fn get_committee_by_shard_group(
                &self,
                epoch: Epoch,
                shard_group: ShardGroup,
            ) -> Result<Arc<Committee<Addr>>, EpochManagerError> {
                match self.committees.get(&epoch) {
                    Some(shard_groups) if shard_groups.contains(&shard_group) => Ok(committee()),
                    _ => Err(EpochManagerError::NoEpochFound(epoch)),
                }
            }

            async fn get_committees_overlapping_shard_group(
                &self,
                _: Epoch,
                _: ShardGroup,
            ) -> Result<HashMap<ShardGroup, Committee<Addr>>, EpochManagerError> {
                unimplemented!()
            }

            async fn get_random_committee_member(
                &self,
                _: Epoch,
                _: Option<ShardGroup>,
                _: HashSet<Addr>,
            ) -> Result<ValidatorNode<Addr>, EpochManagerError> {
                unimplemented!()
            }

            async fn lock_epoch(&self, _: Epoch) -> Result<(), EpochManagerError> {
                unimplemented!()
            }

            async fn get_observed_epoch_hash(&self, _: Epoch) -> Result<Option<FixedHash>, EpochManagerError> {
                unimplemented!()
            }

            async fn get_birthday_epoch(&self) -> Result<Option<Epoch>, EpochManagerError> {
                unimplemented!()
            }
        }

        fn is_invalid_checkpoint(result: &Result<(), NetworkStateSyncError>) -> bool {
            matches!(result, Err(NetworkStateSyncError::InvalidCheckpoint { .. }))
        }

        #[tokio::test]
        async fn a_checkpoint_before_the_records_is_checked_for_form_only() {
            let records = EpochRecords::default().with(5, &[low(), high()]);

            let result = verify_checkpoint(Network::MainNet, &records, &checkpoint(2, low()), low(), Epoch(5)).await;

            assert!(result.is_ok(), "{result:?}");
        }

        #[tokio::test]
        async fn an_older_recorded_checkpoint_is_checked_against_its_committee() {
            let records = EpochRecords::default()
                .with(3, &[low(), high()])
                .with(5, &[low(), high()]);

            let result = verify_checkpoint(Network::MainNet, &records, &checkpoint(3, low()), low(), Epoch(5)).await;

            assert!(is_invalid_checkpoint(&result), "{result:?}");
        }

        /// A shard group is one the peer names, so naming one the epoch did not have must not skip the committee check.
        #[tokio::test]
        async fn a_shard_group_outside_a_recorded_epoch_is_rejected() {
            let records = EpochRecords::default()
                .with(3, &[low(), high()])
                .with(5, &[low(), high()]);
            let all = ShardGroup::new(1u32, 255u32);

            for epoch in [3, 5] {
                let result =
                    verify_checkpoint(Network::MainNet, &records, &checkpoint(epoch, all), all, Epoch(5)).await;
                assert!(is_invalid_checkpoint(&result), "{epoch}: {result:?}");
                assert!(result.unwrap_err().is_peer_fault());
            }
        }

        #[test]
        fn a_next_epoch_set_other_than_the_local_one_diverges() {
            let local = ValidatorSetHash::from_array([1; 32]);
            let committed = ValidatorSetHash::from_array([2; 32]);

            assert!(check_validator_set(Epoch(4), local, local).is_ok());
            assert!(matches!(
                check_validator_set(Epoch(4), local, committed),
                Err(NetworkStateSyncError::ValidatorSetDiverged { epoch, .. }) if epoch == Epoch(4)
            ));
        }
    }
}
