//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use log::*;
use ootle_network::Network;
use rand::seq::SliceRandom;
use tari_consensus::{
    check_quorum_certificate_signatures,
    traits::{ConsensusSpec, SyncManager, SyncStatus},
};
use tari_consensus_types::{LeafBlock, ProposalCertificate};
use tari_engine_types::ProtocolVersion;
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    ShardGroup,
    VotePower,
    committee::{Committee, CommitteeMember},
    optional::Optional,
    shard::Shard,
};
use tari_ootle_p2p::{
    PeerAddress,
    proto::rpc::{GetCheckpointsRequest, GetCheckpointsResponse, GetHighQcRequest, ShardCursor, SyncStateRequest},
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{BookkeepingEpochAgnosticRead, EpochCheckpoint, SubstateValueFilterFlags},
};
use tari_rpc_framework::RpcError;
use tari_state_tree::Version;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tari_validator_node_rpc::{
    client::{TariValidatorNodeRpcClientFactory, ValidatorNodeClientFactory},
    rpc_service::ValidatorNodeRpcClient,
};

use crate::{
    error::RpcStateSyncError,
    shard_sync::{ShardSync, calculate_state_root_for_shard, discard_all_unverified_state},
    stats::StateSyncStats,
    version_proofs::CommitteeProofValidator,
};

const LOG_TARGET: &str = "tari::ootle::rpc_state_sync";
/// The number of sync runs a peer that served invalid state is left out of, unless configured otherwise.
pub const DEFAULT_SYNC_PEER_SKIP_RUNS: u32 = 3;
/// The base time a peer's state stream has to complete each version, unless configured otherwise. A version's
/// allowance grows with its bytes at the minimum throughput.
pub const DEFAULT_SYNC_PROGRESS_TIMEOUT: Duration = Duration::from_secs(60);
/// The streaming rate, in bytes per second, at which a version's bytes extend its time allowance, unless configured
/// otherwise.
pub const DEFAULT_SYNC_MIN_THROUGHPUT_BYTES_PER_SEC: u64 = 256 * 1024;

pub struct RpcStateSyncClientProtocol<TConsensusSpec: ConsensusSpec> {
    network: Network,
    epoch_manager: TConsensusSpec::EpochManager,
    state_store: TConsensusSpec::StateStore,
    client_factory: TariValidatorNodeRpcClientFactory,
    signer_service: TConsensusSpec::SignerService,
    /// The number of state versions the state store keeps the tree nodes of.
    state_history_length: Version,
    stats: StateSyncStats,
    skip_sync: bool,
    /// Checkpoints fetched and validated during the current sync, by shard group. A checkpoint is stored only
    /// once every shard synced from it matches it, since a stored checkpoint tells peers this node holds that
    /// state. Its shard states are trusted, so a stream that fails against it is the streaming peer's fault.
    unsaved_checkpoints: HashMap<ShardGroup, EpochCheckpoint>,
    /// Peers that served invalid state, left out of the next `peer_skip_runs` sync runs.
    skipped_peers: SkippedPeers<PeerAddress>,
    peer_skip_runs: u32,
    progress_timeout: Duration,
    min_throughput_bytes_per_sec: u64,
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
        state_history_length: Version,
    ) -> Self {
        Self {
            network,
            epoch_manager,
            state_store,
            client_factory,
            signer_service,
            state_history_length,
            stats: StateSyncStats::default(),
            skip_sync: false,
            unsaved_checkpoints: HashMap::new(),
            skipped_peers: SkippedPeers::default(),
            peer_skip_runs: DEFAULT_SYNC_PEER_SKIP_RUNS,
            progress_timeout: DEFAULT_SYNC_PROGRESS_TIMEOUT,
            min_throughput_bytes_per_sec: DEFAULT_SYNC_MIN_THROUGHPUT_BYTES_PER_SEC,
        }
    }

    /// Sets the base time a peer's state stream has to complete each version. Zero sets no limit.
    pub fn with_progress_timeout(mut self, progress_timeout: Duration) -> Self {
        self.progress_timeout = progress_timeout;
        self
    }

    /// Sets the streaming rate, in bytes per second, at which a version's bytes extend its time allowance. Zero
    /// grants no extension beyond the base time.
    pub fn with_min_throughput(mut self, bytes_per_sec: u64) -> Self {
        self.min_throughput_bytes_per_sec = bytes_per_sec;
        self
    }

    /// Sets the number of sync runs a peer that served invalid state is left out of.
    pub fn with_peer_skip_runs(mut self, peer_skip_runs: u32) -> Self {
        self.peer_skip_runs = peer_skip_runs;
        self
    }

    /// Leaves `peer` out of the next sync runs if `err` shows it served invalid state. A transport failure says
    /// nothing about the peer's state, so it does not count.
    fn skip_peer_if_misbehaved(&mut self, peer: &PeerAddress, err: &RpcStateSyncError) {
        if err.is_peer_fault() {
            warn!(
                target: LOG_TARGET,
                "🛜 Leaving {peer} out of the next {} state sync run(s): {err}", self.peer_skip_runs
            );
            self.skipped_peers.skip(*peer, self.peer_skip_runs);
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
            info!(target: LOG_TARGET, "🛜 Checkpoint already stored: {cp}");
            return Ok(Some(cp));
        }
        if let Some(checkpoint) = self.unsaved_checkpoints.get(&for_shard_group) {
            return Ok(Some(checkpoint.clone()));
        }

        self.stats.total_requests += 1;

        let mut checkpoints = match client
            .get_checkpoints(GetCheckpointsRequest {
                from_epoch: Some(prev_epoch.into()),
                num_to_return: 1,
                shard_group: Some(for_shard_group.encode_as_u32()),
            })
            .await
        {
            Ok(GetCheckpointsResponse { checkpoints }) => checkpoints,
            Err(RpcError::RequestFailed(err)) if err.is_not_found() => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        let Some(checkpoint) = checkpoints.pop() else {
            return Ok(None);
        };
        let checkpoint = EpochCheckpoint::try_from(checkpoint).map_err(RpcStateSyncError::InvalidResponse)?;
        let shard_group = checkpoint.checked_shard_group().map_err(|err| {
            RpcStateSyncError::InvalidResponse(anyhow!(
                "Fetched checkpoint for epoch {} has invalid shard group: {err}",
                checkpoint.epoch()
            ))
        })?;
        if checkpoint.epoch() != prev_epoch || shard_group != for_shard_group {
            return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                "Requested checkpoint for epoch {prev_epoch} shard group {for_shard_group} but peer returned epoch {} \
                 shard group {shard_group}",
                checkpoint.epoch()
            )));
        }
        info!(target: LOG_TARGET, "🛜 Checkpoint: {checkpoint}");
        self.validate_checkpoint(&checkpoint, prev_committee, prev_epoch)?;
        self.check_validator_set(&checkpoint, prev_epoch + Epoch(1)).await?;
        self.unsaved_checkpoints.insert(for_shard_group, checkpoint.clone());
        Ok(Some(checkpoint))
    }

    async fn start_state_sync(
        &mut self,
        client: &mut ValidatorNodeRpcClient,
        shard: Shard,
        num_preshards: NumPreshards,
        checkpoint: &EpochCheckpoint,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let checkpoint_shard_root = checkpoint.get_shard_root(shard);
        let shard_sync = ShardSync::new(
            self.network,
            num_preshards,
            &self.state_store,
            shard,
            checkpoint_shard_root,
            checkpoint.get_shard_state_version(shard),
            checkpoint.epoch(),
            self.state_history_length,
        );
        // A V0 checkpoint commits no state versions, so only from V1 does its version bound the stream.
        let checkpoint_commits_state_versions = checkpoint
            .protocol_version()
            .map_err(|e| RpcStateSyncError::InvalidResponse(e.into()))? >=
            ProtocolVersion::V1;
        let validator = CommitteeProofValidator::new(&self.epoch_manager, checkpoint.epoch());
        let shard_sync = shard_sync
            .with_version_proofs(&validator)
            .with_trusted_checkpoint_version(checkpoint_commits_state_versions)
            .with_progress_timeout(self.progress_timeout)
            .with_min_throughput(self.min_throughput_bytes_per_sec);
        let maybe_persisted_state_version = shard_sync.discard_unverified_state()?;

        if shard_sync.local_state_root(maybe_persisted_state_version)? == checkpoint_shard_root {
            info!(target: LOG_TARGET, "Checkpoint state root indicates no further state changes. Nothing to sync for {shard}");
            shard_sync.align_to_checkpoint_version(maybe_persisted_state_version)?;
            return Ok(None);
        }

        let start_state_version = shard_sync.start_state_version(maybe_persisted_state_version)?;
        info!(
            target: LOG_TARGET,
            "🛜Syncing from v{start_state_version}",
        );

        self.stats.total_requests += 1;
        let state_stream = client
            .sync_state(SyncStateRequest {
                cursors: vec![ShardCursor {
                    shard: shard.as_u32(),
                    start_state_version,
                }],
                until_epoch: Some(checkpoint.epoch().into()),
                value_filters: SubstateValueFilterFlags::all_substates().bits(),
                follow: false,
                include_version_proofs: true,
            })
            .await?;

        shard_sync
            .sync_from_stream(&mut self.stats, maybe_persisted_state_version, state_stream)
            .await
    }

    /// True if this node's committed state already matches `checkpoint` for every shard it is responsible for (its
    /// local shard group plus the global shard), with the value of every up substate. Distinguishes a rolled-back node
    /// (complete state, checkpoint retained) from one whose first-time state sync was interrupted after one shard
    /// group's checkpoint was stored but before the others finished streaming (checkpoint present, state incomplete).
    async fn local_state_matches_checkpoint(&self, checkpoint: &EpochCheckpoint) -> Result<bool, RpcStateSyncError> {
        let local_info = self.epoch_manager.get_local_committee_info(checkpoint.epoch()).await?;
        self.state_store.with_read_tx(|tx| {
            for shard in local_info.shard_group().shard_iter_with_global() {
                let version = tx.state_tree_versions_get_latest(shard)?;
                let local_root = calculate_state_root_for_shard(tx, shard, version)?;
                if local_root != checkpoint.get_shard_root(shard) ||
                    tx.state_sync_missing_values_get_earliest(shard)?.is_some()
                {
                    return Ok(false);
                }
            }
            Ok(true)
        })
    }

    async fn get_sync_sources(
        &self,
        local_shard_group: ShardGroup,
        current_epoch: Epoch,
        our_vn_addr: &PeerAddress,
        skipped_peers: &HashSet<PeerAddress>,
    ) -> Result<HashMap<ShardGroup, SyncSource<PeerAddress>>, RpcStateSyncError> {
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
                    SyncSource::new(signing_committee, &local_committee, our_vn_addr, skipped_peers),
                )
            })
            .collect::<HashMap<_, _>>();
        info!(target: LOG_TARGET, "🛜 Querying {} committee(s) from epoch {}", sources.len(), prev_epoch);
        Ok(sources)
    }

    /// Fails if `checkpoint`, which ends the epoch before `epoch`, commits `epoch` to a validator set other than the
    /// one this node assigned. Such a node would run `epoch` on committees the rest of the network does not, so it
    /// does not sync into it. A checkpoint that names no set leaves nothing to check.
    ///
    /// The divergence is this node's own, so every peer and shard group would fail the same way: callers return it
    /// rather than trying another source.
    async fn check_validator_set(&self, checkpoint: &EpochCheckpoint, epoch: Epoch) -> Result<(), RpcStateSyncError> {
        let Some(committed) = checkpoint.next_epoch_validator_set_hash() else {
            return Ok(());
        };
        let local = self.epoch_manager.get_validator_set_hash(epoch).await?;
        if local != committed {
            return Err(RpcStateSyncError::ValidatorSetDiverged {
                epoch,
                local,
                committed,
            });
        }
        Ok(())
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

    /// Synchronizes the given [`Shard`].
    async fn sync_shard(
        &mut self,
        shard: Shard,
        shard_group: ShardGroup,
        epoch: Epoch,
        num_preshards: NumPreshards,
        source: &SyncSource<PeerAddress>,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let prev_epoch = epoch
            .checked_sub(Epoch(1))
            .ok_or_else(|| RpcStateSyncError::InvalidResponse(anyhow!("Epoch is zero")))?;
        info!(target: LOG_TARGET, "🛜 Syncing state for shard {shard} and epoch {}", prev_epoch);

        let mut last_hard_error = None;
        let mut saw_unavailable_peer = false;

        // A peer that served invalid state earlier in this run is left out of the rest of it too.
        let peers = leave_out_skipped(&source.serving_peers, |addr| self.skipped_peers.is_skipped(addr));
        for member in &peers {
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
                    saw_unavailable_peer = true;
                    continue;
                },
                Err(err @ RpcStateSyncError::ValidatorSetDiverged { .. }) => return Err(err),
                Err(err) => {
                    if is_checkpoint_temporarily_unavailable(&err, prev_epoch) {
                        saw_unavailable_peer = true;
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
                    self.skip_peer_if_misbehaved(&member.address, &err);
                    last_hard_error = Some(err);
                    continue;
                },
            };

            match self
                .start_state_sync(&mut client, shard, num_preshards, &checkpoint)
                .await
            {
                Ok(maybe_version) => {
                    return Ok(maybe_version);
                },
                Err(err) if is_peer_unavailable(&err) => {
                    saw_unavailable_peer = true;
                    warn!(
                        target: LOG_TARGET,
                        "⚠️{member} is not ready to serve state for shard {shard}: {err}. Attempting another peer if \
                         available"
                    );
                    continue;
                },
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️Failed to sync state from {member}: {err}. Attempting another peer if available"
                    );
                    self.skip_peer_if_misbehaved(&member.address, &err);
                    last_hard_error = Some(err);
                    continue;
                },
            }
        }

        if let Some(err) = last_hard_error {
            return Err(err);
        }

        if saw_unavailable_peer {
            return Err(RpcStateSyncError::CheckpointNotAvailable { epoch: prev_epoch });
        }

        Err(RpcStateSyncError::SyncFailedAllPeers {
            num_peers: source.serving_peers.len(),
        })
    }

    async fn sync_global_shard(
        &mut self,
        current_epoch: Epoch,
        num_preshards: NumPreshards,
        sources: &HashMap<ShardGroup, SyncSource<PeerAddress>>,
    ) -> Result<Option<Version>, RpcStateSyncError> {
        let mut last_error = None;

        for (sg, source) in sources {
            // Any previous-epoch checkpoint carries the global shard root, so the first shard group to succeed
            // justifies the whole global shard sync.
            let result = self
                .sync_shard(Shard::global(), *sg, current_epoch, num_preshards, source)
                .await;
            match result {
                Ok(maybe_version) => {
                    let Some(version) = maybe_version else {
                        info!(target: LOG_TARGET, "🛜 No state changes for global shard");
                        return Ok(None);
                    };
                    info!(target: LOG_TARGET, "🛜 Synced global shard to v{}", version);
                    return Ok(Some(version));
                },
                Err(err @ RpcStateSyncError::ValidatorSetDiverged { .. }) => return Err(err),
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
        self.unsaved_checkpoints.clear();
        discard_all_unverified_state(&self.state_store)?;
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
        let skipped_peers = self.skipped_peers.start_run(current_epoch);
        let sync_sources = match self
            .get_sync_sources(local_info.shard_group(), current_epoch, &our_vn.address, &skipped_peers)
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
        let num_preshards = local_info.num_preshards();

        self.sync_global_shard(current_epoch, num_preshards, &sync_sources)
            .await?;

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
                self.sync_shard(shard, shard_group, current_epoch, num_preshards, &source)
                    .await?;
            }
            // The global shard synced first, so every shard this node takes from the checkpoint now matches it.
            if let Some(checkpoint) = self.unsaved_checkpoints.remove(&shard_group) {
                self.state_store.with_write_tx(|tx| checkpoint.save(tx))?;
            }
        }

        self.stats.total_time = timer.elapsed();
        Ok(())
    }
}

/// Where to sync one shard group's state from. Checkpoint validity depends only on the quorum signatures of
/// the committee that produced it, so who serves the bytes is chosen for availability rather than trust.
struct SyncSource<TAddr> {
    /// The previous epoch's committee for the shard group. Its quorum threshold and voting powers are the
    /// only thing a fetched checkpoint is validated against.
    signing_committee: Committee<TAddr>,
    /// Peers to request the checkpoint and state from, in order of preference, excluding this node.
    /// Members of our current committee are running consensus and hold the previous epoch's state for our
    /// whole shard group (either as continuing members or because they synced it from the same checkpoint),
    /// so they come first; signing-committee members that left at the epoch boundary are the fallback.
    /// Peers in `skipped_peers` are left out, unless that would leave none.
    serving_peers: Vec<CommitteeMember<TAddr>>,
}

impl<TAddr: Clone + Eq + Hash> SyncSource<TAddr> {
    fn new(
        signing_committee: Committee<TAddr>,
        local_committee: &Committee<TAddr>,
        our_vn_addr: &TAddr,
        skipped_peers: &HashSet<TAddr>,
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
            .collect::<Vec<_>>();

        Self {
            signing_committee,
            serving_peers: leave_out_skipped(&serving_peers, |addr| skipped_peers.contains(addr)),
        }
    }
}

/// The peers in `peers` that are not skipped, in order, or all of them if every one is skipped.
fn leave_out_skipped<TAddr: Clone>(
    peers: &[CommitteeMember<TAddr>],
    is_skipped: impl Fn(&TAddr) -> bool,
) -> Vec<CommitteeMember<TAddr>> {
    let unskipped = peers
        .iter()
        .filter(|m| !is_skipped(&m.address))
        .cloned()
        .collect::<Vec<_>>();
    if unskipped.is_empty() && !peers.is_empty() {
        warn!(
            target: LOG_TARGET,
            "🛜 Every one of the {} peer(s) able to serve state is skipped for serving invalid state. Trying them \
             anyway.",
            peers.len()
        );
        return peers.to_vec();
    }
    unskipped
}

/// Peers that served invalid state, with the number of sync runs each is still left out of.
#[derive(Debug)]
struct SkippedPeers<TAddr> {
    /// The highest epoch a run has been at. Committees change with the epoch, so the list starts empty when a run
    /// moves past it. A run may target an epoch below it, as a probed target can lag the current epoch.
    epoch: Option<Epoch>,
    runs_left: HashMap<TAddr, u32>,
}

impl<TAddr> Default for SkippedPeers<TAddr> {
    fn default() -> Self {
        Self {
            epoch: None,
            runs_left: HashMap::new(),
        }
    }
}

impl<TAddr: Clone + Eq + Hash> SkippedPeers<TAddr> {
    /// Leaves `peer` out of the next `runs` sync runs.
    fn skip(&mut self, peer: TAddr, runs: u32) {
        if runs > 0 {
            self.runs_left.insert(peer, runs);
        }
    }

    fn is_skipped(&self, peer: &TAddr) -> bool {
        self.runs_left.contains_key(peer)
    }

    /// Starts a sync run at `epoch` and returns the peers it leaves out, counting the run against each of them.
    fn start_run(&mut self, epoch: Epoch) -> HashSet<TAddr> {
        if self.epoch.is_none_or(|current| epoch > current) {
            self.runs_left.clear();
            self.epoch = Some(epoch);
        }
        let skipped = self.runs_left.keys().cloned().collect();
        self.runs_left.retain(|_, runs| {
            *runs -= 1;
            *runs > 0
        });
        skipped
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
        let mut later_genesis_reports = LaterGenesisReports::default();

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

            if qc.epoch() > leaf_epoch {
                // A zero-block QC for our group at a later epoch is that epoch's genesis justify. It is unsigned, so it
                // counts only as one member's report (see `LaterGenesisReports`).
                if qc.shard_group() == leaf.shard_group() && qc.justifies_zero_block() {
                    later_genesis_reports.record(member, qc.epoch());
                    if let Some(epoch) = later_genesis_reports.proven_epoch(committee.max_failures()) {
                        return Ok(ProbeOutcome::HigherQcSeen { epoch });
                    }
                    continue;
                }

                // A QC for another shard group is evidence about that group's chain only.
                if qc.shard_group() != leaf.shard_group() {
                    debug!(
                        target: LOG_TARGET,
                        "🛜 Probe: {} returned QC at epoch {} for {}, a group other than leaf {}; skipping",
                        member.address, qc.epoch(), qc.shard_group(), leaf,
                    );
                    continue;
                }
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
                self.network,
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
        // Every path to `Running` passes through here or through `sync`, so consensus never starts on top of state
        // an interrupted sync left unverified, and the stale-node GC never runs while such state could still be
        // rewound.
        discard_all_unverified_state(&self.state_store)?;

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
            // The state-root match is required, not merely the checkpoint's presence: state sync stores each shard
            // group's checkpoint as soon as that group's shards match it, so a first-time sync interrupted before
            // the remaining groups also leaves a leaf-less node holding a checkpoint — but with incomplete state.
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

/// Members of the leaf committee whose probe answer is a later epoch's genesis certificate (a zero-block QC
/// for the leaf's shard group at a later epoch).
///
/// A member creates a genesis only after committing the epoch before it, and a committee moves one epoch at a time, so
/// an honest reporter at epoch E shows every epoch from the leaf's up to E - 1 was finalised. The certificate carries
/// no signatures, so one answer proves nothing. Reporters whose combined power exceeds the committee's fault tolerance
/// include at least one honest member, whose epoch bounds the lowest reported one from above.
#[derive(Debug, Default)]
struct LaterGenesisReports {
    reporters: HashSet<RistrettoPublicKeyBytes>,
    power: VotePower,
    lowest_epoch: Option<Epoch>,
}

impl LaterGenesisReports {
    fn record<TAddr>(&mut self, member: &CommitteeMember<TAddr>, epoch: Epoch) {
        if self.reporters.insert(member.public_key) {
            self.power += member.vote_power;
            self.lowest_epoch = Some(self.lowest_epoch.map_or(epoch, |lowest| lowest.min(epoch)));
        }
    }

    /// The lowest reported epoch, once the reporters' power exceeds `max_failures`.
    fn proven_epoch(&self, max_failures: VotePower) -> Option<Epoch> {
        if self.power > max_failures {
            self.lowest_epoch
        } else {
            None
        }
    }
}

/// True if the peer could not supply the `prev_epoch` checkpoint yet, which another peer or a later attempt
/// may.
fn is_checkpoint_temporarily_unavailable(err: &RpcStateSyncError, prev_epoch: Epoch) -> bool {
    match err {
        RpcStateSyncError::CheckpointNotAvailable { epoch } => *epoch == prev_epoch,
        err => is_peer_unavailable(err),
    }
}

/// True if the peer rejected the request because it is not yet in a state to serve it.
fn is_peer_unavailable(err: &RpcStateSyncError) -> bool {
    matches!(err, RpcStateSyncError::RpcError(RpcError::RequestFailed(status)) if status.is_unavailable())
}

#[cfg(test)]
mod tests {
    use tari_rpc_framework::RpcStatus;

    use super::*;

    fn request_failed(status: RpcStatus) -> RpcStateSyncError {
        RpcStateSyncError::RpcError(RpcError::RequestFailed(status))
    }

    fn member(id: u8) -> CommitteeMember<u8> {
        CommitteeMember {
            address: id,
            public_key: RistrettoPublicKeyBytes::from([id; 32]),
            vote_power: VotePower::of(1),
        }
    }

    fn committee(ids: &[u8]) -> Committee<u8> {
        Committee::new(ids.iter().map(|id| member(*id)).collect())
    }

    fn serving_peers(source: &SyncSource<u8>) -> HashSet<u8> {
        source.serving_peers.iter().map(|m| m.address).collect()
    }

    #[test]
    fn a_skipped_peer_is_left_out_of_the_next_runs() {
        let signing = committee(&[1, 2, 3]);
        let local = committee(&[0, 1, 2, 3]);
        let mut skipped = SkippedPeers::default();
        let epoch = Epoch(5);

        assert!(skipped.start_run(epoch).is_empty());
        skipped.skip(2, DEFAULT_SYNC_PEER_SKIP_RUNS);

        for _ in 0..DEFAULT_SYNC_PEER_SKIP_RUNS {
            let source = SyncSource::new(signing.clone(), &local, &0, &skipped.start_run(epoch));
            assert_eq!(serving_peers(&source), HashSet::from([1, 3]));
        }
        let source = SyncSource::new(signing, &local, &0, &skipped.start_run(epoch));
        assert_eq!(serving_peers(&source), HashSet::from([1, 2, 3]));
        assert!(skipped.runs_left.is_empty());
    }

    #[test]
    fn peers_that_joined_this_epoch_serve_state() {
        let source = SyncSource::new(committee(&[1]), &committee(&[0, 1, 2]), &0, &HashSet::new());

        assert_eq!(serving_peers(&source), HashSet::from([1, 2]));
    }

    #[test]
    fn every_peer_is_tried_when_all_are_skipped() {
        let signing = committee(&[1, 2]);
        let local = committee(&[0, 1, 2]);
        let mut skipped = SkippedPeers::default();
        let epoch = Epoch(5);
        skipped.start_run(epoch);
        skipped.skip(1, DEFAULT_SYNC_PEER_SKIP_RUNS);
        skipped.skip(2, DEFAULT_SYNC_PEER_SKIP_RUNS);

        let source = SyncSource::new(signing, &local, &0, &skipped.start_run(epoch));

        assert_eq!(serving_peers(&source), HashSet::from([1, 2]));
    }

    #[test]
    fn a_peer_skipped_during_a_run_is_left_out_of_the_rest_of_it() {
        let signing = committee(&[1, 2, 3]);
        let local = committee(&[0, 1, 2, 3]);
        let mut skipped = SkippedPeers::default();
        let source = SyncSource::new(signing, &local, &0, &skipped.start_run(Epoch(5)));

        skipped.skip(2, DEFAULT_SYNC_PEER_SKIP_RUNS);
        let peers = leave_out_skipped(&source.serving_peers, |addr| skipped.is_skipped(addr));
        assert_eq!(
            peers.iter().map(|m| m.address).collect::<HashSet<_>>(),
            HashSet::from([1, 3])
        );

        skipped.skip(1, DEFAULT_SYNC_PEER_SKIP_RUNS);
        skipped.skip(3, DEFAULT_SYNC_PEER_SKIP_RUNS);
        let peers = leave_out_skipped(&source.serving_peers, |addr| skipped.is_skipped(addr));
        assert_eq!(peers.len(), 3);
    }

    #[test]
    fn a_new_epoch_clears_the_skipped_peers() {
        let mut skipped = SkippedPeers::default();
        skipped.start_run(Epoch(5));
        skipped.skip(1, DEFAULT_SYNC_PEER_SKIP_RUNS);

        assert!(skipped.start_run(Epoch(6)).is_empty());
        assert!(skipped.runs_left.is_empty());
    }

    #[test]
    fn a_run_at_an_earlier_epoch_keeps_the_skipped_peers() {
        let mut skipped = SkippedPeers::default();
        skipped.start_run(Epoch(5));
        skipped.skip(1, DEFAULT_SYNC_PEER_SKIP_RUNS);

        assert_eq!(skipped.start_run(Epoch(4)), HashSet::from([1]));
        assert_eq!(skipped.start_run(Epoch(5)), HashSet::from([1]));
        assert!(skipped.start_run(Epoch(6)).is_empty());
    }

    #[test]
    fn a_failure_against_the_checkpoint_is_the_streaming_peer_s_fault() {
        let mismatch = RpcStateSyncError::StateRootMismatch {
            expected: tari_state_tree::SPARSE_MERKLE_PLACEHOLDER_HASH,
            actual: tari_state_tree::SPARSE_MERKLE_PLACEHOLDER_HASH,
        };
        assert!(mismatch.is_peer_fault());
        assert!(RpcStateSyncError::ConflictsWithCheckpoint(anyhow!("above the checkpoint")).is_peer_fault());
    }

    #[test]
    fn only_invalid_state_counts_as_misbehaviour() {
        assert!(RpcStateSyncError::InvalidResponse(anyhow!("bad")).is_peer_fault());
        assert!(!request_failed(RpcStatus::general("")).is_peer_fault());
        assert!(!RpcStateSyncError::RpcError(RpcError::ReplyTimeout).is_peer_fault());
    }

    #[test]
    fn genesis_reports_up_to_the_fault_tolerance_prove_nothing() {
        let max_failures = VotePower::of(1);
        let mut reports = LaterGenesisReports::default();
        reports.record(&member(1), Epoch(8));
        assert_eq!(reports.proven_epoch(max_failures), None);
    }

    #[test]
    fn genesis_reports_beyond_the_fault_tolerance_prove_the_lowest_epoch() {
        let max_failures = VotePower::of(1);
        let mut reports = LaterGenesisReports::default();
        reports.record(&member(1), Epoch(9));
        reports.record(&member(2), Epoch(8));
        assert_eq!(reports.proven_epoch(max_failures), Some(Epoch(8)));
    }

    #[test]
    fn a_repeated_genesis_reporter_counts_once() {
        let max_failures = VotePower::of(1);
        let mut reports = LaterGenesisReports::default();
        reports.record(&member(1), Epoch(8));
        reports.record(&member(1), Epoch(8));
        assert_eq!(reports.proven_epoch(max_failures), None);
    }

    #[test]
    fn an_unavailable_peer_is_temporary_whatever_its_details() {
        assert!(is_checkpoint_temporarily_unavailable(
            &request_failed(RpcStatus::unavailable("")),
            Epoch(3)
        ));
    }

    #[test]
    fn only_an_unavailable_rejection_marks_the_peer_unavailable() {
        assert!(is_peer_unavailable(&request_failed(RpcStatus::unavailable(""))));
        assert!(!is_peer_unavailable(&request_failed(RpcStatus::general(""))));
        assert!(!is_peer_unavailable(&RpcStateSyncError::CheckpointNotAvailable {
            epoch: Epoch(3)
        }));
    }

    #[test]
    fn a_general_failure_is_not_temporary() {
        assert!(!is_checkpoint_temporarily_unavailable(
            &request_failed(RpcStatus::general("Consensus is not running on this node")),
            Epoch(3)
        ));
    }

    #[test]
    fn a_missing_checkpoint_is_temporary_only_for_the_requested_epoch() {
        assert!(is_checkpoint_temporarily_unavailable(
            &RpcStateSyncError::CheckpointNotAvailable { epoch: Epoch(3) },
            Epoch(3)
        ));
        assert!(!is_checkpoint_temporarily_unavailable(
            &RpcStateSyncError::CheckpointNotAvailable { epoch: Epoch(2) },
            Epoch(3)
        ));
    }
}
