//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use log::*;
use ootle_network::Network;
use tari_indexer_client::types::EpochCheckpointEntry;
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    ShardGroup,
    StateVersion,
    ValidatorSetHash,
    optional::{IsNotFoundError, Optional},
    shard::Shard,
};
use tari_ootle_light_client::{
    DEFAULT_MAX_WALK_EPOCHS,
    LightClientError,
    TrustedValidatorSet,
    TrustedValidatorSetHash,
    VerifiedCommittee,
    WalkPlan,
    plan_walk,
};

use crate::{
    apis::config::ConfigKey,
    models::{
        AnchoredValidatorSet,
        CheckpointStateVersion,
        ValidatorSetAnchor,
        ValidatorSetReanchor,
        ValidatorSetReanchoredEvent,
        ValidatorSetStatus,
        WalletEvent,
    },
    network::WalletNetworkInterface,
    storage::{WalletEventStoreWriter, WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

const LOG_TARGET: &str = "tari::ootle::wallet_sdk::apis::validator_set";

/// The shard whose shard group the wallet walks the validator set handover through. Every shard is in some shard group
/// in every epoch.
pub const WALK_SHARD: Shard = Shard::first();

/// The shortest the wallet allows that an epoch lasted on average when it judges whether it could have fallen behind by
/// a claimed gap of more than [`DEFAULT_MAX_WALK_EPOCHS`]: three quarters of the 20 minute epoch of ten base layer
/// blocks at the two minute target. Difficulty adjustment keeps the average over hundreds of epochs near the target.
/// Epochs that run faster only delay a re-anchor to [`REANCHOR_AFTER_FAILING_FOR`].
const MIN_EPOCH_DURATION: Duration = Duration::from_secs(15 * 60);

/// How long the walk must fail on every sync before the wallet re-anchors in its place, whatever the gap.
const REANCHOR_AFTER_FAILING_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// The most checkpoints one request of a walk asks for, which is the most an indexer serves at once.
const CHECKPOINTS_PER_REQUEST: u32 = 100;

/// The most committees [`ValidatorSetShared`] keeps verified at once.
const MAX_CACHED_COMMITTEES: usize = 256;

/// What every clone of the SDK shares about the validator sets it trusts.
#[derive(Default)]
pub struct ValidatorSetShared {
    /// Held while walking or re-anchoring, so that one change to the trusted sets completes before the next starts.
    lock: tokio::sync::Mutex<()>,
    /// Committees verified against the trusted validator set of their epoch, by epoch and shard group. An entry is
    /// used only while the set it was verified against is the one the wallet trusts for its epoch.
    committees: Mutex<HashMap<(Epoch, ShardGroup), VerifiedCommittee>>,
    /// The shard state versions of the latest checkpoint of each shard group last verified, by shard group. Cleared
    /// when the wallet anchors anew.
    shard_group_versions: Mutex<HashMap<ShardGroup, VerifiedShardGroupVersions>>,
}

/// The state versions that a shard group's verified checkpoint of the end of `epoch` commits for its shards.
#[derive(Debug, Clone)]
struct VerifiedShardGroupVersions {
    epoch: Epoch,
    versions: Vec<(Shard, StateVersion)>,
}

impl ValidatorSetShared {
    fn cached_shard_group_versions(&self) -> HashMap<ShardGroup, VerifiedShardGroupVersions> {
        self.shard_group_versions.lock().unwrap().clone()
    }

    fn cache_shard_group_versions(&self, versions: HashMap<ShardGroup, VerifiedShardGroupVersions>) {
        *self.shard_group_versions.lock().unwrap() = versions;
    }

    fn clear_shard_group_versions(&self) {
        self.shard_group_versions.lock().unwrap().clear();
    }

    fn cached_committee(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
        validator_set_hash: &ValidatorSetHash,
    ) -> Option<VerifiedCommittee> {
        self.committees
            .lock()
            .unwrap()
            .get(&(epoch, shard_group))
            .filter(|committee| committee.validator_set_hash() == validator_set_hash)
            .cloned()
    }

    fn cache_committee(&self, committee: VerifiedCommittee) {
        let mut committees = self.committees.lock().unwrap();
        if committees.len() >= MAX_CACHED_COMMITTEES {
            committees.clear();
        }
        committees.insert((committee.epoch(), committee.shard_group()), committee);
    }
}

/// Keeps the wallet's trust in the network's validator sets: it anchors on a validator set, walks the end-of-epoch
/// handovers from it, and verifies committees against the sets it trusts.
///
/// The wallet keeps the latest trusted set, and the hash of every set from its anchor on, so that proofs anchored in
/// earlier epochs stay verifiable.
pub struct ValidatorSetApi<'a, TStore, TNetworkInterface> {
    store: &'a TStore,
    network_interface: &'a TNetworkInterface,
    network: Network,
    shared: &'a ValidatorSetShared,
}

impl<'a, TStore, TNetworkInterface> ValidatorSetApi<'a, TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
{
    pub fn new(
        store: &'a TStore,
        network_interface: &'a TNetworkInterface,
        network: Network,
        shared: &'a ValidatorSetShared,
    ) -> Self {
        Self {
            store,
            network_interface,
            network,
            shared,
        }
    }

    /// The latest validator set the wallet trusts, or `None` before it first anchors.
    pub fn latest(&self) -> Result<Option<TrustedValidatorSet>, ValidatorSetApiError> {
        let latest = self.store.with_read_tx(|tx| {
            tx.config_get::<TrustedValidatorSet>(ConfigKey::TrustedValidatorSet.as_key_str())
                .optional()
        })?;
        Ok(latest.map(|config| config.value))
    }

    /// The validator set the wallet's trusted sets descend from, or `None` before it first anchors.
    pub fn anchor(&self) -> Result<Option<AnchoredValidatorSet>, ValidatorSetApiError> {
        Ok(self.store.with_read_tx(|tx| tx.trusted_validator_sets_get_anchor())?)
    }

    /// The wallet's latest `limit` re-anchors, newest first. Each one trusted its indexer for the set it anchored on.
    pub fn reanchors(&self, limit: u64) -> Result<Vec<ValidatorSetReanchor>, ValidatorSetApiError> {
        Ok(self
            .store
            .with_read_tx(|tx| tx.validator_set_reanchors_get_latest(limit))?)
    }

    /// Where the wallet's trust in the network's validator sets stands, or `None` before it first anchors.
    pub fn status(&self) -> Result<Option<ValidatorSetStatus>, ValidatorSetApiError> {
        let status = self.store.with_read_tx(|tx| {
            let Some(latest) = tx
                .config_get::<TrustedValidatorSet>(ConfigKey::TrustedValidatorSet.as_key_str())
                .optional()?
            else {
                return Ok::<_, WalletStorageError>(None);
            };
            let Some(anchor) = tx.trusted_validator_sets_get_anchor()? else {
                return Ok(None);
            };
            let synced_at = tx
                .config_get::<u64>(ConfigKey::ValidatorSetSyncedAt.as_key_str())
                .optional()?
                .map(|config| config.value);
            let walk_failing_since = tx
                .config_get::<Option<u64>>(ConfigKey::ValidatorSetWalkFailingSince.as_key_str())
                .optional()?
                .and_then(|config| config.value);
            Ok(Some(ValidatorSetStatus {
                latest: latest.value.trusted_hash(),
                anchor,
                synced_at,
                walk_failing_since,
            }))
        })?;
        Ok(status)
    }

    /// Brings the trusted validator set up to the one the latest checkpoint names. On first use the wallet anchors on
    /// that set. Later it walks the handovers to it, however far behind it is.
    ///
    /// Before the network's first end-of-epoch checkpoint, the wallet anchors on the current epoch's validator set
    /// itself (see [`Self::anchor_on_current_epoch`]), and an anchored wallet is up to date.
    ///
    /// When the walk fails because the indexer's checkpoints do not connect to the trusted set, the wallet re-anchors
    /// on the set the latest checkpoint names in place of the walk, but only once its own clock vouches for the need:
    /// - the walk has failed on every sync for [`REANCHOR_AFTER_FAILING_FOR`], or
    /// - the gap is more than [`DEFAULT_MAX_WALK_EPOCHS`], and the wallet has been out of sync long enough to fall that
    ///   far behind.
    ///
    /// A wallet that synced recently never re-anchors on a claimed gap. A walk that keeps failing for a day re-anchors
    /// however recently the wallet synced, so an indexer that keeps the walk failing that long chooses the new anchor,
    /// as it does on first use. A walk that fails keeps the sets verified before the failure.
    pub async fn sync(&self) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
        let _guard = self.shared.lock.lock().await;
        let Some(latest_checkpoint) = self.latest_checkpoint().await? else {
            return match self.latest()? {
                Some(trusted) => {
                    self.store.with_write_tx(|tx| record_synced(tx))?;
                    Ok(trusted)
                },
                None => self.anchor_on_current_epoch(ValidatorSetAnchor::FirstUse).await,
            };
        };
        let Some(trusted) = self.latest()? else {
            info!(target: LOG_TARGET, "Anchoring on the validator set that the checkpoint of {} names", latest_checkpoint.epoch);
            return self.anchor_on(&latest_checkpoint, ValidatorSetAnchor::FirstUse);
        };
        let plan = plan_walk(&trusted, latest_checkpoint.epoch, DEFAULT_MAX_WALK_EPOCHS);
        let synced = match plan {
            WalkPlan::UpToDate => trusted,
            WalkPlan::Walk { .. } | WalkPlan::Reanchor => {
                match self.walk(trusted.clone(), latest_checkpoint.epoch).await {
                    Ok(walked) => walked,
                    Err(err) if err.is_unwalkable_history() => {
                        let failing_for = self.walk_failing_for()?;
                        let fell_behind_offline = matches!(plan, WalkPlan::Reanchor) &&
                            self.out_of_sync_long_enough(&trusted, &latest_checkpoint)?;
                        if failing_for < REANCHOR_AFTER_FAILING_FOR && !fell_behind_offline {
                            return Err(err);
                        }
                        warn!(
                            target: LOG_TARGET,
                            "The walk from the trusted validator set of {} to the checkpoint of {} failed ({err}), and has failed for {}s. Re-anchoring on the set it names.",
                            trusted.epoch(),
                            latest_checkpoint.epoch,
                            failing_for.as_secs()
                        );
                        return self.anchor_on(&latest_checkpoint, ValidatorSetAnchor::Reanchor);
                    },
                    Err(err) => return Err(err),
                }
            },
        };
        self.store.with_write_tx(|tx| record_synced(tx))?;
        Ok(synced)
    }

    /// Anchors anew on the validator set the latest checkpoint names, or on the consensus epoch's set before the
    /// network's first checkpoint, forgetting every set trusted before. This trusts the indexer as first use does, and
    /// is how the wallet recovers when the network's checkpoints do not walk from its anchor.
    pub async fn reanchor(&self) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
        let _guard = self.shared.lock.lock().await;
        let Some(latest_checkpoint) = self.latest_checkpoint().await? else {
            return self.anchor_on_current_epoch(ValidatorSetAnchor::Reanchor).await;
        };
        warn!(target: LOG_TARGET, "Re-anchoring on the validator set that the checkpoint of {} names", latest_checkpoint.epoch);
        self.anchor_on(&latest_checkpoint, ValidatorSetAnchor::Reanchor)
    }

    /// The committee of `shard_group` in `epoch`, verified against the validator set the wallet trusts for `epoch`.
    /// Walks forward first if `epoch` is past the latest trusted set.
    ///
    /// Returns `None` when the wallet trusts no set for `epoch`: one before its anchor, or one that no checkpoint names
    /// yet.
    pub async fn verify_committee(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Option<VerifiedCommittee>, ValidatorSetApiError> {
        let mut latest = self.latest()?;
        if latest.as_ref().is_none_or(|latest| latest.epoch() < epoch) {
            latest = Some(self.sync().await?);
        }
        if let Some(latest) = latest.filter(|latest| latest.epoch() == epoch) &&
            latest.walk_committee().shard_group() == shard_group
        {
            return Ok(Some(latest.walk_committee().clone()));
        }

        let Some(validator_set) = self
            .store
            .with_read_tx(|tx| tx.trusted_validator_sets_get(epoch).optional())?
        else {
            return Ok(None);
        };
        if let Some(committee) = self
            .shared
            .cached_committee(epoch, shard_group, validator_set.validator_set_hash())
        {
            return Ok(Some(committee));
        }
        let committee = self
            .network_interface
            .get_committee(epoch, shard_group.start())
            .await
            .map_err(network_error)?;
        let committee = validator_set.verify_committee(&committee)?;
        self.shared.cache_committee(committee.clone());
        Ok(Some(committee))
    }

    /// The state version every shard reached at the end of the latest epoch the indexer serves a checkpoint of for its
    /// shard group, and that epoch. Each version is read from an end-of-epoch checkpoint that the committee of the
    /// shard's group signed, verified against the validator set the wallet trusts for that epoch, so none is ahead of
    /// what the network committed.
    ///
    /// Empty before the network has ended an epoch. Each call asks the indexer for the latest checkpoint of every shard
    /// group, and verifies only those of a group whose latest checkpoint ends another epoch than the one last verified.
    pub async fn verified_shard_state_versions(
        &self,
    ) -> Result<HashMap<Shard, CheckpointStateVersion>, ValidatorSetApiError> {
        let trusted = match self.latest()? {
            Some(trusted) => trusted,
            None => self.sync().await?,
        };
        let entries = self
            .network_interface
            .get_latest_shard_group_checkpoints()
            .await
            .map_err(network_error)?;
        if entries.is_empty() {
            return Ok(HashMap::new());
        }
        check_each_shard_held_once(trusted.num_preshards(), &entries)?;

        let cached = self.shared.cached_shard_group_versions();
        let mut verified = HashMap::with_capacity(entries.len());
        for entry in &entries {
            let group_versions = match cached
                .get(&entry.shard_group)
                .filter(|cached| cached.epoch == entry.epoch)
            {
                Some(cached) => cached.clone(),
                None => self.verify_shard_group_versions(entry).await?,
            };
            verified.insert(entry.shard_group, group_versions);
        }
        let versions = verified
            .values()
            .flat_map(|group| {
                group.versions.iter().map(|&(shard, state_version)| {
                    (shard, CheckpointStateVersion {
                        epoch: group.epoch,
                        state_version,
                    })
                })
            })
            .collect();
        self.shared.cache_shard_group_versions(verified);
        Ok(versions)
    }

    /// The state versions that `entry`, the latest checkpoint the indexer lists for its shard group, commits, once it
    /// verifies against the committee of that group in its epoch.
    async fn verify_shard_group_versions(
        &self,
        entry: &EpochCheckpointEntry,
    ) -> Result<VerifiedShardGroupVersions, ValidatorSetApiError> {
        let committee = self
            .verify_committee(entry.epoch, entry.shard_group)
            .await?
            .ok_or(ValidatorSetApiError::NoTrustedValidatorSet { epoch: entry.epoch })?;
        let checkpoint = committee.verify_checkpoint(entry)?;
        if checkpoint.shard_group() != entry.shard_group {
            return Err(ValidatorSetApiError::CheckpointOfAnotherShardGroup {
                listed: entry.shard_group,
                shard_group: checkpoint.shard_group(),
            });
        }
        let versions = checkpoint
            .shard_group()
            .shard_iter()
            .filter_map(|shard| Some((shard, checkpoint.shard_state_version(shard)?)))
            .collect();
        Ok(VerifiedShardGroupVersions {
            epoch: checkpoint.epoch(),
            versions,
        })
    }

    /// The latest checkpoint of the walked shard group, or `None` before the network has ended an epoch.
    async fn latest_checkpoint(&self) -> Result<Option<EpochCheckpointEntry>, ValidatorSetApiError> {
        self.network_interface
            .get_latest_epoch_checkpoint(WALK_SHARD)
            .await
            .optional()
            .map_err(network_error)
    }

    fn anchor_on(
        &self,
        checkpoint: &EpochCheckpointEntry,
        anchor: ValidatorSetAnchor,
    ) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
        let trusted = TrustedValidatorSet::anchor_on_first_use(self.network, WALK_SHARD, checkpoint)?;
        self.store_anchor(&trusted, anchor)?;
        Ok(trusted)
    }

    /// Anchors on the validator set of the epoch the network's consensus is in, with the walked shard group's committee
    /// in it, for a network that has not ended an epoch yet and so has no checkpoint to anchor on.
    ///
    /// The epoch is the one the indexer observed the validators execute in, so that every receipt proven after the
    /// anchor is proven in an epoch whose validator set the wallet trusts.
    ///
    /// This takes the indexer at its word on the validator set, as anchoring on a first-use checkpoint does, whose
    /// quorum certificate no trusted committee can check either. The first end-of-epoch checkpoint must carry a quorum
    /// of the anchored committee's signatures for the walk to pass it.
    async fn anchor_on_current_epoch(
        &self,
        anchor: ValidatorSetAnchor,
    ) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
        let epoch = self
            .network_interface
            .get_consensus_epoch()
            .await
            .map_err(network_error)?
            .ok_or(ValidatorSetApiError::ConsensusEpochUnobserved)?;
        let committee = self
            .network_interface
            .get_committee(epoch, WALK_SHARD)
            .await
            .map_err(network_error)?;
        info!(target: LOG_TARGET, "The network has no checkpoint yet. Anchoring on the validator set of {epoch}");
        let trusted = TrustedValidatorSet::anchor_pinned(
            self.network,
            WALK_SHARD,
            epoch,
            &committee.validator_set_hash,
            &committee,
        )?;
        self.store_anchor(&trusted, anchor)?;
        Ok(trusted)
    }

    fn store_anchor(
        &self,
        trusted: &TrustedValidatorSet,
        anchor: ValidatorSetAnchor,
    ) -> Result<(), ValidatorSetApiError> {
        self.store.with_write_tx(|tx| {
            if anchor == ValidatorSetAnchor::Reanchor {
                let previous = tx
                    .config_get::<TrustedValidatorSet>(ConfigKey::TrustedValidatorSet.as_key_str())
                    .optional()?;
                tx.append_wallet_event(&WalletEvent::ValidatorSetReanchored(ValidatorSetReanchoredEvent {
                    previous_epoch: previous.map(|previous| previous.value.epoch()),
                    epoch: trusted.epoch(),
                    validator_set_hash: *trusted.validator_set_hash(),
                }))?;
            }
            tx.trusted_validator_sets_clear()?;
            tx.trusted_validator_sets_insert(&trusted.trusted_hash(), Some(anchor))?;
            tx.config_set(ConfigKey::TrustedValidatorSet.as_key_str(), trusted, false)?;
            record_synced(tx)
        })?;
        self.shared.clear_shard_group_versions();
        Ok(())
    }

    /// How long the walk has failed on every sync, counting this failure, which starts the count when it is the first.
    fn walk_failing_for(&self) -> Result<Duration, ValidatorSetApiError> {
        let now = unix_now();
        let failing_since = self.store.with_write_tx(|tx| {
            let key = ConfigKey::ValidatorSetWalkFailingSince.as_key_str();
            match tx
                .config_get::<Option<u64>>(key)
                .optional()?
                .and_then(|since| since.value)
            {
                Some(since) => Ok::<_, WalletStorageError>(since),
                None => {
                    tx.config_set(key, &Some(now), false)?;
                    Ok(now)
                },
            }
        })?;
        Ok(Duration::from_secs(now.saturating_sub(failing_since)))
    }

    /// Whether the wallet's clock allows that it fell from `trusted` to `latest_checkpoint` behind: every epoch of the
    /// gap takes at least [`MIN_EPOCH_DURATION`], and the wallet last synced long enough ago for all of them.
    fn out_of_sync_long_enough(
        &self,
        trusted: &TrustedValidatorSet,
        latest_checkpoint: &EpochCheckpointEntry,
    ) -> Result<bool, ValidatorSetApiError> {
        let Some(synced_at) = self.store.with_read_tx(|tx| {
            tx.config_get::<u64>(ConfigKey::ValidatorSetSyncedAt.as_key_str())
                .optional()
        })?
        else {
            return Ok(true);
        };
        let out_of_sync = Duration::from_secs(unix_now().saturating_sub(synced_at.value));
        let gap = latest_checkpoint
            .epoch
            .as_u64()
            .saturating_sub(trusted.epoch().as_u64()) +
            1;
        let gap_takes_at_least = MIN_EPOCH_DURATION.saturating_mul(u32::try_from(gap).unwrap_or(u32::MAX));
        Ok(out_of_sync >= gap_takes_at_least)
    }

    /// Walks the handovers from `trusted` to the validator set of the epoch after `to_epoch`.
    async fn walk(
        &self,
        mut trusted: TrustedValidatorSet,
        to_epoch: Epoch,
    ) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
        while trusted.epoch() <= to_epoch {
            let checkpoints = self
                .network_interface
                .list_epoch_checkpoints(trusted.epoch(), CHECKPOINTS_PER_REQUEST, WALK_SHARD)
                .await
                .map_err(network_error)?;
            let mut walked = Vec::with_capacity(checkpoints.len());
            for checkpoint in &checkpoints {
                // A checkpoint past the trusted set's epoch follows one the indexer has not served yet.
                if checkpoint.epoch > to_epoch || checkpoint.epoch > trusted.epoch() {
                    break;
                }
                match trusted.advance(checkpoint) {
                    Ok(next) => {
                        trusted = next;
                        walked.push(trusted.trusted_hash());
                    },
                    Err(err) => {
                        self.store_walked(&trusted, &walked)?;
                        return Err(err.into());
                    },
                }
            }
            if walked.is_empty() {
                return Err(ValidatorSetApiError::CheckpointNotServed { epoch: trusted.epoch() });
            }
            self.store_walked(&trusted, &walked)?;
        }
        Ok(trusted)
    }

    fn store_walked(
        &self,
        latest: &TrustedValidatorSet,
        walked: &[TrustedValidatorSetHash],
    ) -> Result<(), ValidatorSetApiError> {
        if walked.is_empty() {
            return Ok(());
        }
        self.store.with_write_tx(|tx| {
            for validator_set in walked {
                tx.trusted_validator_sets_insert(validator_set, None)?;
            }
            let stored = tx
                .config_get::<TrustedValidatorSet>(ConfigKey::TrustedValidatorSet.as_key_str())
                .optional()?;
            if stored.is_none_or(|stored| stored.value.epoch() < latest.epoch()) {
                tx.config_set(ConfigKey::TrustedValidatorSet.as_key_str(), latest, false)?;
            }
            Ok::<_, WalletStorageError>(())
        })?;
        Ok(())
    }
}

/// Records that the trusted validator set is up to date as of now.
fn record_synced<TTx: WalletStoreWriter>(tx: &mut TTx) -> Result<(), WalletStorageError> {
    tx.config_set(ConfigKey::ValidatorSetSyncedAt.as_key_str(), &unix_now(), false)?;
    tx.config_set(
        ConfigKey::ValidatorSetWalkFailingSince.as_key_str(),
        &None::<u64>,
        false,
    )
}

/// Checks that the shard groups of `entries` hold every shard of the network once, and no shard it does not have.
fn check_each_shard_held_once(
    num_preshards: NumPreshards,
    entries: &[EpochCheckpointEntry],
) -> Result<(), ValidatorSetApiError> {
    let all_shards = ShardGroup::all_shards(num_preshards);
    if let Some(entry) = entries.iter().find(|entry| {
        !all_shards.contains(&entry.shard_group.start()) || !all_shards.contains(&entry.shard_group.end())
    }) {
        return Err(ValidatorSetApiError::CheckpointOfShardsOutsideNetwork {
            shard_group: entry.shard_group,
        });
    }
    for shard in num_preshards.all_shards_iter() {
        match entries
            .iter()
            .filter(|entry| entry.shard_group.contains(&shard))
            .count()
        {
            0 => return Err(ValidatorSetApiError::ShardWithoutCheckpoint { shard }),
            1 => {},
            _ => return Err(ValidatorSetApiError::ShardInTwoCheckpoints { shard }),
        }
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

fn network_error<E: std::error::Error + Send + Sync + 'static>(err: E) -> ValidatorSetApiError {
    ValidatorSetApiError::NetworkInterfaceError(anyhow::Error::new(err))
}

#[derive(Debug, thiserror::Error)]
pub enum ValidatorSetApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Network interface error: {0}")]
    NetworkInterfaceError(anyhow::Error),
    #[error("The indexer's validator set data does not verify: {0}")]
    Unverified(#[from] LightClientError),
    #[error("The indexer served no checkpoint ending {epoch} to walk the validator set handover through")]
    CheckpointNotServed { epoch: Epoch },
    #[error(
        "The network has no checkpoint yet, and the indexer has not observed which epoch the validators are in, so \
         the wallet has no validator set to anchor on. It retries shortly."
    )]
    ConsensusEpochUnobserved,
    #[error("The indexer served a checkpoint ending {epoch}, but the wallet trusts no validator set for that epoch")]
    NoTrustedValidatorSet { epoch: Epoch },
    #[error(
        "The indexer served a checkpoint of shard group {shard_group} as the latest checkpoint of shard group {listed}"
    )]
    CheckpointOfAnotherShardGroup {
        listed: ShardGroup,
        shard_group: ShardGroup,
    },
    #[error("The indexer listed the latest checkpoint of no shard group holding shard {shard}")]
    ShardWithoutCheckpoint { shard: Shard },
    #[error("The indexer listed the latest checkpoints of two shard groups that both hold shard {shard}")]
    ShardInTwoCheckpoints { shard: Shard },
    #[error(
        "The indexer listed a latest checkpoint of shard group {shard_group}, which holds shards the network does not \
         have"
    )]
    CheckpointOfShardsOutsideNetwork { shard_group: ShardGroup },
}

impl ValidatorSetApiError {
    /// Whether the error shows that the indexer's checkpoints do not walk from the trusted set, as opposed to a failure
    /// to reach the indexer.
    fn is_unwalkable_history(&self) -> bool {
        matches!(self, Self::Unverified(_) | Self::CheckpointNotServed { .. })
    }
}

impl IsNotFoundError for ValidatorSetApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error())
    }
}
