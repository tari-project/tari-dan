//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{path::Path, time::Duration};

use log::info;
use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_daemon::indexer_jrpc_impl::IndexerJsonRpcNetworkInterface;
use tari_dan_wallet_sdk::{DanWalletSdk, WalletSdkConfig};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_engine_types::commit_result::FinalizeResult;
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};
use tari_validator_node_client::{
    types::{GetTransactionResultRequest, TemplateMetadata},
    ValidatorNodeClient,
};
use tokio::time;
use url::Url;

use crate::{cli::CommonArgs, stats::Stats, templates::get_templates};

type WalletSdk = DanWalletSdk<SqliteWalletStore, IndexerJsonRpcNetworkInterface>;
pub struct Runner {
    pub(crate) sdk: WalletSdk,
    pub(crate) cli: CommonArgs,
    pub(crate) faucet_template: TemplateMetadata,
    pub(crate) tariswap_template: TemplateMetadata,
    pub(crate) stats: Stats,
}

impl Runner {
    pub async fn init(cli: CommonArgs) -> anyhow::Result<Self> {
        let sdk = initialize_wallet_sdk(&cli.db_path, cli.indexer_url.clone())?;
        let (faucet_template, tariswap_template) = get_templates(&cli.validator_node_url).await?;
        Ok(Self {
            sdk,
            cli,
            faucet_template,
            tariswap_template,
            stats: Stats::default(),
        })
    }

    pub async fn submit_transaction_and_wait(&mut self, transaction: Transaction) -> anyhow::Result<FinalizeResult> {
        let tx_id = self.submit_transaction(transaction).await?;
        let finalize = self.wait_for_transaction(tx_id).await?;
        Ok(finalize)
    }

    pub async fn submit_transaction(&mut self, transaction: Transaction) -> anyhow::Result<TransactionId> {
        let inputs = transaction
            .to_referenced_substates()?
            .into_iter()
            .map(|s| SubstateRequirement::new(s, None))
            .collect();

        self.stats.inc_transaction();
        let tx_id = self
            .sdk
            .transaction_api()
            .submit_transaction(transaction, inputs)
            .await?;
        Ok(tx_id)
    }

    pub async fn wait_for_transaction(&mut self, tx_id: TransactionId) -> anyhow::Result<FinalizeResult> {
        let mut client = ValidatorNodeClient::connect(self.cli.validator_node_url.clone())?;
        loop {
            let result = client
                .get_transaction_result(GetTransactionResultRequest { transaction_id: tx_id })
                .await
                .optional()?;
            let Some((result, exec_time)) = result.and_then(|r| Some((r.result?, r.execution_time?))) else {
                time::sleep(Duration::from_secs(1)).await;
                continue;
            };

            self.stats.add_execution_time(exec_time);
            // TODO: record and get timestamp of when the transaction was finalized
            // self.stats.add_time_to_finalize(finalize_time);

            let finalize = result.finalize;
            if !finalize.is_full_accept() {
                return Err(anyhow::anyhow!(
                    "Transaction failed: {:?}",
                    finalize.result.full_reject().unwrap()
                ));
            }

            self.stats
                .add_substate_created(finalize.result.accept().unwrap().up_len());

            return Ok(finalize);
        }
    }

    pub fn log_stats(&self) {
        info!("Stats:");
        info!("  - Num transactions: {}", self.stats.num_transactions());
        info!("  - Total execution time: {:.2?}", self.stats.total_execution_time());
        // info!("  - Total time to finalize: {:.2?}", self.stats.total_time_to_finalize());
        info!("  - Num substates created: {}", self.stats.num_substates_created());
    }
}

fn initialize_wallet_sdk<P: AsRef<Path>>(db_path: P, indexer_url: Url) -> Result<WalletSdk, anyhow::Error> {
    let store = SqliteWalletStore::try_open(db_path)?;
    store.run_migrations()?;

    let sdk_config = WalletSdkConfig {
        password: None,
        jwt_expiry: Duration::from_secs(100_000),
        jwt_secret_key: "secret".to_string(),
    };
    let indexer = IndexerJsonRpcNetworkInterface::new(indexer_url);
    let wallet = DanWalletSdk::initialize(store, indexer, sdk_config)?;
    Ok(wallet)
}
