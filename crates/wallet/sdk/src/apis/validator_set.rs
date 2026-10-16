//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

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
    models::{AnchoredValidatorSet, ValidatorSetAnchor},
    network::WalletNetworkInterface,
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

const LOG_TARGET: &str = "tari::ootle::wallet_sdk::apis::validator_set";

/// The shard whose shard group the wallet walks the validator set handover through. Every shard is in some shard group
/// in every epoch.
pub const WALK_SHARD: Shard = Shard::first();

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
}

impl<'a, TStore, TNetworkInterface> ValidatorSetApi<'a, TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
{
    pub fn new(store: &'a TStore, network_interface: &'a TNetworkInterface, network: Network) -> Self {
        Self {
            store,
            network_interface,
            network,
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

    /// Brings the trusted validator set up to the one the latest checkpoint names. On first use the wallet anchors on
    /// that set. Later it walks the handovers to it, or re-anchors on it when the trusted set is more than
    /// [`DEFAULT_MAX_WALK_EPOCHS`] behind.
    ///
    /// A walk that fails keeps the sets verified before the failure.
    pub async fn sync(&self) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
        let latest_checkpoint = self.latest_checkpoint().await?;
        let Some(trusted) = self.latest()? else {
            info!(target: LOG_TARGET, "Anchoring on the validator set that the checkpoint of {} names", latest_checkpoint.epoch);
            return self.anchor_on(&latest_checkpoint, ValidatorSetAnchor::FirstUse);
        };
        match plan_walk(&trusted, latest_checkpoint.epoch, DEFAULT_MAX_WALK_EPOCHS) {
            WalkPlan::UpToDate => Ok(trusted),
            WalkPlan::Walk { to_epoch, .. } => self.walk(trusted, to_epoch).await,
            WalkPlan::Reanchor => {
                warn!(
                    target: LOG_TARGET,
                    "The trusted validator set of {} is too far behind the checkpoint of {} to walk. Re-anchoring on the set it names.",
                    trusted.epoch(),
                    latest_checkpoint.epoch
                );
                self.anchor_on(&latest_checkpoint, ValidatorSetAnchor::Reanchor)
            },
        }
    }

    /// Anchors anew on the validator set the latest checkpoint names, forgetting every set trusted before. This trusts
    /// the indexer as first use does, and is how the wallet recovers when the network's checkpoints do not walk from
    /// its anchor.
    pub async fn reanchor(&self) -> Result<TrustedValidatorSet, ValidatorSetApiError> {
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
        if let Some(latest) = latest.filter(|latest| latest.epoch() == epoch) {
            if latest.walk_committee().shard_group() == shard_group {
                return Ok(Some(latest.walk_committee().clone()));
            }
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
            tx.trusted_validator_sets_clear()?;
            tx.trusted_validator_sets_insert(&trusted.trusted_hash(), Some(anchor))?;
            tx.config_set(ConfigKey::TrustedValidatorSet.as_key_str(), &trusted, false)
        })?;
        Ok(trusted)
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

impl IsNotFoundError for ValidatorSetApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error())
    }
}
