//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_indexer_client::error::IndexerRestClientError;
use tari_ootle_address::Network;

use crate::{
    provider::{IndexerTrust, indexer::IndexerProvider},
    wallet::{NetworkWallet, NoWallet},
};

/// Builder for constructing a [`Provider`](super::Provider) connected to an Ootle indexer.
///
/// # Example
///
/// ```rust,ignore
/// let provider = ProviderBuilder::new()
///     .wallet(wallet)
///     .connect("http://127.0.0.1:12500")
///     .await?;
/// ```
#[derive(Debug)]
pub struct ProviderBuilder<Wallet = NoWallet> {
    wallet: Wallet,
    network: Network,
    indexer_trust: IndexerTrust,
}

impl ProviderBuilder<NoWallet> {
    pub fn new() -> Self {
        Self {
            network: Network::MainNet,
            wallet: NoWallet,
            indexer_trust: IndexerTrust::Verify,
        }
    }

    pub fn with_network(mut self, network: Network) -> Self {
        self.network = network;
        self
    }
}

impl<Wallet> ProviderBuilder<Wallet> {
    pub fn wallet<W: NetworkWallet>(self, wallet: W) -> ProviderBuilder<W> {
        ProviderBuilder {
            network: wallet.default_address().network(),
            wallet,
            indexer_trust: self.indexer_trust,
        }
    }

    /// Whether the provider verifies what the indexer serves. Defaults to [`IndexerTrust::Verify`].
    pub fn indexer_trust(mut self, indexer_trust: IndexerTrust) -> Self {
        self.indexer_trust = indexer_trust;
        self
    }

    pub async fn connect<T: AsRef<str>>(self, url: T) -> Result<IndexerProvider<Wallet>, IndexerRestClientError> {
        let client = tari_indexer_client::connect_rest(url.as_ref())?;
        Ok(IndexerProvider::new(client, self.wallet, self.network).with_indexer_trust(self.indexer_trust))
    }

    /// Connects to the indexer with a custom transaction timeout. This is the amount of time that the provider will
    /// wait for a transaction to be finalized before considering it failed. The default is 32 seconds,
    /// It is not recommended to set this to a value lower than 30 seconds.
    pub async fn connect_with_transaction_timeout<T: AsRef<str>>(
        self,
        url: T,
        tx_timeout: Duration,
    ) -> Result<IndexerProvider<Wallet>, IndexerRestClientError> {
        let client = tari_indexer_client::connect_rest(url.as_ref())?;
        Ok(IndexerProvider::new(client, self.wallet, self.network)
            .with_indexer_trust(self.indexer_trust)
            .with_transaction_timeout(tx_timeout))
    }
}

impl Default for ProviderBuilder<NoWallet> {
    fn default() -> Self {
        Self::new()
    }
}
