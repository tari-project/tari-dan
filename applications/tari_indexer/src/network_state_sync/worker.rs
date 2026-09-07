//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    pin::pin,
    sync::Arc,
};

use futures::{StreamExt, future::Either, stream::FuturesUnordered};
use log::*;
use ootle_network::Network;
#[cfg(feature = "metrics")]
use tari_consensus::consensus_constants::ConsensusConstants;
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
use tari_ootle_common_types::{Epoch, ShardGroup, StateVersion, VotePower, optional::Optional, shard::Shard};
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
use tari_ootle_transaction::TransactionId;
use tari_rpc_framework::RpcRequestOptions;
use tari_shutdown::ShutdownSignal;
use tari_template_lib_types::{Amount, TemplateAddress, TransactionReceiptAddress};
use tokio::{
    sync::{broadcast, watch},
    time,
};
use tokio_util::sync::CancellationToken;

#[cfg(feature = "metrics")]
use crate::{network_state_sync::NetworkStateMetrics, store::ReadOnlyStore};
use crate::{
    network_state_sync::{
        committee_client::{ValidatorCommitteeRpcPool, ValidatorRpcSession},
        config::NetworkWideStateSyncConfig,
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

#[derive(Clone)]
pub struct NetworkWideStateSync {
    network: Network,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    networking: NetworkingHandle<TariMessagingSpec>,
    store: SqliteIndexerStore,
    stats: SyncStats,
    config: NetworkWideStateSyncConfig,
    notify: Notify<IndexerEvent>,
    transaction_event_notify: Notify<TransactionEvent>,
    validator_status: ValidatorStatusMonitor,
    shard_watermarks: Arc<ShardWatermarks>,
    #[cfg(feature = "metrics")]
    metrics: NetworkStateMetrics,
    #[cfg(feature = "metrics")]
    consensus_constants: ConsensusConstants,
}

impl NetworkWideStateSync {
    pub fn new(
        network: Network,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        networking: NetworkingHandle<TariMessagingSpec>,
        storage: SqliteIndexerStore,
        config: NetworkWideStateSyncConfig,
        notify: Notify<IndexerEvent>,
        transaction_event_notify: Notify<TransactionEvent>,
        validator_status: ValidatorStatusMonitor,
        shard_watermarks: Arc<ShardWatermarks>,
        #[cfg(feature = "metrics")] metrics: NetworkStateMetrics,
        #[cfg(feature = "metrics")] consensus_constants: ConsensusConstants,
    ) -> Self {
        Self {
            network,
            epoch_manager,
            networking,
            store: storage,
            stats: SyncStats::new(),
            config,
            notify,
            transaction_event_notify,
            validator_status,
            shard_watermarks,
            #[cfg(feature = "metrics")]
            metrics,
            #[cfg(feature = "metrics")]
            consensus_constants,
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
                let target_burn_rate_bps = self.consensus_constants.exhaust_burn_rate(current_epoch).as_bps();
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
    #[expect(clippy::too_many_lines)]
    async fn sync_group_checkpoints(
        &self,
        shard_group: ShardGroup,
        pool: &mut ValidatorCommitteeRpcPool,
        epoch: Epoch,
        progress: &SharedSyncProgress,
    ) -> Result<(), NetworkStateSyncError> {
        let Some(prev_epoch) = epoch.checked_sub(Epoch(1)) else {
            return Ok(());
        };
        let from_epoch = progress
            .lock()
            .await
            .checkpoint_epoch(shard_group)
            .unwrap_or_else(Epoch::zero);
        if from_epoch >= prev_epoch {
            debug!(target: LOG_TARGET, "🌍️ No checkpoints to sync for shard group {shard_group} from epoch {from_epoch}");
            return Ok(());
        }
        info!(target: LOG_TARGET, "🌍️ Syncing checkpoints from {from_epoch} for shard group {shard_group}");
        // Perform sync operations using the pool and checkpoint
        let validator_status = self.validator_status.clone();
        let checkpoints: Vec<_> = pool
            .try_with_random_members(|mut session| {
                let validator_status = validator_status.clone();
                async move {
                    // Verify how far this peer has committed before trusting it as a sync source.
                    // `probe` only returns Err for a forged/malformed proof (other failures are
                    // logged internally and return Ok(None)), which disqualifies the peer so
                    // another committee member is tried.
                    if let Err(e) = validator_status.probe(&mut session, shard_group).await {
                        return Err(NetworkStateSyncError::InvalidCommitProof {
                            details: format!("shard group {shard_group}: {e}"),
                        });
                    }
                    let resp = session
                        .get_checkpoints(rpc::GetCheckpointsRequest {
                            from_epoch: Some(from_epoch.into()),
                            num_to_return: 100,
                        })
                        .await?;

                    debug!(target: LOG_TARGET, "🌍️ Received {} checkpoints for shard group {} from peer {}", resp.checkpoints.len(), shard_group, session.peer_address());

                    resp.checkpoints
                        .into_iter()
                        .map(|cp| {
                            EpochCheckpoint::try_from(cp).map_err(|e| {
                                NetworkStateSyncError::InvalidCheckpoint {
                                    details: format!(
                                        "Failed to convert checkpoint for shard group {}: {}",
                                        shard_group, e
                                    ),
                                }
                            })
                        })
                        .collect()
                }
            })
            .await?;

        if checkpoints.is_empty() {
            info!(target: LOG_TARGET, "🌍️ No checkpoints found for shard group {shard_group} from epoch {from_epoch} (prev_epoch {prev_epoch})");
            let mut progress = progress.lock().await;
            progress.record_checkpoint(shard_group, prev_epoch);
            let sync_progress_snapshot = progress.clone();
            self.store
                .with_write_tx(move |tx| tx.key_value_set(Key::SyncProgress, sync_progress_snapshot))
                .await?;
            return Ok(());
        }

        info!(target: LOG_TARGET, "🌍️ Found {} checkpoints for shard group {shard_group} from epoch {from_epoch}", checkpoints.len());

        for checkpoint in checkpoints {
            info!(target: LOG_TARGET, "🌍️ Validating checkpoint for shard group {shard_group}: {}", checkpoint.header().calculate_hash());

            let checkpoint_shard_group =
                checkpoint
                    .checked_shard_group()
                    .map_err(|e| NetworkStateSyncError::InvalidCheckpoint {
                        details: format!("Checkpoint for shard group {} is not valid: {}", shard_group, e),
                    })?;

            // TODO: we require historical committees to validate older checkpoints. Figure out the best way to
            //       avoid needing the full historical validator data (e.g. VN merkle inclusion proof + historic L1
            // block MR), or,       decide it is ok to require this data to be locally stored by all
            // indexers. For now, to avoid       complexity that may be removed later, we'll skip
            // validating them and only validate prev_epochs       checkpoint.
            if checkpoint.epoch() == prev_epoch {
                // Use the checkpoint's own shard group, not the iterator's: the network may have
                // had a different shard-group structure at prev_epoch than the current epoch we
                // are iterating, so the QC is signed by the committee for `checkpoint_shard_group`,
                // not `shard_group`.
                let committee = self
                    .epoch_manager
                    .get_committee_by_shard_group(checkpoint.epoch(), checkpoint_shard_group)
                    .await?;
                checkpoint
                    .validate(checkpoint.epoch(), committee.quorum_threshold(), |pk| {
                        Ok(committee.get_power_by_public_key(pk).unwrap_or_else(VotePower::zero))
                    })
                    .map_err(|e| NetworkStateSyncError::InvalidCheckpoint {
                        details: format!(
                            "Failed to validate checkpoint for shard group {}: {}",
                            checkpoint_shard_group, e
                        ),
                    })?;
            } else {
                checkpoint
                    .validate_well_formed()
                    .map_err(|e| NetworkStateSyncError::InvalidCheckpoint {
                        details: format!(
                            "Failed to validate well-formedness of checkpoint for shard group {}: {}",
                            checkpoint_shard_group, e
                        ),
                    })?;
                debug!(target: LOG_TARGET, "🌍️ Skipping checkpoint for shard group {shard_group} with epoch {} (expected {})", checkpoint.epoch(), prev_epoch);
            }

            info!(target: LOG_TARGET, "🌍️ Inserting checkpoint for {}, shard group {}", checkpoint.epoch(), checkpoint_shard_group);

            self.stats.increment_checkpoints();
            let xtr_exhausted = Amount::from(checkpoint.header().accumulated_data().total_exhaust_burn);
            let checkpoint_epoch = checkpoint.epoch();
            let mut progress = progress.lock().await;
            progress.record_checkpoint(shard_group, checkpoint_epoch);
            let sync_progress_snapshot = progress.clone();
            self.store
                .with_write_tx(move |tx| {
                    if !tx.epoch_checkpoint_exists(shard_group, checkpoint_epoch)? {
                        tx.insert_or_ignore_epoch_checkpoint(&checkpoint)?;

                        let exhausted = tx
                            .key_value_get_value::<_, Amount>(Key::TariAccumulatedExhaustBurn)
                            .optional()?;

                        let new_exhausted = exhausted.unwrap_or_else(Amount::zero) + xtr_exhausted;
                        tx.key_value_set(Key::TariAccumulatedExhaustBurn, new_exhausted)?;
                    }
                    tx.key_value_set(Key::SyncProgress, sync_progress_snapshot)
                })
                .await?;
        }
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

        while let Some(result) = groups.next().await {
            result?;
        }
        Ok(())
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
            if let Err(err) = self
                .sync_group_checkpoints(shard_group, &mut pool, current_epoch, &progress)
                .await
            {
                if !err.is_peer_fault() {
                    return Err(err);
                }
                warn!(target: LOG_TARGET, "⚠️ Checkpoint sync for shard group {} failed: {}", shard_group, err);
                self.pause(shard_group, "checkpoint sync failed", &cancel, &mut epoch)
                    .await;
                continue;
            }
            let mut session = match pool.new_session().await {
                Ok(s) => s,
                Err(e) => {
                    warn!(target: LOG_TARGET, "⚠️ Failed to create session for shard group {}: {}", shard_group, e);
                    self.pause(shard_group, "no session", &cancel, &mut epoch).await;
                    continue;
                },
            };
            match self.validator_status.probe(&mut session, shard_group).await {
                Ok(Some(verified_tip)) => {
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
                .sync_shard_group_state(shards, &progress, shard_group, &mut session, &cancel, &mut epoch)
                .await
            {
                Ok(StreamEnd::Cancelled) => return Ok(()),
                Ok(StreamEnd::TimedOut) => {
                    debug!(target: LOG_TARGET, "🌍️ State sync stream for shard group {shard_group} from {} had nothing to send for the deadline. Reopening", session.peer_address());
                },
                Ok(StreamEnd::EpochAdvanced) => {
                    info!(target: LOG_TARGET, "🌍️ Epoch advanced to {}. Reopening state sync for shard group {shard_group}", *epoch.borrow());
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
    /// rather than on the reopen after the deadline.
    async fn sync_shard_group_state(
        &mut self,
        shards: impl Iterator<Item = Shard>,
        progress: &SharedSyncProgress,
        shard_group: ShardGroup,
        session: &mut ValidatorRpcSession,
        cancel: &CancellationToken,
        epoch: &mut watch::Receiver<Epoch>,
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
                )
                .await?;
            if matches!(end, StreamEnd::Cancelled | StreamEnd::EpochAdvanced) {
                return Ok(end);
            }
        }

        let cursors = cursors_for(&shards, &*progress.lock().await);
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
    ) -> Result<StreamEnd, NetworkStateSyncError> {
        let mut order = StreamOrder::new(&cursors);

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
        let mut update_buf = Vec::new();
        let mut invalidations_buf = Vec::new();
        let mut utxos_buf = Vec::new();
        let mut transactions_buf = Vec::new();
        let mut validator_fee_pools_buf = Vec::new();
        let mut template_catalogue_buf: Vec<(TemplateAddress, PublishedTemplateMetadata)> = Vec::new();
        let mut xtr_claimed = Amount::zero();
        let mut xtr_fees = Amount::zero();
        let mut xtr_receipt_burn = Amount::zero();

        loop {
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(StreamEnd::Cancelled),
                Ok(()) = epoch.changed() => return Ok(StreamEnd::EpochAdvanced),
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
                Some(rpc::sync_state_response::Response::Complete(complete)) => {
                    let shard = Shard::from(complete.shard);
                    order
                        .accept_marker(shard)
                        .map_err(|details| NetworkStateSyncError::InvalidStateUpdate { details })?;
                    // Terminal watermark: advance recorded progress to the version the producer is
                    // synced to. This covers trailing versions that streamed no updates because their
                    // substates are all filtered out for our subscription - without it we could never
                    // observe that we have caught up to such a shard and would re-sync it from scratch
                    // every round.
                    let synced_to = StateVersion::new(complete.synced_to_version);
                    let msg_epoch =
                        complete
                            .epoch
                            .map(Epoch::from)
                            .ok_or_else(|| NetworkStateSyncError::InvalidStateUpdate {
                                details: "Received sync completion without epoch".to_string(),
                            })?;
                    // Only persist when the watermark advances - a caught-up shard re-sends the same
                    // version on every reopen, and we must not write on every empty one.
                    let mut progress = progress.lock().await;
                    let already_synced = progress.last_state_version(shard).is_some_and(|v| synced_to <= v);
                    if !already_synced {
                        progress.record_state_version(shard, synced_to, msg_epoch);
                        let sync_progress_snapshot = progress.clone();
                        self.store
                            .clone()
                            .with_write_tx(move |tx| tx.key_value_set(Key::SyncProgress, sync_progress_snapshot))
                            .await?;
                    }
                    drop(progress);
                    // The completion marker is the only point at which this shard is known to be
                    // level with the committee, which is what the substate cache needs: mid-stream
                    // the indexer holds every transition up to some version while the chain is
                    // arbitrarily far ahead of it. Confirmed even when the watermark did not move -
                    // a quiet shard is still one this indexer is keeping up with.
                    self.shard_watermarks.confirm(shard, synced_to);
                    level_shards.insert(shard);
                    debug!(target: LOG_TARGET, "🌍️ Completed state sync for shard {shard} in shard group {shard_group} to epoch {msg_epoch} and state version {synced_to}");
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
            order
                .accept_batch(shard, state_version, batch.has_more)
                .map_err(|details| NetworkStateSyncError::InvalidStateUpdate { details })?;
            let msg_epoch = batch
                .epoch
                .map(Epoch::from)
                .ok_or_else(|| NetworkStateSyncError::InvalidStateUpdate {
                    details: "Received state update without epoch".to_string(),
                })?;

            for update in batch.updates {
                let update =
                    SubstateUpdateProof::try_from(update).map_err(|e| NetworkStateSyncError::InvalidStateUpdate {
                        details: format!("Failed to convert substate update: {}", e),
                    })?;

                extend_bufs_from_substate_update(
                    &self.notify,
                    shard,
                    state_version,
                    update,
                    msg_epoch,
                    value_filters,
                    &mut update_buf,
                    &mut invalidations_buf,
                    &mut utxos_buf,
                    &mut transactions_buf,
                    &mut validator_fee_pools_buf,
                    &mut template_catalogue_buf,
                    &mut xtr_claimed,
                    &mut xtr_fees,
                    &mut xtr_receipt_burn,
                )?;
            }
            if batch.has_more {
                debug!(target: LOG_TARGET, "🌍️ more updates for shard {shard} (epoch: {msg_epoch}, state version: {state_version})");
                continue;
            }

            debug!(target: LOG_TARGET, "🌍️ Received {} updates for shard {shard} (epoch: {msg_epoch}, state version: {state_version})", update_buf.len());

            self.stats.increase_state_updates(update_buf.len());

            let updates = std::mem::take(&mut update_buf);
            let invalidations = std::mem::take(&mut invalidations_buf);
            let utxos = std::mem::take(&mut utxos_buf);
            let transactions = std::mem::take(&mut transactions_buf);
            let validator_fee_pools = std::mem::take(&mut validator_fee_pools_buf);
            let template_catalogue = std::mem::take(&mut template_catalogue_buf);

            let updates_len = updates.len();
            let utxos_len = utxos.len();
            let transactions_len = transactions.len();
            let template_catalogue_len = template_catalogue.len();
            let event_count: usize = transactions.iter().map(|(_, t)| t.events.len()).sum();
            self.stats.increase_events(event_count);

            let mut progress = progress.lock().await;
            progress.record_state_version(shard, state_version, msg_epoch);
            let sync_progress_snapshot = progress.clone();

            let network = self.network;
            let event_filters = self.config.event_filters.clone();
            let watched_templates = self.config.watched_templates.clone();
            let xtr_claimed_snapshot = xtr_claimed;
            let xtr_fees_snapshot = xtr_fees;
            let xtr_receipt_burn_snapshot = xtr_receipt_burn;

            let inserted_events = self
                .store
                .clone()
                .with_write_tx(move |tx| -> Result<Vec<InsertedEvent>, StorageError> {
                    debug!(target: LOG_TARGET, "✅ Committing {} updates for shard {shard} (epoch: {msg_epoch}, state version: {state_version})", updates_len);
                    // TODO: this is not currently used. Consider removing.
                    tx.batch_insert_substate_transitions(network, shard, state_version, updates)?;
                    // Must commit with the watermark below: the substate cache serves an entry on the
                    // argument that it holds every transition up to that watermark, which a reader
                    // seeing one of the two without the other would break.
                    tx.substate_cache_invalidate(invalidations, state_version)?;
                    debug!(target: LOG_TARGET, "✅ Committing {} UTXOs for shard {shard} (epoch: {msg_epoch})", utxos_len);
                    tx.batch_insert_utxo_updates(msg_epoch, utxos)?;
                    for substate_data in validator_fee_pools {
                        tx.upsert_substate(&substate_data)?;
                    }
                    debug!(target: LOG_TARGET, "✅ Committing {} transactions for shard {shard} (epoch: {msg_epoch})", transactions_len);
                    let inserted = tx.batch_insert_transaction_receipts(transactions, &event_filters)?;
                    if !watched_templates.is_empty() {
                        process_watched_substate_events(tx, &inserted, &watched_templates)?;
                    }

                    if !template_catalogue.is_empty() {
                        debug!(target: LOG_TARGET, "✅ Upserting {} template catalogue entries for shard {shard} (epoch: {msg_epoch})", template_catalogue_len);
                        for (template_addr, metadata) in template_catalogue {
                            tx.upsert_template_catalogue(&template_addr, &metadata)?;
                        }
                    }

                    tx.key_value_set(Key::SyncProgress, sync_progress_snapshot)?;
                    let claimed = tx.key_value_get_value(Key::TariAccumulatedClaimed).optional()?;
                    let new_claimed = claimed.unwrap_or_else(Amount::zero) + xtr_claimed_snapshot;
                    tx.key_value_set(Key::TariAccumulatedClaimed, new_claimed)?;
                    let fees = tx.key_value_get_value(Key::TariAccumulatedFees).optional()?;
                    let new_fees = fees.unwrap_or_else(Amount::zero) + xtr_fees_snapshot;
                    tx.key_value_set(Key::TariAccumulatedFees, new_fees)?;
                    let receipt_burn = tx.key_value_get_value(Key::TariAccumulatedReceiptExhaustBurn).optional()?;
                    let new_receipt_burn = receipt_burn.unwrap_or_else(Amount::zero) + xtr_receipt_burn_snapshot;
                    tx.key_value_set(Key::TariAccumulatedReceiptExhaustBurn, new_receipt_burn)?;
                    Ok(inserted)
                })
                .await?;
            drop(progress);

            // The stream flushes (has_more == false) once per state version, so each commit must fold only
            // that version's delta. Reset the running totals here, mirroring the buffer drains above.
            xtr_claimed = Amount::zero();
            xtr_fees = Amount::zero();
            xtr_receipt_burn = Amount::zero();

            for inserted in inserted_events {
                self.transaction_event_notify.notify(TransactionEvent {
                    id: inserted.id,
                    transaction_id: inserted.transaction_id,
                    event: inserted.event,
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
    /// The plan is being wound down.
    Cancelled,
}

/// A cursor per shard resuming after the version recorded for it, in the order of `shards`. A shard
/// never synced resumes from version one.
fn cursors_for(shards: &[Shard], progress: &SyncProgress) -> Vec<rpc::ShardCursor> {
    shards
        .iter()
        .map(|&shard| rpc::ShardCursor {
            shard: shard.as_u32(),
            start_state_version: progress.last_state_version(shard).map_or(0, |v| v.as_u64()) + 1,
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
struct StreamOrder {
    /// Highest version committed per requested shard, seeded from the cursor so a responder cannot
    /// replay versions the caller already holds.
    committed_versions: HashMap<Shard, StateVersion>,
    /// Set while a version is split across chunks, until the chunk that flushes it.
    pending_chunk: Option<(Shard, StateVersion)>,
}

impl StreamOrder {
    fn new(cursors: &[rpc::ShardCursor]) -> Self {
        Self {
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

    fn accept_batch(&mut self, shard: Shard, state_version: StateVersion, has_more: bool) -> Result<(), String> {
        let Some(committed_version) = self.committed_versions.get(&shard).copied() else {
            return Err(format!("Received batch for unrequested shard {shard}"));
        };
        if state_version <= committed_version {
            return Err(format!(
                "Received v{state_version} for shard {shard}, which is not ahead of the committed v{committed_version}"
            ));
        }
        if let Some(pending) = self.pending_chunk &&
            pending != (shard, state_version)
        {
            return Err(format!(
                "Received v{state_version} of shard {shard} while v{} of shard {} is still incomplete",
                pending.1, pending.0
            ));
        }

        if has_more {
            self.pending_chunk = Some((shard, state_version));
        } else {
            self.pending_chunk = None;
            self.committed_versions.insert(shard, state_version);
        }
        Ok(())
    }

    fn accept_marker(&mut self, shard: Shard) -> Result<(), String> {
        if !self.committed_versions.contains_key(&shard) {
            return Err(format!("Received completion marker for unrequested shard {shard}"));
        }
        if let Some((pending_shard, pending_version)) = self.pending_chunk {
            return Err(format!(
                "Received completion marker for shard {shard} while v{pending_version} of shard {pending_shard} is \
                 still incomplete"
            ));
        }
        Ok(())
    }
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
                        .get("prev_template")
                        .and_then(|v| TemplateAddress::from_hex(v).ok());

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

/// Sorts one streamed transition into the buffers a commit is assembled from.
///
/// A transition that retires anything cached reaches `invalidations_buf`, which for a substate's
/// first creation is the record that it did not exist. Only those whose substate `value_filters` selects carry a
/// value; the rest arrive as an id and a version under `ALL_HASHES` and must reach nothing else -
/// indexing one, counting it in the economic totals or emitting an event for it would all be reading
/// a value that was never sent.
fn extend_bufs_from_substate_update(
    notify: &Notify<IndexerEvent>,
    shard: Shard,
    state_version: StateVersion,
    update: SubstateUpdateProof,
    msg_epoch: Epoch,
    value_filters: SubstateValueFilterFlags,
    update_buf: &mut Vec<(Epoch, SubstateUpdateProof)>,
    invalidations_buf: &mut Vec<SubstateCacheInvalidation>,
    utxos_buf: &mut Vec<UtxoUpdateRecord>,
    transactions_buf: &mut Vec<(TransactionReceiptAddress, TransactionReceipt)>,
    validator_fee_pools_buf: &mut Vec<SubstateData>,
    template_catalogue_buf: &mut Vec<(TemplateAddress, PublishedTemplateMetadata)>,
    xtr_claimed_mut: &mut Amount,
    xtr_fees_mut: &mut Amount,
    xtr_receipt_burn_mut: &mut Amount,
) -> Result<(), NetworkStateSyncError> {
    invalidations_buf.extend(match &update {
        SubstateUpdateProof::Create(create) => {
            SubstateCacheInvalidation::created(create.substate.substate_id(), create.substate.version)
        },
        SubstateUpdateProof::Destroy(destroy) => Some(SubstateCacheInvalidation::destroyed(
            destroy.substate_id.clone(),
            destroy.version,
        )),
    });

    if !value_filters.contains_substate(update.substate_id()) {
        return Ok(());
    }

    match &update {
        SubstateUpdateProof::Create(create) => {
            if create.substate.substate_id().is_template() {
                if let Some(metadata) = &create.substate.template_metadata &&
                    let Some(template_addr) = create.substate.substate_id().as_template()
                {
                    template_catalogue_buf.push((template_addr.as_template_address(), metadata.clone()));
                }
                update_buf.push((msg_epoch, update));
                return Ok(());
            }
            match create.substate.value().value() {
                Some(SubstateValue::Utxo(utxo)) => {
                    if let Some(address) = create.substate.substate_id().as_utxo_address() {
                        let is_frozen = utxo.is_frozen();
                        if let Some(ref output) = utxo.output {
                            utxos_buf.push(UtxoUpdateRecord::Unspent(Box::new(UtxoUnspent {
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
                        // Accumulate the realized-share pair from the same receipt: what the payer spent
                        // and the burn taken out of it, so `burn / paid` recovers the share independent
                        // of the header-sourced burn total.
                        let fee_receipt = receipt.fee_receipt();
                        *xtr_receipt_burn_mut += Amount::from(fee_receipt.exhaust_burn());
                        *xtr_fees_mut += Amount::from(fee_receipt.total_fees_paid());

                        notify.notify(TransactionFinalizedEvent {
                            transaction_id: TransactionId::from_receipt_address(address),
                            outcome: receipt.outcome,
                        });
                        transactions_buf.push((address, receipt.clone()));
                    } else {
                        warn!(target: LOG_TARGET, "⚠️ NEVER HAPPEN: Received Transaction Receipt substate with invalid address: {}", create.substate.substate_id());
                    }
                },
                Some(SubstateValue::ValidatorFeePool(_)) => {
                    validator_fee_pools_buf.push(SubstateData {
                        substate_id: create.substate.substate_id().clone(),
                        version: create.substate.version,
                        value: create.substate.value().clone(),
                        template_metadata: None,
                    });
                },
                Some(SubstateValue::ClaimedOutputTombstone(claim)) => {
                    *xtr_claimed_mut += Amount::from(claim.value);
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
                utxos_buf.push(UtxoUpdateRecord::Spent(UtxoSpent {
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

    update_buf.push((msg_epoch, update));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        fn order(cursors: &[(u32, u64)]) -> StreamOrder {
            let cursors = cursors
                .iter()
                .map(|&(shard, start_state_version)| rpc::ShardCursor {
                    shard,
                    start_state_version,
                })
                .collect::<Vec<_>>();
            StreamOrder::new(&cursors)
        }

        fn v(n: u64) -> StateVersion {
            StateVersion::new(n)
        }

        #[test]
        fn it_accepts_shards_streamed_one_after_another() {
            let mut o = order(&[(1, 1), (2, 1)]);
            o.accept_batch(S1, v(1), false).unwrap();
            o.accept_batch(S1, v(4), false).unwrap();
            o.accept_marker(S1).unwrap();
            o.accept_batch(S2, v(7), false).unwrap();
            o.accept_marker(S2).unwrap();
        }

        #[test]
        fn it_accepts_a_version_split_across_chunks() {
            let mut o = order(&[(1, 1)]);
            o.accept_batch(S1, v(3), true).unwrap();
            o.accept_batch(S1, v(3), true).unwrap();
            o.accept_batch(S1, v(3), false).unwrap();
            o.accept_marker(S1).unwrap();
        }

        #[test]
        fn it_rejects_a_version_below_the_cursor() {
            // A cursor of 5 asks to resume at v5, so v5 is wanted and everything under it is already held.
            assert!(order(&[(1, 5)]).accept_batch(S1, v(3), false).is_err());
            assert!(order(&[(1, 5)]).accept_batch(S1, v(4), false).is_err());
            order(&[(1, 5)]).accept_batch(S1, v(5), false).unwrap();
        }

        #[test]
        fn it_rejects_a_replayed_version() {
            let mut o = order(&[(1, 1)]);
            o.accept_batch(S1, v(9), false).unwrap();
            assert!(o.accept_batch(S1, v(9), false).is_err());
        }

        #[test]
        fn it_rejects_a_regressing_version() {
            let mut o = order(&[(1, 1)]);
            o.accept_batch(S1, v(9), false).unwrap();
            assert!(o.accept_batch(S1, v(8), false).is_err());
        }

        #[test]
        fn it_accepts_a_followed_shard_streamed_again_after_its_marker() {
            let mut o = order(&[(1, 1), (2, 1)]);
            o.accept_batch(S1, v(2), false).unwrap();
            o.accept_marker(S1).unwrap();
            o.accept_marker(S2).unwrap();
            o.accept_batch(S1, v(3), false).unwrap();
            o.accept_marker(S1).unwrap();
            // A forced marker on an epoch change closes off nothing new.
            o.accept_marker(S1).unwrap();
            assert!(o.accept_batch(S1, v(3), false).is_err());
        }

        #[test]
        fn it_rejects_an_interleaved_shard_mid_version() {
            let mut o = order(&[(1, 1), (2, 1)]);
            o.accept_batch(S1, v(3), true).unwrap();
            assert!(o.accept_batch(S2, v(1), false).is_err());
        }

        #[test]
        fn it_rejects_a_marker_while_a_version_is_incomplete() {
            let mut o = order(&[(1, 1)]);
            o.accept_batch(S1, v(3), true).unwrap();
            assert!(o.accept_marker(S1).is_err());
        }

        #[test]
        fn it_rejects_unrequested_shards() {
            assert!(order(&[(1, 1)]).accept_batch(S2, v(1), false).is_err());
            assert!(order(&[(1, 1)]).accept_marker(S2).is_err());
        }
    }
}
