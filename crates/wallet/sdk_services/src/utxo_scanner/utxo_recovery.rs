//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{array, time::Duration};

use log::*;
use tari_engine_types::UtxoOutput;
use tari_ootle_common_types::{optional::IsNotFoundError, response_status::TransactionStatusResponseError};
use tari_ootle_transaction::Network;
use tari_ootle_wallet_sdk::{
    WalletSdk,
    WalletSdkSpec,
    models::{
        AccountAndViewKeys,
        UtxoRecoveredEvent,
        UtxoRecoveryCompletedEvent,
        UtxoRecoveryStartedEvent,
        WalletEvent,
    },
    network::WalletNetworkInterface,
    storage::{ReadableWalletStore, WalletStorageError, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_template_lib_types::{ComponentAddress, ResourceAddress, UtxoAddress, UtxoId};
use tokio::sync::watch;

use crate::{notify::Notify, utxo_scanner::StealthScannerApiError};

const LOG_TARGET: &str = "tari::ootle::wallet_services::utxo_recovery";

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
            if let Err(err) = self.process_utxo_validation_queue().await {
                match &err {
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
                }
            }

            if waker.changed().await.is_err() {
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

    #[allow(clippy::too_many_lines)]
    pub async fn process_utxo_validation_queue(&mut self) -> Result<(), StealthScannerApiError> {
        let mut start_event_published = false;
        let mut num_recovered = 0;
        loop {
            let batch = self
                .sdk
                .store()
                .with_read_tx(|tx| tx.utxo_process_queue_fetch_batch(100))?;

            if batch.is_empty() {
                debug!(target: LOG_TARGET, "✅ No more UTXOs to process");
                return Ok(());
            }

            if !start_event_published {
                self.round_id += 1;
                self.notify(UtxoRecoveryStartedEvent {
                    round_id: self.round_id,
                });
                start_event_published = true;
            }

            for (resource_addr, tag_and_nonce_to_view_key_map) in &batch {
                if tag_and_nonce_to_view_key_map.is_empty() {
                    error!(target: LOG_TARGET, "❓️ NEVER HAPPEN: Asked indexer for zero UTXOs for resource {}.", resource_addr);
                }
                let tag_and_nonce_pairs = tag_and_nonce_to_view_key_map.keys().copied().collect();

                // TODO(perf): we should check if any UTXOs are already known (by tag and nonce?) because we
                // created/spent them locally, and skip querying those. This would also avoid having to deal with this
                // downstream (previous bug caused spent to be marked as unspent).

                // max 3.3kB per request (excl underlying protocol overhead *cough* json + hex)
                let utxos = self
                    .sdk
                    .get_network_interface()
                    .get_unspent_utxos(*resource_addr, tag_and_nonce_pairs)
                    .await
                    .map_err(|e| StealthScannerApiError::NetworkInterfaceError(e.into()))?;

                self.sdk.store().with_write_tx(|tx| {
                    // We're trusting that the indexer is up to date and accurate. If any UTXOs we asked for are not
                    // returned, remove them from the queue to avoid retrying forever because they are presumably spent.
                    let missing_utxos = tag_and_nonce_to_view_key_map
                        .keys()
                        .filter(|(tag, nonce)| {
                            !utxos.iter().any(|(_, utxo)| {
                                utxo.output.as_ref().is_some_and(|output| utxo.tag() == Some(*tag) && output.output.public_nonce == *nonce)
                            })
                        });

                    let mut count = 0usize;
                    for (tag, nonce) in missing_utxos {
                        count += 1;
                        tx.utxo_process_queue_remove_item(*resource_addr, *tag, *nonce)?;
                    }
                    if count > 0 {
                        debug!(target: LOG_TARGET, "❓️ Removed {} missing UTXOs from the processing queue for resource {} as they were not returned by the indexer.", count, resource_addr);
                    }
                    Ok::<_, WalletStorageError>(())
                })?;

                if utxos.is_empty() {
                    // We asked for some UTXOs but got none back. This could happen if the UTXOs were all spent later.
                    // Ignore this and continue syncing.
                    continue;
                }

                if utxos.len() != tag_and_nonce_to_view_key_map.len() {
                    // We could error as above, but let's process what we got
                    warn!(target: LOG_TARGET, "⚠️ Mismatch in number of UTXOs queried ({}) vs returned by indexer ({}).",
                        tag_and_nonce_to_view_key_map.len(),
                        utxos.len(),
                    );
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
                num_recovered += self.process_recovered_utxos(*resource_addr, utxos)?;
            }

            if start_event_published {
                self.notify(UtxoRecoveryCompletedEvent {
                    round_id: self.round_id,
                    num_recovered,
                });
            }
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
