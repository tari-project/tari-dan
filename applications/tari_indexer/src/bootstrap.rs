//   Copyright 2023. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, anyhow};
use libp2p::identity;
use log::info;
use ootle_byte_type::ToByteType;
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common::configuration::bootstrap::{ApplicationType, grpc_default_port};
use tari_consensus::consensus_constants::ConsensusConstants;
use tari_crypto::tari_utilities::ByteArray;
use tari_engine::wasm::WasmModuleCache;
use tari_engine_types::{calculate_template_binary_hash, published_template::PublishedTemplateMetadata};
use tari_epoch_manager::service::{EpochManagerConfig, EpochManagerHandle};
use tari_epoch_oracles::{
    EpochOracle,
    base_layer::{
        BaseLayerBlockHeaderStore,
        BaseLayerEpochOracleConfig,
        BaseLayerEpochOracleFeatures,
        BaseLayerOracle,
    },
    configured::{ConfiguredEpochOracle, RealTimeEpochTicker},
    hybrid::{HybridEpochOracle, mpsc_ticker},
    store::EpochOracleStore,
};
use tari_indexer_client::event::{IndexerEvent, TransactionEvent};
use tari_indexer_lib::cached_substate_manager::DEFAULT_CACHE_TTL;
use tari_networking::{
    MessagingMode,
    NetworkingHandle,
    RelayCircuitLimits,
    RelayReservationLimits,
    SwarmConfig,
    gossip_queue,
};
use tari_ootle_app_utilities::{
    claim_burn_proof_verifier::KnowledgeProofVerifier,
    configuration::convert_network_to_l1_network,
    epoch_oracle_config::EpochOracleType,
    fee_tables::get_fee_table_by_network,
    identity_management,
    keypair::RistrettoKeypair,
    protocol_activation::check_and_record_activation_schedule,
    seed_peer::SeedPeer,
    shared_consts::TXTR_FAUCET_INITIAL_SUPPLY,
};
use tari_ootle_common_types::optional::Optional;
use tari_ootle_p2p::{PeerAddress, TRANSACTION_TOPIC, TariMessagingSpec};
use tari_ootle_storage::global::GlobalDb;
use tari_ootle_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_ootle_transaction::Network;
use tari_ootle_transaction_validation::create_gossip_transaction_validator;
use tari_shutdown::ShutdownSignal;
use tari_template_builtin::all_builtin_templates;
use tari_template_lib_types::{Amount, TemplateAddress, crypto::RistrettoPublicKeyBytes};
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;
use tokio::task;

use crate::{
    ApplicationConfig,
    IndexerEpochManagerSpec,
    Noop,
    base_layer::verify_correct_network,
    config::PublishedIndexerConfig,
    dry_run::processor::DryRunTransactionProcessor,
    network_client::TariNetworkClient,
    network_state_sync,
    network_state_sync::{NetworkWideStateSyncConfig, ValidatorStatusMonitor},
    notify::Notify,
    storage_sqlite::{SqliteIndexerStore, models::Key},
    store::{IndexerStore, IndexerStoreReadTransaction, IndexerStoreWriteTransaction},
    substate_cache::SqliteSubstateCache,
    substate_manager::SubstateManager,
    template_manager::TemplateManager,
    transaction_gossip::TransactionGossipService,
    transaction_manager::TransactionManager,
    transaction_pruner::TransactionPruner,
};

const LOG_TARGET: &str = "tari::indexer::bootstrap";

/// How long a substate stays journalled as recently changed. The journal exists only to stop a
/// committee fetch that started before a transition arrived from writing its result back as current,
/// so it needs to outlive an in-flight fetch and nothing more.
const SUBSTATE_CACHE_JOURNAL_RETENTION: Duration = Duration::from_secs(300);

const SUBSTATE_CACHE_PRUNE_INTERVAL: Duration = Duration::from_secs(60);

/// Keepalives a shard's stream may miss before a record that a substate does not exist stops being
/// served for it.
///
/// Derived from the keepalive interval rather than configured: the bound is not a staleness budget
/// but a liveness test - a negative is only as true as the stream that would retract it - and a
/// standalone setting could be left behind when that interval changes. Two may be lost before the
/// cache closes, which is enough to ride out an ordinary reopen without holding a dead stream open.
const NEGATIVE_SERVE_KEEPALIVES: u32 = 3;

#[allow(clippy::too_many_lines)]
pub async fn spawn_services(
    config: &ApplicationConfig,
    shutdown: ShutdownSignal,
    keypair: RistrettoKeypair,
    global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    consensus_constants: ConsensusConstants,
    #[cfg(feature = "metrics")] metrics_registry: &mut prometheus_client::registry::Registry,
) -> Result<Services, anyhow::Error> {
    ensure_directories_exist(config)?;

    // Initialize networking
    let identity = identity::Keypair::sr25519_from_bytes(keypair.secret_key().as_bytes().to_vec())
        .context("Failed to create libp2p identity from secret bytes")?;
    let seed_peers = config
        .peer_seeds
        .peer_seeds
        .iter()
        .map(|s| SeedPeer::from_str(s))
        .collect::<anyhow::Result<Vec<SeedPeer>>>()?;
    let seed_peers = seed_peers
        .into_iter()
        .map(|p| {
            let peer_id = p.to_peer_id();
            (peer_id, p.into_address())
        })
        .collect();

    // The indexer sends no direct messages and subscribes to one gossip topic. When gossip indexing
    // is off it stays out of the mesh entirely rather than joining it and forwarding nothing.
    let (messaging_mode, rx_transaction_gossip) = if config.indexer.index_gossiped_transactions {
        const MAX_QUEUED_INBOUND_MESSAGES: usize = 100_000;
        let (tx_transaction_gossip, rx_transaction_gossip) = gossip_queue(
            MAX_QUEUED_INBOUND_MESSAGES,
            config.indexer.max_transaction_gossip_queue_bytes,
        );
        #[cfg(feature = "metrics")]
        crate::transaction_gossip::TransactionGossipQueueCollector::new(tx_transaction_gossip.clone())
            .register(metrics_registry);

        let tx_gossip_messages_by_topic = HashMap::from([(TRANSACTION_TOPIC.to_string(), tx_transaction_gossip)]);
        (
            MessagingMode::GossipOnly {
                tx_gossip_messages_by_topic,
            },
            Some(rx_transaction_gossip),
        )
    } else {
        (MessagingMode::Disabled, None)
    };

    let network_builder = tari_networking::Builder::<TariMessagingSpec>::new(identity)
        .with_messaging_mode(messaging_mode)
        .with_config(tari_networking::Config {
            // TODO: configurable
            listeners: vec![
                format!("/ip4/0.0.0.0/tcp/{}", config.indexer.p2p.listener_port)
                    .parse()
                    .expect("Failed to parse listener address"),
                format!("/ip4/0.0.0.0/udp/{}/quic-v1", config.indexer.p2p.listener_port)
                    .parse()
                    .expect("Failed to parse listener address"),
            ],
            swarm: SwarmConfig {
                protocol_version: format!("/tari/{}/0.0.1", config.network)
                    .parse()
                    .expect("Failed to parse protocol version"),
                user_agent: format!("/tari/indexer/{}", env!("CARGO_PKG_VERSION")),
                enable_mdns: config.indexer.p2p.enable_mdns,
                enable_relay: config.indexer.p2p.enable_relay,
                // The indexer reports a `Reject` verdict for messages that fail to decode or fail
                // validation, so it can score the peers that send them.
                gossip_sub_scored_topics: vec![TRANSACTION_TOPIC.to_string()],
                relay_circuit_limits: RelayCircuitLimits::high(),
                relay_reservation_limits: RelayReservationLimits::high(),
                rendezvous_server_enabled: config.indexer.p2p.enable_rendezvous,
                ..Default::default()
            },
            reachability_mode: config.indexer.p2p.reachability_mode.into(),
            announce: false,
            rendezvous_namespace: format!("tari-{}", config.network.to_string().to_lowercase()),
            ..Default::default()
        })
        .with_seed_peers(seed_peers);
    #[cfg(feature = "metrics")]
    let network_builder = network_builder.with_metrics(metrics_registry);

    let (networking, _) = network_builder.spawn(shutdown.clone())?;

    // Connect to substate db
    let store = SqliteIndexerStore::try_create(config.state_db_path())?;
    check_store(config, &store).await?;

    seed_builtin_template_catalogue(&store).await?;

    if config.network.is_testnet() {
        // TODO: We happen to know that the testnet faucet mints u64::MAX tXTR. This is hacky.
        hack_xtr_initial_supply(&store).await?;
    }

    check_and_record_activation_schedule(
        config.network,
        &global_db,
        config.indexer.allow_past_protocol_activation,
    )?;

    // Epoch event oracle
    let epoch_event_oracle = create_epoch_oracle(config, global_db.clone(), &consensus_constants).await?;

    // Epoch manager
    let (epoch_manager, _) = tari_epoch_manager::service::spawn_service::<IndexerEpochManagerSpec>(
        EpochManagerConfig {
            num_preshards: consensus_constants.num_preshards,
            base_layer_confirmations: consensus_constants.base_layer_confirmations,
            committee_size: consensus_constants
                .committee_size_per_shard_group
                .try_into()
                .context("committee_size must be non-zero")?,
            validator_node_sidechain_id: config.indexer.sidechain_id.as_ref().map(|pk| pk.to_byte_type()),
            // N/A
            fee_claim_public_key: RistrettoPublicKeyBytes::zero(),
        },
        global_db.clone(),
        keypair.public_key().to_byte_type(),
        epoch_event_oracle,
        Noop,
        shutdown.clone(),
    );

    // Client factory
    let validator_node_client_factory = TariValidatorNodeRpcClientFactory::new(networking.clone());
    let network_client = TariNetworkClient::new(
        epoch_manager.clone(),
        validator_node_client_factory.clone(),
        consensus_constants.num_preshards,
    );

    let watched_templates: Arc<HashSet<TemplateAddress>> =
        Arc::new(config.indexer.watched_templates.iter().copied().collect());

    let event_notifier = Notify::new(1000);
    let transaction_event_notifier = Notify::new(1024);
    let validator_status = ValidatorStatusMonitor::new(epoch_manager.clone());

    #[cfg(feature = "metrics")]
    let network_state_metrics = network_state_sync::NetworkStateMetrics::register(metrics_registry);

    // Shared between the state sync (which confirms how far each shard is synced) and the substate
    // cache (which serves an entry only while its shard is being kept up with).
    let shard_watermarks = Arc::new(network_state_sync::ShardWatermarks::new());

    network_state_sync::NetworkWideStateSync::new(
        config.network,
        epoch_manager.clone(),
        networking.clone(),
        store.clone(),
        NetworkWideStateSyncConfig {
            event_filters: Arc::from(config.indexer.event_filters.clone()),
            work_interval: config.indexer.state_scanning_interval,
            stream_deadline: config.indexer.state_sync_stream_deadline,
            keepalive_interval: config.indexer.state_sync_keepalive_interval,
            watched_templates: watched_templates.clone(),
        },
        event_notifier.clone(),
        transaction_event_notifier.clone(),
        validator_status.clone(),
        shard_watermarks.clone(),
        #[cfg(feature = "metrics")]
        network_state_metrics,
        #[cfg(feature = "metrics")]
        consensus_constants.clone(),
    )
    .spawn(shutdown.clone());

    // Substate manager
    let substate_cache = SqliteSubstateCache::new(
        store.clone(),
        shard_watermarks,
        config.indexer.substate_cache_max_serve_lag,
        config.indexer.state_sync_keepalive_interval * NEGATIVE_SERVE_KEEPALIVES,
        SUBSTATE_CACHE_JOURNAL_RETENTION,
        DEFAULT_CACHE_TTL,
        config.indexer.substate_cache_max_entries,
    );
    substate_cache.spawn_pruner(SUBSTATE_CACHE_PRUNE_INTERVAL, shutdown.clone());
    let substate_manager = SubstateManager::new(
        config.network,
        store.clone(),
        epoch_manager.clone(),
        validator_node_client_factory.clone(),
        substate_cache.clone(),
    )
    .with_substate_proof_verification(config.indexer.verify_substate_proofs)
    .with_negative_cache_ttl(config.indexer.substate_cache_negative_ttl);
    #[cfg(feature = "metrics")]
    let substate_manager = substate_manager.with_metrics(metrics_registry);

    // Template manager
    let wasm_cache_dir = config.to_data_dir().join("wasm_cache");

    let template_manager = task::spawn_blocking({
        let global_db = global_db.clone();
        let substate_manager = substate_manager.clone();
        let wasm_cache_dir = wasm_cache_dir.clone();
        move || {
            let wasm_cache = WasmModuleCache::open(&wasm_cache_dir).map_err(|e| {
                anyhow!(
                    "Failed to open WASM module cache at {}: {}",
                    wasm_cache_dir.display(),
                    e,
                )
            })?;
            let manager = TemplateManager::initialize(global_db, substate_manager, wasm_cache)?;
            anyhow::Ok(manager)
        }
    })
    .await
    .context("template manager init thread panicked")??;

    // Dry run - use a shorter cache TTL for more accurate fee estimates. A nonexistent input is held
    // for the shorter of the two: an input resolved as absent estimates a transaction that would in
    // fact have succeeded, which is the same staleness `dry_run_cache_ttl` is set to bound.
    // Proof verification is left off here: dry run only produces a fee estimate, and gating it on
    // proof availability would make transaction submission fragile when proofs are momentarily
    // unavailable.
    let dry_run_substate_manager = substate_manager
        .clone()
        .with_cache_ttl(config.indexer.dry_run_cache_ttl)
        .with_negative_cache_ttl(
            config
                .indexer
                .dry_run_cache_ttl
                .min(config.indexer.substate_cache_negative_ttl),
        )
        .with_substate_proof_verification(false);
    let fee_table = get_fee_table_by_network(config.network);
    let dry_run_transaction_processor = DryRunTransactionProcessor::new(
        config.network,
        fee_table.clone(),
        epoch_manager.clone(),
        dry_run_substate_manager,
        wasm_cache_dir,
        // We do not verify the kernel merkle proof, since that requires syncing L1 headers
        // TODO: maybe at least validate the well-formedness of the proof
        KnowledgeProofVerifier::new(
            config.network,
            config.indexer.sidechain_id.as_ref().map(|p| p.to_byte_type()),
        ),
        // The processor resolves the exhaust burn rate for the current epoch on each dry-run estimate.
        consensus_constants.clone(),
    )?;

    // Both of these have defaults that decide how much disk this node uses and what it deletes, so
    // they are logged unconditionally: an operator who upgraded without touching their config should
    // be able to see what changed underneath them in their own logs.
    info!(
        target: LOG_TARGET,
        "⚙️ Transaction storage: gossip indexing {}, retention {}",
        if config.indexer.index_gossiped_transactions { "ON" } else { "OFF" },
        config
            .indexer
            .transaction_retention_epochs
            .map_or_else(|| "forever".to_string(), |epochs| format!("{epochs} epoch(s)")),
    );

    if let Some(retention_epochs) = config.indexer.transaction_retention_epochs {
        info!(
            target: LOG_TARGET,
            "🧹 Pruning transactions more than {} epoch(s) behind, checked every {:.0?}",
            retention_epochs,
            config.indexer.transaction_prune_interval,
        );
        TransactionPruner::new(
            store.clone(),
            epoch_manager.clone(),
            retention_epochs,
            config.indexer.transaction_prune_interval,
        )
        .spawn(shutdown.clone());
    }

    if let Some(rx_transaction_gossip) = rx_transaction_gossip {
        info!(target: LOG_TARGET, "📥 Indexing transactions from network gossip");
        TransactionGossipService::new(
            store.clone(),
            epoch_manager.clone(),
            create_gossip_transaction_validator(
                config.network,
                consensus_constants.max_transaction_weight,
                consensus_constants.max_transaction_validity_epochs,
            ),
            consensus_constants.max_transaction_validity_epochs,
            networking.clone(),
            rx_transaction_gossip,
            #[cfg(feature = "metrics")]
            crate::transaction_gossip::TransactionGossipMetrics::register(metrics_registry),
        )
        .spawn(shutdown.clone());
    }

    let transaction_manager = TransactionManager::new(
        network_client.clone(),
        store.clone(),
        config.network,
        consensus_constants.max_transaction_weight,
        consensus_constants.max_transaction_validity_epochs,
        substate_cache,
    );

    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    save_identities(config, &keypair)?;
    Ok(Services {
        network: config.network,
        consensus_constants: consensus_constants.clone(),
        published_config: PublishedIndexerConfig::from(&config.indexer),
        keypair,
        networking,
        epoch_manager,
        _validator_node_client_factory: validator_node_client_factory,
        store,
        global_db,
        template_manager,
        substate_manager,
        transaction_manager,
        dry_run_transaction_processor,
        event_notifier,
        transaction_event_notifier,
        watched_templates,
        validator_status,
    })
}

pub struct Services {
    pub network: Network,
    pub consensus_constants: ConsensusConstants,
    pub published_config: PublishedIndexerConfig,
    pub keypair: RistrettoKeypair,
    pub networking: NetworkingHandle<TariMessagingSpec>,
    pub epoch_manager: EpochManagerHandle<PeerAddress>,
    pub _validator_node_client_factory: TariValidatorNodeRpcClientFactory,
    pub store: SqliteIndexerStore,
    pub global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    pub template_manager: TemplateManager,
    pub substate_manager: SubstateManager,
    pub transaction_manager:
        TransactionManager<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, SqliteIndexerStore>,
    pub dry_run_transaction_processor: DryRunTransactionProcessor,
    pub event_notifier: Notify<IndexerEvent>,
    pub transaction_event_notifier: Notify<TransactionEvent>,
    pub watched_templates: Arc<HashSet<TemplateAddress>>,
    pub validator_status: ValidatorStatusMonitor,
}

fn ensure_directories_exist(config: &ApplicationConfig) -> io::Result<()> {
    fs::create_dir_all(config.to_data_dir())?;
    Ok(())
}

fn save_identities(config: &ApplicationConfig, identity: &RistrettoKeypair) -> anyhow::Result<()> {
    identity_management::save_as_json(config.to_identity_file_path(), identity)
        .context("Failed to save indexer identity")?;

    Ok(())
}

async fn create_base_layer_client(config: &ApplicationConfig) -> anyhow::Result<GrpcBaseNodeClient> {
    let url = config
        .epoch_oracle
        .base_layer
        .base_node_grpc_url
        .clone()
        .unwrap_or_else(|| {
            let port = grpc_default_port(
                ApplicationType::BaseNode,
                convert_network_to_l1_network(&config.network),
            );
            format!("http://127.0.0.1:{port}")
                .parse()
                .expect("Default base node GRPC URL is malformed")
        });
    GrpcBaseNodeClient::connect(url)
        .await
        .context("Failed to create gRPC base node client")
}

async fn create_epoch_oracle<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Clone + Send + 'static>(
    config: &ApplicationConfig,
    store: TStore,
    consensus_constants: &ConsensusConstants,
) -> anyhow::Result<EpochOracle<TStore>> {
    match config.epoch_oracle.oracle_type {
        EpochOracleType::BaseLayer => {
            let oracle = create_base_layer_epoch_oracle(config, store, consensus_constants).await?;
            Ok(EpochOracle::BaseLayer(oracle))
        },
        EpochOracleType::Configured => {
            let oracle = create_configured_epoch_oracle(config, store).await?;
            Ok(EpochOracle::Configured(oracle))
        },
        EpochOracleType::Hybrid => {
            let oracle = create_hybrid_epoch_oracle(config, store, consensus_constants).await?;
            Ok(EpochOracle::Hybrid(oracle))
        },
    }
}

async fn create_base_layer_epoch_oracle<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + 'static>(
    config: &ApplicationConfig,
    store: TStore,
    consensus_constants: &ConsensusConstants,
) -> anyhow::Result<BaseLayerOracle<TStore>> {
    let mut base_node_client = create_base_layer_client(config).await?;
    verify_correct_network(&mut base_node_client, config.network).await?;
    Ok(BaseLayerOracle::new(
        store,
        base_node_client,
        BaseLayerEpochOracleConfig {
            start_height: config.epoch_oracle.base_layer.start_height,
            height_lag: consensus_constants.base_layer_confirmations,
            scanning_interval: config.epoch_oracle.base_layer.scanning_interval,
            sidechain_id: config.indexer.sidechain_id.as_ref().map(|p| p.to_byte_type()),
            features: BaseLayerEpochOracleFeatures {
                sync_headers: false,
                sync_validator_node_changes: true,
            },
            epoch_end_spread_blocks: consensus_constants.epoch_end_spread_blocks,
        },
        config.network,
    ))
}

async fn create_configured_epoch_oracle<TStore: EpochOracleStore + Send>(
    config: &ApplicationConfig,
    store: TStore,
) -> anyhow::Result<ConfiguredEpochOracle<TStore, RealTimeEpochTicker>> {
    let oracle_config = config.epoch_oracle.configured.load().await?;
    let oracle = ConfiguredEpochOracle::create(oracle_config, store)?;
    Ok(oracle)
}

async fn create_hybrid_epoch_oracle<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Clone + Send + 'static>(
    config: &ApplicationConfig,
    store: TStore,
    consensus_constants: &ConsensusConstants,
) -> anyhow::Result<HybridEpochOracle<TStore>> {
    let base_layer_oracle = create_base_layer_epoch_oracle(config, store.clone(), consensus_constants).await?;
    let oracle_config = config.epoch_oracle.configured.load().await?;
    let (ticker, trigger) = mpsc_ticker();
    let configured_oracle = ConfiguredEpochOracle::with_custom_ticker(oracle_config, store, ticker);
    Ok(HybridEpochOracle::new(configured_oracle, base_layer_oracle, trigger))
}

async fn check_store<TStore: IndexerStore>(config: &ApplicationConfig, store: &TStore) -> anyhow::Result<()> {
    let configured_network = config.network;
    store
        .with_write_tx(move |tx| {
            match tx.key_value_get_value::<_, Network>(Key::Network).optional()? {
                Some(network) => {
                    if network != configured_network {
                        return Err(anyhow!(
                            "The network in the database ({}) does not match the configured network ({})",
                            network,
                            configured_network
                        ));
                    }
                    Ok(())
                },
                None => {
                    // If the network is not set, we can assume this is a new store and we can set it
                    tx.key_value_set(Key::Network, configured_network)
                        .map_err(|e| anyhow!("Failed to set network in the store: {}", e))
                },
            }
        })
        .await
}

async fn seed_builtin_template_catalogue<TStore: IndexerStore>(store: &TStore) -> anyhow::Result<()> {
    store
        .with_write_tx(|tx| {
            for template in all_builtin_templates() {
                let binary_hash = calculate_template_binary_hash(template.binary);
                let metadata = PublishedTemplateMetadata {
                    template_name: template.name.to_string(),
                    author_public_key: RistrettoPublicKeyBytes::default(),
                    binary_hash,
                    at_epoch: 0,
                    metadata_hash: None,
                };
                tx.upsert_template_catalogue(&template.address, &metadata)?;
            }
            Ok(())
        })
        .await
}

async fn hack_xtr_initial_supply<TStore: IndexerStore>(store: &TStore) -> anyhow::Result<()> {
    store
        .with_write_tx(|tx| {
            // Check if the initial supply is already set
            let existing_supply: Option<Amount> = tx.key_value_get_value(Key::TariAccumulatedClaimed).optional()?;
            if existing_supply.is_some() {
                return Ok(());
            }

            tx.key_value_set(Key::TariAccumulatedClaimed, TXTR_FAUCET_INITIAL_SUPPLY)
                .map_err(|e| anyhow!("Failed to set XTR initial supply in the store: {}", e))
        })
        .await
}
