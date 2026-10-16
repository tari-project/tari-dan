//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    path::Path,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use log::info;
use tari_crypto::tari_utilities::SafePassword;
use tari_engine_types::commit_result::FinalizeResult;
use tari_ootle_common_types::Epoch;
use tari_ootle_transaction::{Network, Transaction, TransactionBuilder, TransactionId};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    WalletSdk as Sdk,
    WalletSdkConfig,
    cipher_seed::CipherSeedRestore,
    local_key_store::LocalKeyStore,
    models::EpochBirthday,
    network::WalletNetworkInterface,
};
use tari_ootle_wallet_sdk_services::indexer_rest_api::IndexerRestApiNetworkInterface;
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_template_lib_types::TemplateAddress;
use tokio::time::sleep;
use url::Url;

use crate::{cli::CommonArgs, stats::Stats, templates::get_templates};

pub struct TariswapTestSdkSpec;

impl tari_ootle_wallet_sdk::WalletSdkSpec for TariswapTestSdkSpec {
    type KeyStore = LocalKeyStore;
    type NetworkInterface = IndexerRestApiNetworkInterface;
    type Store = SqliteWalletStore;
}

type WalletSdk = Sdk<TariswapTestSdkSpec>;
pub struct Runner {
    pub(crate) sdk: WalletSdk,
    pub(crate) cli: CommonArgs,
    pub(crate) faucet_template: TemplateAddress,
    pub(crate) tariswap_template: TemplateAddress,
    pub(crate) stats: Stats,
    /// Validity window shared by every transaction the bench builds, resolved once at startup. A
    /// bench run is short relative to this, so a single window covers the whole run.
    pub(crate) max_epoch: Epoch,
    /// Source of the per-transaction nonce. The transaction id excludes the seal signature, so
    /// the nonce is what makes each member of a batch that repeats the same instructions over the
    /// same unversioned inputs (pool creation, swaps) a distinct transaction.
    next_nonce: AtomicU64,
}

/// Epochs past the epoch at startup that bench transactions stay valid for.
const BENCH_TRANSACTION_VALIDITY_EPOCHS: u64 = 10;

impl Runner {
    pub async fn init(cli: CommonArgs) -> anyhow::Result<Self> {
        let sdk = initialize_wallet_sdk(&cli.db_path, cli.indexer_url.clone())?;
        let (faucet_template, tariswap_template) = get_templates(&cli).await?;
        let current_epoch = sdk.get_network_interface().get_current_epoch().await?;
        let max_epoch = Epoch(current_epoch.as_u64() + BENCH_TRANSACTION_VALIDITY_EPOCHS);
        Ok(Self {
            sdk,
            cli,
            faucet_template,
            tariswap_template,
            stats: Stats::default(),
            max_epoch,
            next_nonce: AtomicU64::new(0),
        })
    }

    pub async fn submit_transaction_and_wait(&mut self, transaction: Transaction) -> anyhow::Result<FinalizeResult> {
        let tx_id = self.submit_transaction(transaction).await?;
        let finalize = self.wait_for_transaction(tx_id).await?;
        Ok(finalize)
    }

    pub async fn submit_transaction(&mut self, transaction: Transaction) -> anyhow::Result<TransactionId> {
        let tx_id = self
            .sdk
            .transaction_api()
            .insert_new_transaction(transaction, None, &[], false)?;

        self.sdk.transaction_api().submit_transaction(tx_id).await?;

        self.stats.inc_transaction();
        Ok(tx_id)
    }

    pub async fn wait_for_transaction(&mut self, tx_id: TransactionId) -> anyhow::Result<FinalizeResult> {
        loop {
            let Some(tx) = self
                .sdk
                .transaction_api()
                .check_and_store_finalized_transaction(tx_id)
                .await?
            else {
                sleep(Duration::from_secs(1)).await;
                continue;
            };

            let Some(ref finalize) = tx.finalize else {
                sleep(Duration::from_secs(1)).await;
                continue;
            };

            self.stats.add_execution_time(tx.execution_time.unwrap());
            self.stats.add_time_to_finalize(tx.finalized_time.unwrap());

            if !finalize.is_full_accept() {
                return Err(anyhow::anyhow!(
                    "Transaction {} failed: {:?}",
                    tx_id,
                    finalize.result.any_reject().unwrap()
                ));
            }

            self.stats
                .add_substate_created(finalize.result.any_accept().unwrap().up_len());

            return Ok(finalize.clone());
        }
    }

    pub fn log_stats(&self) {
        info!("Stats:");
        info!("  - Num transactions: {}", self.stats.num_transactions());
        info!("  - Total execution time: {:.2?}", self.stats.total_execution_time());
        info!(
            "  - Total time to finalize: {:.2?}",
            self.stats.total_time_to_finalize()
        );
        info!("  - Num substates created: {}", self.stats.num_substates_created());
    }

    pub fn new_transaction_builder(&self) -> TransactionBuilder {
        Transaction::builder(self.cli.network, self.max_epoch)
            .with_nonce(self.next_nonce.fetch_add(1, Ordering::Relaxed))
    }
}

fn initialize_wallet_sdk<P: AsRef<Path>>(db_path: P, indexer_url: Url) -> Result<WalletSdk, anyhow::Error> {
    let store = SqliteWalletStore::try_open(db_path)?;
    store.run_migrations()?;

    let sdk_config = WalletSdkConfig {
        network: Network::LocalNet,
        override_keyring_password: Some(SafePassword::from_str("N3Va g0nn4 gu355").unwrap()),
        indexer_trust: IndexerTrust::default(),
    };
    let indexer = IndexerRestApiNetworkInterface::new(indexer_url);
    let mut sdk = WalletSdk::initialize_with_local_key_store(store, indexer, sdk_config, EpochBirthday::far_future())?;
    sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)?;
    Ok(sdk)
}
