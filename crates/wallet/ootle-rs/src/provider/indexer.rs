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
    types::{GetSubstateRequest, GetSubstateResponse, GetSubstatesRequest, GetUtxosRequest, SubmitTransactionRequest},
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
use tracing::debug;
use url::Url;

use crate::{
    Address,
    Network,
    provider::{
        PendingTransaction,
        Provider,
        ProviderError,
        ProviderResult,
        StealthUtxoStream,
        StealthUtxoWatchRequest,
        TransactionEventFilter,
        TransactionEventStream,
        TransactionWatcher,
        WalletProvider,
        WantInput,
        input_resolver::TransactionInputResolver,
        tx_stream::{EventStream, Paused},
        tx_watcher::TransactionWatcherHandle,
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
}

impl<Wallet> IndexerProvider<Wallet> {
    pub fn new(client: IndexerRestApiClient, wallet: Wallet, network: Network) -> Self {
        Self {
            client: Arc::new(client),
            wallet,
            network,
            tx_timeout: Duration::from_secs(60),
            tx_watcher: Arc::new(OnceLock::new()),
        }
    }

    /// The default timeout when waiting for a transaction to be finalized. This is used by the `PendingTransaction`
    /// returned by `send_transaction`.
    pub fn with_transaction_timeout(mut self, timeout: Duration) -> Self {
        self.tx_timeout = timeout;
        self
    }

    pub(crate) fn client(&self) -> &IndexerRestApiClient {
        &self.client
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

    pub async fn fetch_substate<T: Into<SubstateId>>(&self, substate_id: T) -> ProviderResult<Substate> {
        let resp = self
            .client
            .get_substate(&substate_id.into(), GetSubstateRequest::default())
            .await?;
        Ok(up_substate(resp)?)
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

    pub async fn get_substate<T: Into<SubstateId>>(&self, substate_id: T) -> ProviderResult<Option<Substate>> {
        let resp = self
            .client
            .get_substate(&substate_id.into(), GetSubstateRequest::default())
            .await
            .optional()?;
        Ok(resp.map(up_substate).transpose()?)
    }

    /// Fetch a resource by address. The returned [`Resource`] carries its type, optional view key,
    /// divisibility, and total supply.
    pub async fn get_resource(&self, resource_address: ResourceAddress) -> ProviderResult<Resource> {
        let resp = self.client.get_resource(resource_address).await?;
        Ok(resp.resource)
    }

    /// Watch stealth UTXO updates for a resource, resuming from a per-shard cursor.
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
            utxos.extend(resp.utxos);
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
        Ok(resp.result)
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

        let resp = self
            .client
            .submit_transaction(SubmitTransactionRequest { transaction })
            .await?;
        Ok(PendingTransaction::new(watcher, self.weak_client(), resp.transaction_id).with_timeout(self.tx_timeout))
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
        TransactionInputResolver::new(self.weak_client())
            .resolve_inputs(&mut transaction, want_list)
            .await?;
        Ok(transaction)
    }

    async fn fetch_substates<I: IntoIterator<Item = SubstateId> + Send>(
        &self,
        substate_ids: I,
    ) -> ProviderResult<HashMap<SubstateId, Substate>> {
        let substate_ids = substate_ids.into_iter().collect::<Vec<_>>();
        if substate_ids.is_empty() {
            // The request API will not allow us to send an empty list, so we short-circuit here
            return Ok(HashMap::new());
        }

        let resp = self
            .client
            .fetch_substates(GetSubstatesRequest {
                requests: substate_ids
                    .try_into()
                    .map_err(|_| ProviderError::other("Too many substates requested in single request"))?,
                cached_only: false,
                include_proofs: false,
            })
            .await?;

        Ok(resp.substates)
    }
}

/// The live substate a lookup without proofs answers with. The indexer answers such a lookup with a down version only
/// in breach of its API.
fn up_substate(resp: GetSubstateResponse) -> Result<Substate, IndexerRestClientError> {
    let version = resp.version;
    resp.into_substate_value()
        .map(|value| Substate::new(version, value))
        .ok_or_else(|| IndexerRestClientError::InvalidResponse {
            message: format!("Indexer answered v{version} as down to a request without proofs"),
        })
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
