//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::*;
use ootle_network::Network;
use tari_indexer_client::types::EpochCheckpointEntry;
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
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
    models::{AnchoredValidatorSet, ValidatorSetAnchor, ValidatorSetReanchoredEvent, ValidatorSetStatus, WalletEvent},
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

/// Keeps the wallet's trust in the network's validator sets: it anchors on a validator set, walks the end-of-epoch
/// handovers from it, and verifies committees against the sets it trusts.
///
/// The wallet keeps the latest trusted set, and the hash of every set from its anchor on, so that proofs anchored in
/// earlier epochs stay verifiable.
pub struct ValidatorSetApi<'a, TStore, TNetworkInterface> {
    store: &'a TStore,
    network_interface: &'a TNetworkInterface,
    network: Network,
    /// Held while walking or re-anchoring, so that one change to the trusted sets completes before the next starts.
    lock: &'a tokio::sync::Mutex<()>,
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
        lock: &'a tokio::sync::Mutex<()>,
    ) -> Self {
        Self {
            store,
            network_interface,
            network,
            lock,
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
        let _guard = self.lock.lock().await;
        let latest_checkpoint = self.latest_checkpoint().await?;
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

    /// Anchors anew on the validator set the latest checkpoint names, forgetting every set trusted before. This trusts
    /// the indexer as first use does, and is how the wallet recovers when the network's checkpoints do not walk from
    /// its anchor.
    pub async fn reanchor(&self) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
        let _guard = self.lock.lock().await;
        let latest_checkpoint = self.latest_checkpoint().await?;
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
        let committee = self
            .network_interface
            .get_committee(epoch, shard_group.start())
            .await
            .map_err(network_error)?;
        Ok(Some(validator_set.verify_committee(&committee)?))
    }

    async fn latest_checkpoint(&self) -> Result<EpochCheckpointEntry, ValidatorSetApiError> {
        self.network_interface
            .get_latest_epoch_checkpoint(WALK_SHARD)
            .await
            .map_err(network_error)
    }

    fn anchor_on(
        &self,
        checkpoint: &EpochCheckpointEntry,
        anchor: ValidatorSetAnchor,
    ) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
        let trusted = TrustedValidatorSet::anchor_on_first_use(self.network, WALK_SHARD, checkpoint)?;
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
            tx.config_set(ConfigKey::TrustedValidatorSet.as_key_str(), &trusted, false)?;
            record_synced(tx)
        })?;
        Ok(trusted)
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
            for checkpoint in checkpoints.iter().take_while(|checkpoint| checkpoint.epoch <= to_epoch) {
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
