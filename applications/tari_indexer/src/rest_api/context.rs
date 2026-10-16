//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, sync::Arc};

use axum::{
    http,
    response::{IntoResponse, Response},
};
use ootle_byte_type::ToByteType;
use tari_epoch_manager::service::EpochManagerHandle;
use tari_indexer_client::event::{IndexerEvent, TransactionEvent};
use tari_networking::NetworkingHandle;
use tari_ootle_p2p::{PeerAddress, TariMessagingSpec};
use tari_ootle_storage::global::GlobalDb;
use tari_ootle_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_ootle_transaction::Network;
use tari_template_lib_types::{TemplateAddress, crypto::RistrettoPublicKeyBytes};
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;
use tokio::sync::broadcast;

use crate::{
    bootstrap::Services,
    config::PublishedIndexerConfig,
    dry_run::processor::DryRunTransactionProcessor,
    network_state_sync::{ConsensusEpoch, ValidatorStatusMonitor},
    notify::Subscriber,
    rest_api::{
        cache::{HttpCacheConfig, IMMUTABLE},
        committee_cache::CommitteeProofCache,
    },
    storage_sqlite::SqliteIndexerStore,
    store::ReadOnlyStore,
    substate_manager::SubstateManager,
    transaction_manager::TransactionManager,
};

/// Holds the committees of several hundred epochs at a few KB per committee.
const COMMITTEE_PROOF_CACHE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct HandlerContext {
    inner: Arc<InnerContext>,
}

impl HandlerContext {
    pub fn from_services(services: &Services) -> Self {
        Self {
            inner: Arc::new(InnerContext {
                cache_control_enabled: true,
                network: services.network,
                published_config: services.published_config.clone(),
                read_only_store: ReadOnlyStore::new(services.store.clone()),
                global_db: services.global_db.clone(),
                epoch_manager: services.epoch_manager.clone(),
                networking: services.networking.clone(),
                public_key: services.keypair.public_key().to_byte_type(),
                substate_manager: services.substate_manager.clone(),
                transaction_manager: services.transaction_manager.clone(),
                dry_run_transaction_processor: services.dry_run_transaction_processor.clone(),
                subscriber: services.event_notifier.to_subscriber(),
                transaction_event_subscriber: services.transaction_event_notifier.to_subscriber(),
                watched_templates: services.watched_templates.clone(),
                validator_status: services.validator_status.clone(),
                consensus_epoch: services.consensus_epoch.clone(),
                committee_proofs: CommitteeProofCache::new(COMMITTEE_PROOF_CACHE_BYTES),
            }),
        }
    }

    pub fn global_db(&self) -> &GlobalDb<SqliteGlobalDbAdapter<PeerAddress>> {
        &self.inner.global_db
    }

    pub fn network(&self) -> Network {
        self.inner.network
    }

    pub fn published_config(&self) -> &PublishedIndexerConfig {
        &self.inner.published_config
    }

    pub fn epoch_manager(&self) -> &EpochManagerHandle<PeerAddress> {
        &self.inner.epoch_manager
    }

    pub fn networking(&self) -> &NetworkingHandle<TariMessagingSpec> {
        &self.inner.networking
    }

    pub fn public_key(&self) -> &RistrettoPublicKeyBytes {
        &self.inner.public_key
    }

    pub fn is_cache_control_enabled(&self) -> bool {
        self.inner.cache_control_enabled
    }

    pub fn substate_manager(&self) -> &SubstateManager {
        &self.inner.substate_manager
    }

    pub fn transaction_manager(
        &self,
    ) -> &TransactionManager<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, SqliteIndexerStore>
    {
        &self.inner.transaction_manager
    }

    pub fn read_only_store(&self) -> &ReadOnlyStore<SqliteIndexerStore> {
        &self.inner.read_only_store
    }

    pub fn dry_run_transaction_processor(&self) -> &DryRunTransactionProcessor {
        &self.inner.dry_run_transaction_processor
    }

    pub fn apply_cache_control(&self, body: impl IntoResponse, max_age: u32) -> Response {
        let mut response = body.into_response();
        let headers = response.headers_mut();
        self.apply_custom_cache_control(headers, &HttpCacheConfig::new().with_max_age(max_age));
        response
    }

    /// Marks `body` as cacheable forever, for a response whose content can no longer change under its URL.
    pub fn apply_immutable_cache_control(&self, body: impl IntoResponse) -> Response {
        let mut response = body.into_response();
        if self.is_cache_control_enabled() {
            response.headers_mut().insert(http::header::CACHE_CONTROL, IMMUTABLE);
        }
        response
    }

    pub fn apply_custom_cache_control(&self, headers: &mut http::HeaderMap, config: &HttpCacheConfig) {
        if self.is_cache_control_enabled() {
            config.apply(headers);
        }
    }

    pub fn watched_templates(&self) -> &HashSet<TemplateAddress> {
        &self.inner.watched_templates
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<IndexerEvent> {
        self.inner.subscriber.subscribe()
    }

    pub fn subscribe_transaction_events(&self) -> broadcast::Receiver<TransactionEvent> {
        self.inner.transaction_event_subscriber.subscribe()
    }

    pub fn validator_status(&self) -> &ValidatorStatusMonitor {
        &self.inner.validator_status
    }

    pub fn consensus_epoch(&self) -> &ConsensusEpoch {
        &self.inner.consensus_epoch
    }

    pub fn committee_proofs(&self) -> &CommitteeProofCache {
        &self.inner.committee_proofs
    }
}

struct InnerContext {
    cache_control_enabled: bool,
    network: Network,
    published_config: PublishedIndexerConfig,
    global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    networking: NetworkingHandle<TariMessagingSpec>,
    public_key: RistrettoPublicKeyBytes,
    substate_manager: SubstateManager,
    transaction_manager:
        TransactionManager<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, SqliteIndexerStore>,
    read_only_store: ReadOnlyStore<SqliteIndexerStore>,
    dry_run_transaction_processor: DryRunTransactionProcessor,
    subscriber: Subscriber<IndexerEvent>,
    transaction_event_subscriber: Subscriber<TransactionEvent>,
    watched_templates: Arc<HashSet<TemplateAddress>>,
    validator_status: ValidatorStatusMonitor,
    consensus_epoch: ConsensusEpoch,
    committee_proofs: CommitteeProofCache,
}
