//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{array, collections::HashSet, time::Duration};

use log::*;
use tari_engine_types::UtxoOutput;
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
    storage::{ReadableWalletStore, WalletStorageError, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_template_lib_types::{ComponentAddress, ResourceAddress, UtxoAddress, UtxoId};
use tokio::sync::watch;

use crate::{notify::Notify, utxo_scanner::StealthScannerApiError};

const LOG_TARGET: &str = "tari::ootle::wallet_services::utxo_recovery";

const BATCH_SIZE: usize = 100;
/// How long queued UTXOs the indexer did not know wait to be asked for again, unless a scan wakes recovery sooner.
const UNRESOLVED_RETRY_INTERVAL: Duration = Duration::from_secs(60);

pub struct UtxoRecovery<TSpec: WalletSdkSpec> {
    sdk: WalletSdk<TSpec>,
    notify: Option<Notify<WalletEvent>>,
    round_id: usize,
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
    /// returned them nor reported them spent, or because a report that one is frozen could not be settled yet.
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

            for (resource_addr, tag_and_nonce_to_view_key_map) in &batch.entries {
                if tag_and_nonce_to_view_key_map.is_empty() {
                    error!(target: LOG_TARGET, "❓️ NEVER HAPPEN: Asked indexer for zero UTXOs for resource {}.", resource_addr);
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
                        })
                    })
                    .collect();
                let utxos = if self.sdk.sdk_config().indexer_trust.is_verify() {
                    let (utxos, num_deferred) = self.settle_frozen_claims(*resource_addr, utxos).await?;
                    num_unresolved += num_deferred;
                    utxos
                } else {
                    utxos
                };
                num_recovered += self.process_recovered_utxos(*resource_addr, utxos)?;
            }
        }
    }

    /// Settles each report that a UTXO is frozen, which withholds the output from spending, against the UTXO's state
    /// as the indexer proves it. A report is dropped when the answer refuses it for good: the indexer serves no proof,
    /// a proof that does not verify or that the wallet can never trust, or proves the UTXO unfrozen. The output is then
    /// held unfrozen. A UTXO whose report could not be settled for any other reason stays queued for a later round, and
    /// is counted in the returned number.
    ///
    /// A report that a UTXO is unfrozen needs no proof, as it only lets the wallet try a spend that the network
    /// decides. Nor does a report of a freeze the wallet already holds.
    async fn settle_frozen_claims(
        &self,
        resource_address: ResourceAddress,
        utxos: Vec<FoundUtxo>,
    ) -> Result<(Vec<FoundUtxo>, usize), StealthScannerApiError> {
        let substate_api = self.sdk.substate_api();
        let mut settled = Vec::with_capacity(utxos.len());
        let mut num_deferred = 0;
        for mut found in utxos {
            if !found.is_frozen {
                settled.push(found);
                continue;
            }
            let held = self
                .sdk
                .store()
                .with_read_tx(|tx| {
                    tx.stealth_outputs_get_by_commitment(&resource_address, &found.id.into_commitment_bytes())
                })
                .optional()?;
            if held.is_some_and(|output| output.is_frozen) {
                settled.push(found);
                continue;
            }

            let address = UtxoAddress::new(resource_address, found.id);
            match substate_api
                .fetch_substate_from_network(&address.clone().into(), None)
                .await
            {
                Ok(read) => {
                    found.is_frozen = read.substate.as_utxo().is_some_and(|utxo| utxo.is_frozen());
                    if !found.is_frozen {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ The indexer reported stealth output {} frozen, but proved it unfrozen. Keeping the output spendable.",
                            address
                        );
                    }
                    settled.push(found);
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
                    settled.push(found);
                },
                Err(SubstateApiError::StoreError(err)) => return Err(err.into()),
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Could not check the indexer's report that stealth output {} is frozen ({}). Retrying later.",
                        address,
                        err
                    );
                    num_deferred += 1;
                },
            }
        }
        Ok((settled, num_deferred))
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
        let Some(output) = outputs_api.validate_utxo(
            array::from_ref(&keys),
            network,
            resource_address,
            commitment,
            &found.output,
            found.is_frozen,
        )?
        else {
            // The output could be burnt, frozen, or otherwise invalid. If we already have it in the db, update its
            // status, if not, do nothing.
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
}
