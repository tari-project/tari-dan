//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use futures::StreamExt;
use log::{debug, info, trace, warn};
use ootle_byte_type::FromByteType;
use tari_ootle_common_types::{NumPreshards, StateVersion, optional::Optional, shard::Shard};
use tari_ootle_transaction::Network;
use tari_ootle_wallet_sdk::{
    NetworkInterfaceError,
    WalletSdk,
    WalletSdkSpec,
    apis::validator_set::ValidatorSetApiError,
    models::{
        AccountWithAddress,
        CheckpointStateVersion,
        ClaimedStateVersion,
        OutputStatus,
        StartOfShard,
        UtxoSpent,
        UtxoSpentEvent,
        UtxoUnspent,
        WalletEvent,
        WalletSecretKey,
        WalletUtxoUpdate,
    },
    network::{UtxoUpdateStream, WalletNetworkInterface},
    storage::{ReadableWalletStore, WalletStorageError, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_template_lib_types::{ComponentAddress, ResourceAddress, UtxoAddress};

use crate::{notify::Notify, utxo_scanner::StealthScannerApiError};

const LOG_TARGET: &str = "tari::ootle::wallet_services::scanner_round";
// TODO: either fetch num preshards from the network or we should hardcode it to a single value for all apps
const NUM_PRESHARDS: NumPreshards = NumPreshards::P256;

/// A claim is checked against a verified checkpoint at least this many epochs after the latest one the wallet had
/// verified when it stored the claim. The network may have ended one more epoch than the wallet had verified by then,
/// so the checkpoint after that is the first that certainly ended after the claim, and commits every update an honest
/// claim covers.
const CLAIM_CHECK_EPOCHS: u64 = 2;

pub struct UtxoScannerRound<'a, TSpec: WalletSdkSpec> {
    network: Network,
    account: &'a AccountWithAddress,
    view_key: &'a WalletSecretKey,
    resource_address: &'a ResourceAddress,

    sdk: &'a WalletSdk<TSpec>,
    stats: UtxoScanRoundStats,

    /// The cursor each shard's next batch is requested from. It follows the indexer's answers within the round, and is
    /// stored only as far as `stored_cursor_bound` allows, with any progress past it stored as a claim.
    cursors: HashMap<Shard, StateVersion>,
    stored_cursor_bound: StoredCursorBound,
    /// The stored claims that no verified checkpoint refutes.
    claims: HashMap<Shard, ClaimedStateVersion>,
    shard_state_versions_to_set: HashMap<Shard, StateVersion>,
    utxos_to_recover: Vec<(ComponentAddress, UtxoUnspent)>,
    utxos_to_spend: Vec<UtxoSpent>,
    notify: &'a Notify<WalletEvent>,
}

impl<'a, TSpec> UtxoScannerRound<'a, TSpec>
where TSpec: WalletSdkSpec
{
    pub fn new(
        network: Network,
        sdk: &'a WalletSdk<TSpec>,
        account: &'a AccountWithAddress,
        view_key: &'a WalletSecretKey,
        resource_address: &'a ResourceAddress,
        notify: &'a Notify<WalletEvent>,
    ) -> Self {
        Self {
            network,
            sdk,
            account,
            view_key,
            resource_address,
            cursors: HashMap::new(),
            stored_cursor_bound: StoredCursorBound::Unbounded,
            claims: HashMap::new(),
            shard_state_versions_to_set: HashMap::new(),
            utxos_to_recover: Vec::new(),
            utxos_to_spend: Vec::new(),
            notify,
            stats: UtxoScanRoundStats::default(),
        }
    }

    pub async fn scan_for_utxo_updates(&mut self) -> Result<(), StealthScannerApiError> {
        self.stored_cursor_bound = self.load_stored_cursor_bound().await?;
        self.load_cursors()?;
        loop {
            if !self.process_next_batch().await? {
                break;
            }
        }

        Ok(())
    }

    pub fn into_stats(self) -> UtxoScanRoundStats {
        self.stats
    }

    /// Under [`IndexerTrust::Verify`](tari_ootle_wallet_sdk::IndexerTrust::Verify) the stored cursors are bounded by
    /// the shard state versions of the latest verified end-of-epoch checkpoints. Without them the stored cursors stay
    /// where they are, and this round stores its progress only as claims.
    async fn load_stored_cursor_bound(&self) -> Result<StoredCursorBound, StealthScannerApiError> {
        if !self.sdk.sdk_config().indexer_trust.is_verify() {
            return Ok(StoredCursorBound::Unbounded);
        }
        match self.sdk.validator_set_api().verified_shard_state_versions().await {
            Ok(versions) => Ok(StoredCursorBound::Verified(versions)),
            Err(ValidatorSetApiError::StoreError(err)) => Err(err.into()),
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ No verified shard state versions to bound the stealth scan cursors by: {err}. The scan stores its progress only as claims."
                );
                Ok(StoredCursorBound::Verified(HashMap::new()))
            },
        }
    }

    /// Loads the stored cursors, lowering any that are past the bound to it, and resumes each shard from its claim
    /// unless a verified checkpoint refutes it.
    fn load_cursors(&mut self) -> Result<(), StealthScannerApiError> {
        let account = self.account.component_address();
        let (stored, claims) = self.sdk.store().with_read_tx(|tx| {
            let stored = tx.shard_state_version_get(account, self.resource_address)?;
            let claims = if self.stored_cursor_bound.is_verified() {
                tx.shard_state_version_claims_get(account, self.resource_address)?
            } else {
                HashMap::new()
            };
            Ok::<_, WalletStorageError>((stored, claims))
        })?;

        let mut lowered = Vec::new();
        let mut refuted = Vec::new();
        for shard in NUM_PRESHARDS.all_shards_iter() {
            let stored = stored.get(&shard).copied().unwrap_or_else(StateVersion::zero);
            let mut cursor = match self.stored_cursor_bound.ceiling(shard) {
                Some(ceiling) if stored > ceiling => {
                    lowered.push((shard, ceiling));
                    ceiling
                },
                _ => stored,
            };
            if let Some(claim) = claims.get(&shard) {
                if self.stored_cursor_bound.refutes(shard, claim) {
                    refuted.push(shard);
                } else {
                    cursor = cursor.max(claim.state_version);
                    self.claims.insert(shard, *claim);
                }
            }
            self.cursors.insert(shard, cursor);
        }

        if !refuted.is_empty() {
            warn!(
                target: LOG_TARGET,
                "⚠️ {} stealth scan cursor(s) of account {} (resource {}) that an indexer claimed are past the state version a later verified checkpoint commits. Rescanning those shards from the verified cursor.",
                refuted.len(),
                account,
                self.resource_address,
            );
            self.sdk
                .store()
                .with_write_tx(|tx| tx.shard_state_version_claims_remove(account, self.resource_address, refuted))?;
        }

        if !lowered.is_empty() {
            warn!(
                target: LOG_TARGET,
                "⚠️ {} stored stealth scan cursor(s) of account {} (resource {}) were past the verified shard state version. Rescanning those shards from it.",
                lowered.len(),
                self.account.component_address(),
                self.resource_address,
            );
            self.sdk.store().with_write_tx(|tx| {
                tx.shard_state_version_set_many(self.account.component_address(), self.resource_address, lowered)
            })?;
        }
        Ok(())
    }

    async fn process_next_batch(&mut self) -> Result<bool, StealthScannerApiError> {
        info!(
            target: LOG_TARGET,
            "🔍️ Scanning for stealth outputs (account {}, resource {}, num shards {})",
            self.account.component_address(),
            self.resource_address,
            self.cursors.len(),
        );

        let live_output_count = self
            .sdk
            .stealth_outputs_api()
            .count_unspent_or_locked_outputs_for_account(self.account.component_address(), self.resource_address)?;

        let stream = self
            .sdk
            .get_network_interface()
            .stream_stealth_utxo_updates(
                self.account.birthday_epoch(),
                *self.resource_address,
                // NOTE that this will request shards in a random order (HashMap). This is good to avoid always
                // starting with the same shard.
                self.cursors.iter().map(|(shard, cursor)| (*shard, *cursor)).collect(),
                // Optimisation: with no unspent or locked outputs there is no spend left for the wallet to
                // learn of, so only unspent updates are needed. The indexer moves the cursor past the spent
                // updates this skips, so a later pass from that cursor does not deliver them either.
                live_output_count == 0,
            )
            .await
            .map_err(|e| StealthScannerApiError::NetworkInterfaceError(e.into()))?;

        let has_more = self.process_stream(stream).await?;
        Ok(has_more)
    }

    async fn process_stream(
        &mut self,
        mut stream: UtxoUpdateStream<NetworkInterfaceError<TSpec>>,
    ) -> Result<bool, StealthScannerApiError> {
        let mut num_received = 0usize;
        let mut sos: Option<StartOfShard> = None;
        let mut has_more = false;

        while let Some(result) = stream.next().await {
            let update = result.map_err(|e| StealthScannerApiError::NetworkInterfaceError(e.into()))?;
            if let Some(recv_sos) = update.sos {
                debug!(target: LOG_TARGET, "Received StartOfShard for {}", recv_sos.shard);
                // Commit the previous progress and start with the next shard
                if let Some(sos) = sos.take() {
                    if num_received > 0 {
                        debug!(
                            target: LOG_TARGET,
                            "🔍️ Scan complete for account {}: No more stealth outputs found in shard {} (max state version {})",
                            self.account.component_address(),
                            sos.shard,
                            sos.max_state_version
                        );
                    }
                    self.stats.num_received += num_received;
                    num_received = 0;
                    self.commit_progress()?;
                }

                for versions in [&mut self.cursors, &mut self.shard_state_versions_to_set] {
                    versions
                        .entry(recv_sos.shard)
                        .and_modify(|v| {
                            if recv_sos.max_state_version > *v {
                                *v = recv_sos.max_state_version
                            }
                        })
                        .or_insert(recv_sos.max_state_version);
                }

                has_more |= recv_sos.has_more;

                sos = Some(recv_sos);
            }

            if let Some(update) = update.update {
                num_received += 1;
                match update {
                    WalletUtxoUpdate::Unspent(unspent) => {
                        if self.check_if_tag_matches(&unspent)? {
                            debug!(
                                target: LOG_TARGET,
                                "🏷️ Stealth output tag {} matches. Queueing for recovery.",
                                unspent.tag
                            );
                            self.utxos_to_recover.push((*self.account.component_address(), unspent));
                        }
                    },
                    WalletUtxoUpdate::Spent(spent) => {
                        self.utxos_to_spend.push(spent);
                    },
                    WalletUtxoUpdate::Burnt(burnt) => {
                        // NOTE: we treat burnt outputs the same as spent for now. In future, we may want to track them
                        // separately
                        self.utxos_to_spend.push(UtxoSpent {
                            id: burnt.id,
                            version: burnt.version,
                        });
                    },
                }
            }
        }

        self.commit_progress()?;
        self.stats.num_received += num_received;

        if !has_more {
            info!(
                target: LOG_TARGET,
                "🔍️ Scan complete for account {}: No more stealth outputs found",
                self.account.component_address()
            );
        }

        Ok(has_more)
    }

    fn commit_progress(&mut self) -> Result<bool, StealthScannerApiError> {
        if self.utxos_to_recover.is_empty() &&
            self.utxos_to_spend.is_empty() &&
            self.shard_state_versions_to_set.is_empty()
        {
            return Ok(false);
        }

        let mut num_spent = 0;
        let num_recovered = self.utxos_to_recover.len();
        self.stats.num_potential_recoveries += num_recovered;
        self.sdk.store().with_write_tx(|tx| {
            // Mark UTXOs as spent (if they exist)
            for spent in self.utxos_to_spend.drain(..) {
                if Self::spend(tx, self.resource_address, &spent)? {
                    self.notify.notify(UtxoSpentEvent {
                        account_address: *self.account.component_address(),
                        address: UtxoAddress::new(*self.resource_address, spent.id),
                    });
                    num_spent += 1;
                }
            }

            if !self.utxos_to_recover.is_empty() {
                // Queue up tag matching UTXOs for processing
                tx.utxo_process_queue_extend(self.resource_address, self.utxos_to_recover.drain(..))?;
            }

            let bound = &self.stored_cursor_bound;
            let mut cursors = Vec::new();
            let mut claims = Vec::new();
            for (shard, version) in self.shard_state_versions_to_set.drain() {
                if let Some(cursor) = bound.bound(shard, version) {
                    cursors.push((shard, cursor));
                }
                if let Some(claim) = bound.claim(shard, version) &&
                    self.claims
                        .get(&shard)
                        .is_none_or(|stored| claim.state_version > stored.state_version)
                {
                    self.claims.insert(shard, claim);
                    claims.push((shard, claim));
                }
            }
            if !cursors.is_empty() {
                tx.shard_state_version_set_many(self.account.component_address(), self.resource_address, cursors)?;
            }
            if !claims.is_empty() {
                tx.shard_state_version_claims_set_many(
                    self.account.component_address(),
                    self.resource_address,
                    claims,
                )?;
            }
            Ok::<_, StealthScannerApiError>(())
        })?;

        self.stats.num_spent += num_spent;
        Ok(true)
    }

    fn check_if_tag_matches(&self, unspent: &UtxoUnspent) -> Result<bool, StealthScannerApiError> {
        let Ok(public_nonce) = unspent.public_nonce.try_from_byte_type().inspect_err(|e| {
            warn!(
                target: LOG_TARGET,
                "⚠️ Received a malformed public nonce while syncing output: {}. Ignoring output.",
                e
            )
        }) else {
            return Ok(false);
        };

        let tag = self.sdk.stealth_crypto_api().derive_stealth_output_tag(
            self.network,
            &self.view_key.secret,
            &public_nonce,
            self.resource_address,
        );

        if tag != unspent.tag {
            trace!(
                target: LOG_TARGET,
                "Stealth output tag {} does not match (likely not our output)", unspent.tag
            );
            return Ok(false);
        }

        debug!(
            target: LOG_TARGET,
            "Stealth output tag {} matches. Will request full output.", unspent.tag
        );

        Ok(true)
    }

    fn spend(
        tx: &mut <TSpec::Store as WriteableWalletStore>::WriteTransaction<'_>,
        resource_address: &ResourceAddress,
        spent: &UtxoSpent,
    ) -> Result<bool, WalletStorageError> {
        let Some(output) = tx
            .stealth_outputs_get_by_commitment(resource_address, &spent.id.into_commitment_bytes())
            .optional()?
        else {
            debug!(
                target: LOG_TARGET,
                "Spent UTXO {} not found in wallet store (likely not our output)", spent.id
            );
            return Ok(false);
        };
        if output.status == OutputStatus::Spent {
            debug!(target: LOG_TARGET, "Stealth output {} is already spent", spent.id);
            return Ok(false);
        }
        tx.stealth_outputs_mark_as_spent(resource_address, &spent.id)?;
        info!(target: LOG_TARGET, "Marked stealth output {} as spent", spent.id);
        Ok(true)
    }
}

/// How far a round may move the cursors it stores.
enum StoredCursorBound {
    /// Every cursor is stored as the indexer reports it.
    Unbounded,
    /// Each shard's cursor is stored no further than its state version in a verified end-of-epoch checkpoint, which
    /// the network committed. A shard without one keeps the cursor already stored. What the indexer reports past that
    /// is stored as a claim.
    Verified(HashMap<Shard, CheckpointStateVersion>),
}

impl StoredCursorBound {
    fn is_verified(&self) -> bool {
        matches!(self, Self::Verified(_))
    }

    fn ceiling(&self, shard: Shard) -> Option<StateVersion> {
        match self {
            Self::Unbounded => None,
            Self::Verified(versions) => versions.get(&shard).map(|checkpoint| checkpoint.state_version),
        }
    }

    /// The cursor to store for `shard` once the indexer reports it read to `version`, if any.
    fn bound(&self, shard: Shard, version: StateVersion) -> Option<StateVersion> {
        match self {
            Self::Unbounded => Some(version),
            Self::Verified(_) => self.ceiling(shard).map(|ceiling| version.min(ceiling)),
        }
    }

    /// The claim to store for `shard` once the indexer reports it read to `version`, if `version` is past what a
    /// verified checkpoint commits for the shard.
    fn claim(&self, shard: Shard, version: StateVersion) -> Option<ClaimedStateVersion> {
        let Self::Verified(versions) = self else {
            return None;
        };
        let checkpoint = versions.get(&shard);
        if checkpoint.is_some_and(|checkpoint| version <= checkpoint.state_version) {
            return None;
        }
        Some(ClaimedStateVersion {
            state_version: version,
            verified_epoch: checkpoint.map(|checkpoint| checkpoint.epoch),
        })
    }

    /// Whether the verified checkpoint of `shard` proves `claim` past any state version the network committed when it
    /// was made. A claim stored without a verified checkpoint is checked against the first one.
    fn refutes(&self, shard: Shard, claim: &ClaimedStateVersion) -> bool {
        let Self::Verified(versions) = self else {
            return false;
        };
        versions.get(&shard).is_some_and(|checkpoint| {
            claim.state_version > checkpoint.state_version &&
                claim.verified_epoch.is_none_or(|epoch| {
                    checkpoint.epoch.as_u64() >= epoch.as_u64().saturating_add(CLAIM_CHECK_EPOCHS)
                })
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct UtxoScanRoundStats {
    /// Number of UTXO updates received from the network
    pub num_received: usize,
    /// Number of UTXOs that matched the tag and were queued for recovery
    pub num_potential_recoveries: usize,
    /// Number of UTXOs that were marked as spent
    pub num_spent: usize,
}
