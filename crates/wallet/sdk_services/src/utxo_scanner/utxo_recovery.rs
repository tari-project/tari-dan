//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    array,
    collections::{HashMap, HashSet},
    time::Duration,
};

use log::*;
use tari_engine_types::{UtxoOutput, substate::Substate};
use tari_ootle_common_types::{
    optional::{IsNotFoundError, Optional},
    response_status::TransactionStatusResponseError,
};
use tari_ootle_transaction::Network;
use tari_ootle_wallet_sdk::{
    WalletSdk,
    WalletSdkSpec,
    apis::substate::SubstateApiError,
    models::{
        AccountAndViewKeys,
        UtxoRecoveredEvent,
        UtxoRecoveryCompletedEvent,
        UtxoRecoveryStartedEvent,
        WalletEvent,
    },
    network::{FetchedUtxos, WalletNetworkInterface},
    storage::{
        ReadableWalletStore,
        TagAndPublicNoncePair,
        WalletStorageError,
        WalletStoreReader,
        WalletStoreWriter,
        WriteableWalletStore,
    },
};
use tari_template_lib_types::{ComponentAddress, ResourceAddress, UtxoAddress, UtxoId};
use tokio::{sync::watch, time::Instant};

use crate::{notify::Notify, utxo_scanner::StealthScannerApiError};

const LOG_TARGET: &str = "tari::ootle::wallet_services::utxo_recovery";

const BATCH_SIZE: usize = 100;
/// How long queued UTXOs the indexer did not know wait to be asked for again, unless a scan wakes recovery sooner.
const UNRESOLVED_RETRY_INTERVAL: Duration = Duration::from_secs(60);
/// The longest a queued UTXO that could not be settled against a proof waits to be asked for again.
const MAX_UNSETTLED_RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub struct UtxoRecovery<TSpec: WalletSdkSpec> {
    sdk: WalletSdk<TSpec>,
    notify: Option<Notify<WalletEvent>>,
    round_id: usize,
    /// The queued UTXOs that could not be settled against a proof, and when each is next asked for. The wait doubles
    /// with each attempt, from [`UNRESOLVED_RETRY_INTERVAL`] up to [`MAX_UNSETTLED_RETRY_INTERVAL`]. It is held in
    /// memory, so a restart asks for every queued UTXO again.
    unsettled: HashMap<(ResourceAddress, TagAndPublicNoncePair), RetryBackoff>,
}

impl<TSpec> UtxoRecovery<TSpec>
where
    TSpec: WalletSdkSpec,
    <TSpec::NetworkInterface as WalletNetworkInterface>::Error: IsNotFoundError + TransactionStatusResponseError,
{
    pub fn new(sdk: WalletSdk<TSpec>) -> Self {
        Self {
            sdk,
            notify: None,
            round_id: 0,
            unsettled: HashMap::new(),
        }
    }

    pub fn with_notify(mut self, events: Notify<WalletEvent>) -> Self {
        self.notify = Some(events);
        self
    }

    pub async fn run(mut self, mut waker: watch::Receiver<()>) -> anyhow::Result<()> {
        info!(target: LOG_TARGET, "🚀 Starting UTXO recovery process");
        loop {
            // Process the UTXO validation queue until empty. NOTE: this comes before waiting on the waker so that we
            // check the queue on startup
            let num_unresolved = match self.process_utxo_validation_queue().await {
                Ok(num_unresolved) => num_unresolved,
                Err(err) => match &err {
                    // Perhaps intermittent network issue, just log and continue
                    StealthScannerApiError::NetworkInterfaceError(_) |
                    StealthScannerApiError::UnexpectedResponse { .. } => {
                        warn!(target: LOG_TARGET, "⚠️ UTXO recovery process encountered a recoverable error: {}", err);
                        // Sleep to prevent fast spinning
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        // Continue processing the queue
                        continue;
                    },
                    _ => {
                        // Non-recoverable error
                        error!(target: LOG_TARGET, "❌ UTXO recovery process encountered a non-recoverable error: {}", err);
                        // CRASH
                        return Err(err.into());
                    },
                },
            };

            let woken = if num_unresolved == 0 {
                waker.changed().await
            } else {
                match tokio::time::timeout(UNRESOLVED_RETRY_INTERVAL, waker.changed()).await {
                    Ok(woken) => woken,
                    Err(_) => Ok(()),
                }
            };
            if woken.is_err() {
                info!(target: LOG_TARGET, "🔕 UTXO recovery process terminating because the notification channel closed");
                break;
            }
        }
        Ok(())
    }

    fn notify<T: Into<WalletEvent>>(&self, event: T) {
        if let Some(notify) = &self.notify {
            notify.notify(event);
        }
    }

    /// Processes each queued UTXO once and returns the number left queued, because the indexer that answered neither
    /// returned them nor reported them spent, or because a report of one could not be settled against a proof yet. A
    /// UTXO whose report could not be settled is skipped until its wait after the last attempt has passed.
    #[allow(clippy::too_many_lines)]
    pub async fn process_utxo_validation_queue(&mut self) -> Result<usize, StealthScannerApiError> {
        let mut start_event_published = false;
        let mut num_recovered = 0;
        let mut num_unresolved = 0;
        let mut cursor = None;
        loop {
            let batch = self
                .sdk
                .store()
                .with_read_tx(|tx| tx.utxo_process_queue_fetch_batch(cursor, BATCH_SIZE))?;

            let Some(last) = batch.last else {
                debug!(target: LOG_TARGET, "✅ No more UTXOs to process ({} unresolved)", num_unresolved);
                if start_event_published {
                    self.notify(UtxoRecoveryCompletedEvent {
                        round_id: self.round_id,
                        num_recovered,
                    });
                }
                return Ok(num_unresolved);
            };
            // Unresolved entries stay queued, so each batch starts after the last entry of the one before
            cursor = Some(last);

            for (resource_addr, queued) in &batch.entries {
                if queued.is_empty() {
                    error!(target: LOG_TARGET, "❓️ NEVER HAPPEN: Asked indexer for zero UTXOs for resource {}.", resource_addr);
                }
                let now = Instant::now();
                let tag_and_nonce_to_view_key_map = queued
                    .iter()
                    .filter(|(pair, _)| {
                        self.unsettled
                            .get(&(*resource_addr, **pair))
                            .is_none_or(|backoff| backoff.retry_at <= now)
                    })
                    .map(|(pair, account)| (*pair, *account))
                    .collect::<HashMap<_, _>>();
                num_unresolved += queued.len() - tag_and_nonce_to_view_key_map.len();
                if tag_and_nonce_to_view_key_map.is_empty() {
                    continue;
                }
                let tag_and_nonce_pairs = tag_and_nonce_to_view_key_map.keys().copied().collect();

                // TODO(perf): we should check if any UTXOs are already known (by tag and nonce?) because we
                // created/spent them locally, and skip querying those. This would also avoid having to deal with this
                // downstream (previous bug caused spent to be marked as unspent).

                // max 3.3kB per request (excl underlying protocol overhead *cough* json + hex)
                let FetchedUtxos { unspent: utxos, spent } = self
                    .sdk
                    .get_network_interface()
                    .fetch_utxos(*resource_addr, tag_and_nonce_pairs)
                    .await
                    .map_err(|e| StealthScannerApiError::NetworkInterfaceError(e.into()))?;

                let returned = utxos
                    .iter()
                    .filter_map(|(_, utxo)| utxo.output())
                    .map(|output| (output.tag, output.output.public_nonce))
                    .collect::<HashSet<_>>();
                // A spent report names a (tag, nonce) pair, so it means some output with that pair is spent. The queue
                // holds no commitment to tell outputs sharing a pair apart, and an unspent output with the pair
                // resolves it first.
                let spent = spent
                    .into_iter()
                    .filter(|pair| tag_and_nonce_to_view_key_map.contains_key(pair) && !returned.contains(pair))
                    .collect::<HashSet<_>>();

                // A round that only retries unresolved entries publishes no round events
                if !start_event_published && (!spent.is_empty() || !utxos.is_empty()) {
                    self.round_id += 1;
                    self.notify(UtxoRecoveryStartedEvent {
                        round_id: self.round_id,
                    });
                    start_event_published = true;
                }

                self.sdk.store().with_write_tx(|tx| {
                    for (tag, nonce) in &spent {
                        tx.utxo_process_queue_remove_item(*resource_addr, *tag, *nonce)?;
                    }
                    Ok::<_, WalletStorageError>(())
                })?;
                for pair in &spent {
                    self.unsettled.remove(&(*resource_addr, *pair));
                }
                if !spent.is_empty() {
                    debug!(target: LOG_TARGET, "Removed {} spent UTXOs from the processing queue for resource {}", spent.len(), resource_addr);
                }

                // A pair the indexer neither returned nor reported spent may be one it has not indexed yet, so it
                // stays queued for a later round.
                let unresolved = tag_and_nonce_to_view_key_map
                    .keys()
                    .filter(|pair| !returned.contains(*pair) && !spent.contains(*pair))
                    .count();
                if unresolved > 0 {
                    debug!(target: LOG_TARGET, "❓️ The indexer does not know {} queued UTXOs for resource {}. Retrying them later.", unresolved, resource_addr);
                    num_unresolved += unresolved;
                }

                if utxos.is_empty() {
                    continue;
                }

                // Group by account key index
                let utxos = utxos
                    .into_iter()
                    .filter_map(|(id, utxo)| {
                        let is_frozen = utxo.is_frozen;
                        let Some(output) = utxo.into_output() else {
                            warn!(target: LOG_TARGET, "❓️ NEVER HAPPEN: Indexer returned burnt UTXO {}. Ignoring", id);
                            return None;
                        };
                        Some((id, output, is_frozen))
                    })
                    .filter_map(|(id, output, is_frozen)| {
                        let tag_and_nonce_pair = (output.tag, output.output.public_nonce);
                        let Some(account_addr) = tag_and_nonce_to_view_key_map.get(&tag_and_nonce_pair).copied() else {
                            warn!(target: LOG_TARGET, "❓️ NEVER HAPPEN: Indexer returned UTXO with tag {}, nonce {} that we didn't request. Ignoring", output.tag, output.output.public_nonce);
                            return None;
                        };

                        Some(FoundUtxo {
                            account_addr,
                            id,
                            output,
                            is_frozen,
                            is_held: false,
                        })
                    })
                    .collect();
                let utxos = if self.sdk.sdk_config().indexer_trust.is_verify() {
                    let (utxos, deferred) = self.settle_reported_utxos(*resource_addr, utxos).await?;
                    num_unresolved += deferred.len();
                    for pair in deferred {
                        self.unsettled
                            .entry((*resource_addr, pair))
                            .or_insert(RetryBackoff {
                                attempts: 0,
                                retry_at: now,
                            })
                            .defer(now);
                    }
                    for found in &utxos {
                        self.unsettled.remove(&(*resource_addr, found.tag_and_nonce()));
                    }
                    utxos
                } else {
                    utxos
                };
                num_recovered += self.process_recovered_utxos(*resource_addr, utxos)?;
            }
        }
    }

    /// Settles each reported UTXO against the UTXO's state as the indexer proves it. Returns those the wallet may
    /// store, and the tag and nonce pairs of those left queued for a later round. The UTXOs that need a proof are
    /// read in one fetch.
    ///
    /// A UTXO the wallet does not hold is stored only once the indexer proves that the network holds it, with the
    /// output and frozen flag the proof shows. Until then it stays queued, so that an output the network does not hold
    /// is never counted as spendable.
    ///
    /// For a UTXO the wallet holds, only the frozen flag is taken from the report, and only a report that it is
    /// frozen, which withholds the output from spending, needs a proof. That report is dropped when the answer refuses
    /// it for good: the indexer serves no proof, a proof that does not verify or that the wallet can never trust, or
    /// proves the UTXO unfrozen. The output is then held unfrozen. A report that could not be settled for any other
    /// reason stays queued. A report that a held UTXO is unfrozen needs no proof, as it only lets the wallet try a
    /// spend that the network decides. Nor does a report of a freeze the wallet already holds.
    async fn settle_reported_utxos(
        &self,
        resource_address: ResourceAddress,
        utxos: Vec<FoundUtxo>,
    ) -> Result<(Vec<FoundUtxo>, Vec<TagAndPublicNoncePair>), StealthScannerApiError> {
        let mut settled = Vec::with_capacity(utxos.len());
        let mut to_prove = Vec::new();
        for mut found in utxos {
            let held = self
                .sdk
                .store()
                .with_read_tx(|tx| {
                    tx.stealth_outputs_get_by_commitment(&resource_address, &found.id.into_commitment_bytes())
                })
                .optional()?;
            if let Some(held) = held {
                found.is_held = true;
                if !found.is_frozen || held.is_frozen {
                    settled.push(found);
                    continue;
                }
            }
            to_prove.push(found);
        }
        if to_prove.is_empty() {
            return Ok((settled, Vec::new()));
        }

        let ids = to_prove
            .iter()
            .map(|found| UtxoAddress::new(resource_address, found.id).into())
            .collect();
        let reads = match self.sdk.substate_api().get_each_substate_from_network(ids).await {
            Ok(reads) => reads,
            Err(SubstateApiError::StoreError(err)) => return Err(err.into()),
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Could not fetch the proofs of {} stealth outputs the indexer reported for resource {} ({}). Retrying later.",
                    to_prove.len(),
                    resource_address,
                    err
                );
                let deferred = to_prove.iter().map(FoundUtxo::tag_and_nonce).collect();
                return Ok((settled, deferred));
            },
        };

        let mut deferred = Vec::new();
        for (found, (_, read)) in to_prove.into_iter().zip(reads) {
            let pair = found.tag_and_nonce();
            let settled_utxo = if found.is_held {
                Self::settle_held_frozen_claim(resource_address, found, read)?
            } else {
                Self::settle_new_utxo(resource_address, found, read)?
            };
            match settled_utxo {
                Some(found) => settled.push(found),
                None => deferred.push(pair),
            }
        }
        Ok((settled, deferred))
    }

    /// Settles a reported UTXO the wallet does not hold against `read`. Returns the UTXO as the indexer proves it, or
    /// `None` when it is not proven.
    fn settle_new_utxo(
        resource_address: ResourceAddress,
        found: FoundUtxo,
        read: Result<Substate, SubstateApiError>,
    ) -> Result<Option<FoundUtxo>, StealthScannerApiError> {
        let address = UtxoAddress::new(resource_address, found.id);
        let substate = match read {
            Ok(substate) => substate,
            Err(SubstateApiError::StoreError(err)) => return Err(err.into()),
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ The indexer reported stealth output {} unspent without proving it ({}). Retrying later.",
                    address,
                    err
                );
                return Ok(None);
            },
        };
        let Some(utxo) = substate.into_substate_value().into_utxo() else {
            warn!(
                target: LOG_TARGET,
                "⚠️ The indexer proved substate {} holds no UTXO. Retrying later.",
                address
            );
            return Ok(None);
        };
        let is_frozen = utxo.is_frozen();
        let Some(output) = utxo.into_output() else {
            warn!(
                target: LOG_TARGET,
                "⚠️ The indexer reported stealth output {} unspent, but proved it burnt. Retrying later.",
                address
            );
            return Ok(None);
        };
        if (output.tag, output.output.public_nonce) != found.tag_and_nonce() {
            warn!(
                target: LOG_TARGET,
                "⚠️ The indexer proved stealth output {} with an output other than the one it reported. Retrying later.",
                address
            );
            return Ok(None);
        }
        Ok(Some(FoundUtxo {
            output,
            is_frozen,
            ..found
        }))
    }

    /// Settles the report that a UTXO the wallet holds unfrozen is frozen against `read`. Returns `None` when the
    /// report could not be settled yet.
    fn settle_held_frozen_claim(
        resource_address: ResourceAddress,
        mut found: FoundUtxo,
        read: Result<Substate, SubstateApiError>,
    ) -> Result<Option<FoundUtxo>, StealthScannerApiError> {
        let address = UtxoAddress::new(resource_address, found.id);
        match read {
            Ok(substate) => {
                found.is_frozen = substate.substate_value().as_utxo().is_some_and(|utxo| utxo.is_frozen());
                if !found.is_frozen {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ The indexer reported stealth output {} frozen, but proved it unfrozen. Keeping the output spendable.",
                        address
                    );
                }
                Ok(Some(found))
            },
            Err(
                err @ (SubstateApiError::SubstateDoesNotExist { .. } |
                SubstateApiError::SubstateUnproven { .. } |
                SubstateApiError::SubstateUnverified { .. } |
                SubstateApiError::SubstateProvenBeforeAnchor { .. } |
                SubstateApiError::SubstateProofStale { .. }),
            ) => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ The indexer reported stealth output {} frozen without proving it ({}). Keeping the output spendable.",
                    address,
                    err
                );
                found.is_frozen = false;
                Ok(Some(found))
            },
            Err(SubstateApiError::StoreError(err)) => Err(err.into()),
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Could not check the indexer's report that stealth output {} is frozen ({}). Retrying later.",
                    address,
                    err
                );
                Ok(None)
            },
        }
    }

    fn process_recovered_utxos(
        &self,
        resource_address: ResourceAddress,
        utxos_to_recover: Vec<FoundUtxo>,
    ) -> Result<usize, StealthScannerApiError> {
        let num_attempted = utxos_to_recover.len();
        let mut num_recovered = 0;
        for utxo in utxos_to_recover {
            if self.check_unspent_utxo_and_store(self.sdk.network(), resource_address, utxo)? {
                num_recovered += 1;
            }
        }
        if num_attempted > 0 {
            info!(
                target: LOG_TARGET,
                "✅ Recovered {} out of {} applicable UTXOs",
                num_recovered,
                num_attempted,
            );
        }

        Ok(num_recovered)
    }

    fn check_unspent_utxo_and_store(
        &self,
        network: Network,
        resource_address: ResourceAddress,
        found: FoundUtxo,
    ) -> Result<bool, StealthScannerApiError> {
        let account = self.sdk.accounts_api().get_account_by_address(&found.account_addr)?;
        let view_only_key = self.sdk.key_manager_api().get_key(account.view_only_key_id())?;
        let account_key = account
            .owner_key_id()
            .map(|key_id| self.sdk.key_manager_api().get_key(key_id))
            .transpose()?;
        let keys = AccountAndViewKeys {
            account_public_key: *account.owner_public_key(),
            account_key,
            view_only_key,
        };
        let outputs_api = self.sdk.stealth_outputs_api();

        let address = UtxoAddress::new(resource_address, found.id);
        let commitment = found.id.into_commitment_bytes();
        let validated = if found.is_held {
            None
        } else {
            outputs_api.validate_utxo(
                array::from_ref(&keys),
                network,
                resource_address,
                commitment,
                &found.output,
                found.is_frozen,
            )?
        };
        let Some(output) = validated else {
            // The output could be held already, burnt, frozen, or otherwise invalid. If we already have it in the db,
            // update its frozen flag, if not, do nothing.
            let frozen_changed = outputs_api.set_utxo_frozen(&address, found.is_frozen)?.unwrap_or(false);

            if frozen_changed {
                self.notify(UtxoRecoveredEvent {
                    address: UtxoAddress::new(resource_address, found.id),
                    account_address: found.account_addr,
                });
            }
            self.sdk.store().with_write_tx(|tx| {
                tx.utxo_process_queue_remove_item(resource_address, found.output.tag, found.output.output.public_nonce)
            })?;

            return Ok(false);
        };

        let changed = outputs_api.upsert_utxo(&output)?;

        self.sdk.store().with_write_tx(|tx| {
            tx.utxo_process_queue_remove_item(resource_address, found.output.tag, found.output.output.public_nonce)
        })?;

        if !changed {
            debug!(target: LOG_TARGET, "Stealth output {} is already recorded", address);
            return Ok(false);
        }
        self.notify(UtxoRecoveredEvent {
            address: UtxoAddress::new(resource_address, found.id),
            account_address: found.account_addr,
        });
        info!(target: LOG_TARGET, "💰️ Recovered stealth output {} for account {}", address, keys.account_public_key);
        Ok(true)
    }
}

struct FoundUtxo {
    account_addr: ComponentAddress,
    output: UtxoOutput,
    id: UtxoId,
    is_frozen: bool,
    /// The wallet holds the output and its contents, so only the settled frozen flag is taken from the report. The
    /// reported output carries no proof.
    is_held: bool,
}

impl FoundUtxo {
    fn tag_and_nonce(&self) -> TagAndPublicNoncePair {
        (self.output.tag, self.output.output.public_nonce)
    }
}

struct RetryBackoff {
    attempts: u32,
    retry_at: Instant,
}

impl RetryBackoff {
    /// Records a failed attempt at `now`, and waits twice as long as after the attempt before.
    fn defer(&mut self, now: Instant) {
        let wait = UNRESOLVED_RETRY_INTERVAL
            .saturating_mul(1 << self.attempts.min(6))
            .min(MAX_UNSETTLED_RETRY_INTERVAL);
        self.attempts = self.attempts.saturating_add(1);
        self.retry_at = now + wait;
    }
}
