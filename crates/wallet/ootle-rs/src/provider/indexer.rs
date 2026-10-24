//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

use tari_indexer_client::{
    error::IndexerRestClientError,
    rest_api_client::IndexerRestApiClient,
    types::{GetUtxosRequest, SubmitTransactionRequest},
};
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    engine_types::{
        Utxo,
        commit_result::ExecuteResult,
        resource::Resource,
        substate::{Substate, SubstateId},
    },
    optional::Optional,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, UnsignedTransaction};
use tari_template_lib_types::{
    ResourceAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};
use tracing::{debug, warn};
use url::Url;

use crate::{
    Address,
    Network,
    provider::{
        PendingTransaction,
        Provider,
        ProviderError,
        ProviderResult,
        ShardStateVersionBound,
        StealthUtxoStream,
        StealthUtxoWatchRequest,
        TransactionEventFilter,
        TransactionEventStream,
        TransactionWatcher,
        TrustedValidatorSet,
        WalletProvider,
        WantInput,
        input_resolver::TransactionInputResolver,
        tx_stream::{EventStream, Paused},
        tx_watcher::TransactionWatcherHandle,
        verifier::{IndexerTrust, IndexerVerifier, SavedValidatorSet},
    },
    wallet::NetworkWallet,
};

#[derive(Debug, Clone)]
pub struct IndexerProvider<Wallet> {
    client: Arc<IndexerRestApiClient>,
    wallet: Wallet,
    network: Network,
    tx_timeout: Duration,
    tx_watcher: Arc<OnceLock<TransactionWatcherHandle>>,
    verifier: Arc<IndexerVerifier>,
}

impl<Wallet> IndexerProvider<Wallet> {
    /// A provider that verifies what the indexer serves, as [`IndexerTrust::Verify`] describes.
    pub fn new(client: IndexerRestApiClient, wallet: Wallet, network: Network) -> Self {
        let client = Arc::new(client);
        let verifier = Arc::new(IndexerVerifier::new(
            Arc::downgrade(&client),
            network,
            IndexerTrust::Verify,
        ));
        Self {
            client,
            wallet,
            network,
            tx_timeout: Duration::from_secs(60),
            tx_watcher: Arc::new(OnceLock::new()),
            verifier,
        }
    }

    /// Whether this provider verifies what the indexer serves. Clones of a provider share the validator sets they
    /// trust, so this is set before the provider is first cloned or used.
    pub fn with_indexer_trust(mut self, trust: IndexerTrust) -> Self {
        self.verifier = Arc::new(IndexerVerifier::new(Arc::downgrade(&self.client), self.network, trust));
        self
    }

    pub fn indexer_trust(&self) -> IndexerTrust {
        self.verifier.trust()
    }

    /// This provider's trust in the network's validator sets, or `None` before it first anchors. An application that
    /// saves it and passes it to [`restore_validator_set`](Self::restore_validator_set) on its next run walks on from
    /// it, where it would otherwise trust its indexer anew.
    pub async fn saved_validator_set(&self) -> Option<SavedValidatorSet> {
        self.verifier.saved_validator_set().await
    }

    /// Trusts `saved`, such as an earlier run's trust, in place of the validator sets trusted before. The next
    /// verification walks the end-of-epoch handovers on from it.
    pub async fn restore_validator_set(&self, saved: SavedValidatorSet) -> ProviderResult<()> {
        Ok(self.verifier.restore_validator_set(saved).await?)
    }

    /// Brings the trusted validator set up to the one the indexer's latest checkpoint names, anchoring on that set on
    /// first use. Verification does this as it needs; calling it checks that the indexer serves a walkable chain.
    pub async fn sync_validator_set(&self) -> ProviderResult<TrustedValidatorSet> {
        Ok(self.verifier.sync().await?)
    }

    pub(crate) fn verifier(&self) -> &Arc<IndexerVerifier> {
        &self.verifier
    }

    /// The default timeout when waiting for a transaction to be finalized. This is used by the `PendingTransaction`
    /// returned by `send_transaction`.
    pub fn with_transaction_timeout(mut self, timeout: Duration) -> Self {
        self.tx_timeout = timeout;
        self
    }

    /// The indexer URL this provider currently sends requests to.
    pub fn indexer_url(&self) -> Url {
        self.client.endpoint()
    }

    /// Sends all subsequent requests from this provider, its clones and the streams it created to the indexer at
    /// `url`. This is the hook for applications that implement their own indexer failover. The provider keeps the
    /// network it was built for, so `url` must name an indexer on that same network.
    ///
    /// Pending transactions keep being watched: the finalization stream reconnects to the new indexer. A transaction
    /// that finalizes during the reconnect is resolved by the direct result query that
    /// [`PendingTransaction::watch`] makes once its timeout elapses. Streams
    /// from [`watch_events`](Self::watch_events) end with
    /// [`EventWatcherError::EndpointChanged`](crate::provider::EventWatcherError::EndpointChanged), because event ids
    /// are assigned by each indexer and cannot resume a stream on another one. A
    /// [`watch_stealth_utxos`](Self::watch_stealth_utxos) pass in flight finishes against the previous indexer; its
    /// shard cursor is valid on any indexer, so the next pass picks up the new one.
    pub fn set_indexer_url<T: AsRef<str>>(&self, url: T) -> Result<(), IndexerRestClientError> {
        self.client.set_endpoint(url.as_ref())
    }

    pub async fn get_network(&self) -> ProviderResult<Network> {
        let resp = self.client.get_network_info().await?;
        Ok(resp.network)
    }

    /// The live substate of `substate_id`, verified unless the provider trusts its indexer.
    pub async fn fetch_substate<T: Into<SubstateId>>(&self, substate_id: T) -> ProviderResult<Substate> {
        Ok(self.verifier.fetch_substate(&substate_id.into()).await?)
    }

    pub async fn get_epoch(&self) -> ProviderResult<Epoch> {
        let resp = self.client.get_network_info().await?;
        Ok(resp.epoch)
    }

    /// The number of preshards the network is partitioned into. Needed to enumerate the full shard
    /// set when resuming a [`ShardCursor`](crate::provider::ShardCursor), since the UTXO update
    /// stream emits nothing for shards with no updates.
    pub async fn get_num_preshards(&self) -> ProviderResult<NumPreshards> {
        let resp = self.client.get_network_sync_state().await?;
        Ok(resp.network_desc.num_preshards)
    }

    /// Subscribe to a filtered stream of template-emitted events via SSE.
    /// Each event is a template `Event` paired with its originating `TransactionId`.
    ///
    /// This is independent from transaction finalization watching (`PendingTransaction`).
    pub fn watch_events(&self, filter: TransactionEventFilter) -> TransactionEventStream {
        TransactionEventStream::new(Arc::downgrade(&self.client), filter)
    }

    /// The live substate of `substate_id`, verified unless the provider trusts its indexer. `None` if the indexer has
    /// none, which no proof shows.
    pub async fn get_substate<T: Into<SubstateId>>(&self, substate_id: T) -> ProviderResult<Option<Substate>> {
        Ok(self.verifier.fetch_substate(&substate_id.into()).await.optional()?)
    }

    /// Fetch a resource by address, verified unless the provider trusts its indexer. The returned [`Resource`] carries
    /// its type, optional view key, divisibility, and total supply.
    pub async fn get_resource(&self, resource_address: ResourceAddress) -> ProviderResult<Resource> {
        self.verifier
            .fetch_substate(&SubstateId::Resource(resource_address))
            .await?
            .into_substate_value()
            .into_resource()
            .ok_or_else(|| ProviderError::other(format!("Expected resource substate for {resource_address}")))
    }

    /// The bound a pass of [`watch_stealth_utxos`](Self::watch_stealth_utxos) moves a
    /// [`ShardCursor`](crate::provider::ShardCursor)'s verified cursors by: each shard's state version in the
    /// latest verified end-of-epoch checkpoint of its shard group, or [`ShardStateVersionBound::Unbounded`] when
    /// the provider trusts its indexer.
    ///
    /// Fails while the checkpoints do not verify, as when the provider anchored on the latest checkpoint and trusts no
    /// validator set of its epoch. A pass can then go on under [`ShardStateVersionBound::default`], which holds its
    /// progress only as claims.
    pub async fn shard_state_version_bound(&self) -> ProviderResult<ShardStateVersionBound> {
        if !self.verifier.is_verifying() {
            return Ok(ShardStateVersionBound::Unbounded);
        }
        Ok(ShardStateVersionBound::Verified(
            self.verifier.verified_shard_state_versions().await?,
        ))
    }

    /// Watch stealth UTXO updates for a resource, resuming from a per-shard cursor.
    ///
    /// Build the request from a [`ShardCursor`](crate::provider::ShardCursor) with
    /// [`ShardCursor::resume`](crate::provider::ShardCursor::resume) under the bound from
    /// [`shard_state_version_bound`](Self::shard_state_version_bound), and advance it under the same bound, so that an
    /// indexer that reports a shard past its real state version hides none of the shard's updates for long.
    ///
    /// Each drained pass yields typed [`StealthUtxoFrame`](crate::provider::StealthUtxoFrame)s. An
    /// `Unspent` frame carries only the `(tag, public_nonce)` fetch key — resolve the commitment and
    /// viewable-balance ciphertext with [`Self::fetch_unspent_utxos`]. Set `unspent_only = false` on
    /// the request to also receive `Spent`/`Burnt` frames.
    pub fn watch_stealth_utxos(&self, request: StealthUtxoWatchRequest) -> StealthUtxoStream {
        StealthUtxoStream::new(Arc::downgrade(&self.client), request)
    }

    /// Fetch full stealth UTXO outputs by their `(tag, public_nonce)` fetch keys (as delivered by
    /// `StealthUtxoFrame::Unspent`). Returns `(UtxoId, Utxo)` pairs including the commitment and the
    /// viewable-balance ciphertext.
    ///
    /// Only currently-unspent outputs are returned — the indexer prunes an output's ciphertext once
    /// it is spent, so a key whose output has since been spent is simply absent from the result.
    /// Requests are automatically chunked to the indexer's per-request limit.
    ///
    /// The outputs are served unproven and are not verified under any [`IndexerTrust`]. Read an output with
    /// [`Self::get_substate`] to verify it.
    pub async fn fetch_unspent_utxos(
        &self,
        resource_address: ResourceAddress,
        tag_and_nonce_pairs: &[(UtxoTag, RistrettoPublicKeyBytes)],
    ) -> ProviderResult<Vec<(UtxoId, Utxo)>> {
        const MAX_PAIRS_PER_REQUEST: usize = 1000;
        if tag_and_nonce_pairs.is_empty() {
            return Ok(Vec::new());
        }
        let mut utxos = Vec::with_capacity(tag_and_nonce_pairs.len());
        for chunk in tag_and_nonce_pairs.chunks(MAX_PAIRS_PER_REQUEST) {
            let resp = self
                .client
                .get_utxos(GetUtxosRequest {
                    resource_address,
                    tag_and_nonce_pairs: chunk.to_vec(),
                })
                .await?;
            for (id, utxo) in resp.utxos {
                let utxo = utxo.decode().map_err(|e| IndexerRestClientError::InvalidResponse {
                    message: format!("The indexer served UTXO {id} that does not decode: {e}"),
                })?;
                utxos.push((id, utxo));
            }
        }
        Ok(utxos)
    }
}

impl<Wallet: NetworkWallet + Send + Sync> IndexerProvider<Wallet> {
    pub async fn sign_and_send_dry_run(&self, unsigned: UnsignedTransaction) -> ProviderResult<ExecuteResult> {
        self.sign_and_send_dry_run_with(self.wallet(), unsigned).await
    }

    pub async fn sign_and_send_dry_run_with<W: NetworkWallet>(
        &self,
        wallet: &W,
        unsigned: UnsignedTransaction,
    ) -> ProviderResult<ExecuteResult> {
        let unsigned = unsigned.with_dry_run(true);
        let transaction = wallet.sign_transaction(unsigned).await?;
        debug!("Submitting dry-run transaction: {}", transaction.calculate_id());
        self.send_dry_run(transaction).await
    }

    pub async fn send_dry_run(&self, tx: Transaction) -> ProviderResult<ExecuteResult> {
        let resp = self
            .client
            .submit_transaction_dry_run(SubmitTransactionRequest {
                transaction: TransactionEnvelope::encode(tx)?,
            })
            .await?;
        let result = resp
            .result
            .decode()
            .map_err(|e| IndexerRestClientError::InvalidResponse {
                message: format!(
                    "The indexer served a dry-run result of transaction {} that does not decode: {e}",
                    resp.transaction_id
                ),
            })?;
        Ok(result)
    }

    pub async fn send_transaction(&mut self, transaction: Transaction) -> ProviderResult<PendingTransaction> {
        debug!("Sending transaction: {}", transaction.calculate_id());
        let envelope = TransactionEnvelope::encode(transaction)?;
        self.send_transaction_envelope(envelope).await
    }

    pub async fn send_transaction_envelope(
        &mut self,
        transaction: TransactionEnvelope,
    ) -> ProviderResult<PendingTransaction> {
        // Start the tx watcher if not already started
        let watcher = self.get_tx_watcher().clone();
        if let Err(err) = self.verifier.anchor_if_untrusted().await {
            warn!("Failed to anchor the trusted validator set before submitting a transaction: {err}");
        }

        let resp = self
            .client
            .submit_transaction(SubmitTransactionRequest { transaction })
            .await?;
        Ok(
            PendingTransaction::new(watcher, self.weak_client(), self.verifier.clone(), resp.transaction_id)
                .with_timeout(self.tx_timeout),
        )
    }

    pub(crate) fn get_tx_watcher(&self) -> &TransactionWatcherHandle {
        self.tx_watcher.get_or_init(|| {
            let paused = Paused::default();
            let event_stream = EventStream::new(self.weak_client(), paused.waiter());
            let watcher = TransactionWatcher::new(Box::pin(event_stream.into_stream()), paused);
            watcher.spawn()
        })
    }
}

impl<Wallet: NetworkWallet + Send + Sync> Provider for IndexerProvider<Wallet> {
    type Client = IndexerRestApiClient;

    fn network(&self) -> Network {
        self.network
    }

    fn weak_client(&self) -> Weak<Self::Client> {
        Arc::downgrade(&self.client)
    }

    fn default_signer_address(&self) -> &Address {
        self.wallet.default_address()
    }

    async fn resolve_input_want_list(
        &self,
        mut transaction: UnsignedTransaction,
        want_list: &HashSet<WantInput>,
    ) -> ProviderResult<UnsignedTransaction> {
        TransactionInputResolver::new(self.verifier.clone())
            .resolve_inputs(&mut transaction, want_list)
            .await?;
        Ok(transaction)
    }

    async fn fetch_substates<I: IntoIterator<Item = SubstateId> + Send>(
        &self,
        substate_ids: I,
    ) -> ProviderResult<HashMap<SubstateId, Substate>> {
        let substate_ids = substate_ids.into_iter().collect::<Vec<_>>();
        Ok(self.verifier.fetch_substates(&substate_ids).await?)
    }
}

impl<Wallet: NetworkWallet + Send + Sync> WalletProvider for IndexerProvider<Wallet> {
    type Wallet = Wallet;

    fn wallet(&self) -> &Self::Wallet {
        &self.wallet
    }

    fn wallet_mut(&mut self) -> &mut Self::Wallet {
        &mut self.wallet
    }
}
