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
    models::{
        AccountWithAddress,
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

pub struct UtxoScannerRound<'a, TSpec: WalletSdkSpec> {
    network: Network,
    account: &'a AccountWithAddress,
    view_key: &'a WalletSecretKey,
    resource_address: &'a ResourceAddress,

    sdk: &'a WalletSdk<TSpec>,
    stats: UtxoScanRoundStats,

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
            shard_state_versions_to_set: HashMap::new(),
            utxos_to_recover: Vec::new(),
            utxos_to_spend: Vec::new(),
            notify,
            stats: UtxoScanRoundStats::default(),
        }
    }

    pub async fn scan_for_utxo_updates(&mut self) -> Result<(), StealthScannerApiError> {
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

    #[allow(clippy::too_many_lines)]
    async fn process_next_batch(&mut self) -> Result<bool, StealthScannerApiError> {
        let mut shard_state_versions = self
            .sdk
            .store()
            .with_read_tx(|tx| tx.shard_state_version_get(self.account.component_address(), self.resource_address))?;

        // Populate any missing shards with zero because we want to scan all shards
        for shard in NUM_PRESHARDS.all_shards_iter() {
            shard_state_versions.entry(shard).or_insert_with(StateVersion::zero);
        }

        info!(
            target: LOG_TARGET,
            "🔍️ Scanning for stealth outputs (account {}, resource {}, num shards {})",
            self.account.component_address(),
            self.resource_address,
            shard_state_versions.len(),
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
                shard_state_versions.into_iter().collect(),
                // Optimisation: with no unspent or locked outputs there is no spend left for the wallet to
                // learn of, so only unspent updates are needed. The indexer moves the cursor past the spent
                // updates this skips, so they are not delivered on a later pass either.
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

                self.shard_state_versions_to_set
                    .entry(recv_sos.shard)
                    .and_modify(|v| {
                        if recv_sos.max_state_version > *v {
                            *v = recv_sos.max_state_version
                        }
                    })
                    .or_insert(recv_sos.max_state_version);

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

            // Update shard state versions
            if !self.shard_state_versions_to_set.is_empty() {
                tx.shard_state_version_set_many(
                    self.account.component_address(),
                    self.resource_address,
                    self.shard_state_versions_to_set.drain(),
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
        match tx
            .stealth_outputs_mark_as_spent(resource_address, &spent.id)
            .optional()?
        {
            Some(_) => {
                info!(target: LOG_TARGET, "Marked stealth output {} as spent", spent.id);
                Ok(true)
            },
            None => {
                debug!(
                    target: LOG_TARGET,
                    "Spent UTXO {} not found in wallet store (likely not our output)", spent.id
                );
                Ok(false)
            },
        }
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
