//   Copyright 2022. The Tari Project
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

use std::{collections::HashMap, fs, io, str::FromStr, sync::Arc};

use anyhow::{Context, anyhow};
use futures::{FutureExt, future};
use libp2p::identity;
use log::*;
use ootle_byte_type::ToByteType;
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common::{
    configuration::bootstrap::{ApplicationType, grpc_default_port},
    exit_codes::{ExitCode, ExitError},
};
use tari_consensus::consensus_constants::ConsensusConstants;
#[cfg(not(feature = "metrics"))]
use tari_consensus::traits::hooks::NoopHooks;
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::Epoch;
use tari_epoch_manager::{
    EpochManagerReader,
    service::{EpochManagerConfig, EpochManagerHandle},
};
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
use tari_networking::{
    MessagingMode,
    NetworkingHandle,
    RelayCircuitLimits,
    RelayReservationLimits,
    SwarmConfig,
    gossip_queue,
    message_queue,
};
use tari_ootle_app_utilities::{
    claim_burn_proof_verifier::TariClaimBurnProofVerifier,
    configuration::convert_network_to_l1_network,
    epoch_oracle_config::{BaseLayerOracleConfig, EpochOracleType},
    fee_tables::get_fee_table_by_network,
    identity_management,
    keypair::RistrettoKeypair,
    seed_peer::SeedPeer,
    transaction_executor::TariTransactionProcessor,
};
use tari_ootle_common_types::services::template_provider::TemplateProvider;
use tari_ootle_p2p::{PeerAddress, TRANSACTION_TOPIC, TariMessagingSpec, max_gossip_message_size};
use tari_ootle_storage::{StateStore, global::GlobalDb};
use tari_ootle_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_ootle_template_provider::MemoryCacheTemplateProvider;
use tari_ootle_transaction::{Network, Transaction};
use tari_ootle_transaction_validation::{
    BasicValidations,
    BlobReferenceValidator,
    EpochRangeValidator,
    InputSubstateValidator,
    PublishTemplateLimitValidator,
    SignatureLimitValidator,
    StealthTransactionLimitsValidator,
    TemplateExistsValidator,
    TransactionDryRunValidator,
    TransactionNetworkValidator,
    TransactionSignatureValidator,
    TransactionSizeValidator,
    TransactionValidationError,
    TransactionValidityWindowValidator,
    TransactionWeightValidator,
    Validator,
    WithContext,
};
use tari_rpc_framework::RpcServer;
use tari_shutdown::ShutdownSignal;
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

#[cfg(feature = "metrics")]
use crate::consensus::metrics::PrometheusConsensusMetrics;
#[cfg(feature = "metrics")]
use crate::epoch_metrics::{EpochManagerCollector, MeteredEpochOracle, PrometheusEpochOracleMetrics};
#[cfg(feature = "metrics")]
use crate::inbound_queue_metrics::InboundQueueCollector;
#[cfg(feature = "metrics")]
use crate::state_store_metrics::StateStoreMemoryCollector;
use crate::{
    ApplicationConfig,
    ValidatorNodeEpochManagerSpec,
    ValidatorNodeStateStore,
    base_layer::verify_correct_network,
    consensus::{
        self,
        ConsensusHandle,
        TariBlockTransactionExecutor,
        TariBlockTransactionValidator,
        spec::ValidatorTemplateProvider,
    },
    file_l1_submitter::FileLayerOneSubmitter,
    memory_budget,
    migrations,
    p2p::{
        NopLogger,
        create_tari_validator_node_rpc_service,
        services::{
            consensus_gossip::{self},
            mempool::{self, MempoolHandle},
            messaging::{ConsensusInboundMessaging, ConsensusOutboundMessaging},
        },
    },
};

const LOG_TARGET: &str = "tari::validator_node::bootstrap";

#[allow(clippy::too_many_lines)]
pub async fn spawn_services(
    config: ApplicationConfig,
    shutdown: ShutdownSignal,
    keypair: RistrettoKeypair,
    global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    consensus_constants: ConsensusConstants,
    #[cfg(feature = "metrics")] metrics_registry: &mut prometheus_client::registry::Registry,
) -> Result<Services<ValidatorNodeStateStore>, anyhow::Error> {
    let mut handles = Vec::with_capacity(10);

    ensure_directories_exist(&config)?;

    // Bounded ingress queues. Each is drained serially by its service, so a queue absorbs arrival
    // bursts that outpace processing; beyond its budget messages are dropped rather than queued
    // without limit. Budgets are per-queue — see `ValidatorNodeConfig`. The message-count bound is
    // a secondary guard against a flood of tiny messages, whose per-message overhead the byte
    // budget does not capture.
    const MAX_QUEUED_INBOUND_MESSAGES: usize = 100_000;

    let (tx_consensus_messages, rx_consensus_messages) = message_queue(
        MAX_QUEUED_INBOUND_MESSAGES,
        config.validator_node.max_consensus_messaging_queue_bytes,
    );
    let (tx_transaction_gossip_messages, rx_transaction_gossip_messages) = gossip_queue(
        MAX_QUEUED_INBOUND_MESSAGES,
        config.validator_node.max_transaction_gossip_queue_bytes,
    );
    let (tx_consensus_gossip_messages, rx_consensus_gossip_messages) = gossip_queue(
        MAX_QUEUED_INBOUND_MESSAGES,
        config.validator_node.max_consensus_gossip_queue_bytes,
    );
    #[cfg(feature = "metrics")]
    InboundQueueCollector::new(
        tx_transaction_gossip_messages.clone(),
        tx_consensus_gossip_messages.clone(),
        tx_consensus_messages.clone(),
    )
    .register(metrics_registry);

    let mut tx_gossip_messages_by_topic = HashMap::new();
    tx_gossip_messages_by_topic.insert(TRANSACTION_TOPIC.to_string(), tx_transaction_gossip_messages);
    tx_gossip_messages_by_topic.insert(consensus_gossip::TOPIC_PREFIX.to_string(), tx_consensus_gossip_messages);

    let identity = identity::Keypair::sr25519_from_bytes(keypair.secret_key().as_bytes().to_vec()).map_err(|e| {
        ExitError::new(
            ExitCode::ConfigError,
            format!("Failed to create libp2p identity from secret bytes: {}", e),
        )
    })?;
    let seed_peers = config
        .peer_seeds
        .peer_seeds
        .iter()
        .map(|s| SeedPeer::from_str(s))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let seed_peers = seed_peers
        .into_iter()
        .map(|p| {
            let peer_id = p.to_peer_id();
            (peer_id, p.into_address())
        })
        .collect();
    #[allow(unused_mut)]
    let mut network_builder = tari_networking::Builder::<TariMessagingSpec>::new(identity)
        .with_messaging_mode(MessagingMode::Enabled {
            tx_messages: tx_consensus_messages,
            tx_gossip_messages_by_topic,
        })
        .with_config(tari_networking::Config {
            // TODO: configurable
            listeners: vec![
                format!("/ip4/0.0.0.0/tcp/{}", config.validator_node.p2p.listener_port)
                    .parse()
                    .expect("Failed to parse listener address"),
                format!("/ip4/0.0.0.0/udp/{}/quic-v1", config.validator_node.p2p.listener_port)
                    .parse()
                    .expect("Failed to parse listener address"),
            ],
            swarm: SwarmConfig {
                protocol_version: format!("/tari/{}/0.0.1", config.network).parse()?,
                user_agent: format!("/tari/validator/{}", env!("CARGO_PKG_VERSION")),
                enable_mdns: config.validator_node.p2p.enable_mdns,
                enable_relay: config.validator_node.p2p.enable_relay,
                // Both topics report a `Reject` verdict for messages that fail to decode or fail
                // validation, so both can score the peers that send them.
                gossip_sub_scored_topics: vec![
                    TRANSACTION_TOPIC.to_string(),
                    consensus_gossip::TOPIC_PREFIX.to_string(),
                ],
                gossip_sub_max_message_size: max_gossip_message_size(consensus_constants.max_transaction_size_bytes),
                // TODO: allow node operator to configure
                relay_circuit_limits: RelayCircuitLimits::high(),
                relay_reservation_limits: RelayReservationLimits::high(),
                rendezvous_server_enabled: config.validator_node.p2p.enable_rendezvous,
                ..Default::default()
            },
            reachability_mode: config.validator_node.p2p.reachability_mode.into(),
            announce: true,
            rendezvous_namespace: format!("tari-{}", config.network.to_string().to_lowercase()),
            ..Default::default()
        })
        .with_seed_peers(seed_peers);

    #[cfg(feature = "metrics")]
    {
        network_builder = network_builder.with_metrics(metrics_registry);
    }
    let (mut networking, join_handle) = network_builder.spawn(shutdown.clone())?;
    handles.push(join_handle);

    info!(target: LOG_TARGET, "Message logging initializing");

    info!(target: LOG_TARGET, "State store initializing");

    // TODO: just enable it always for now, later make it configurable and default to true for testnets
    let db_options = config.validator_node.state_store_options();

    memory_budget::check_against_available_memory(&memory_budget::MemoryBudget::from_config(
        &config.validator_node,
        &db_options,
        &config.validator_node.templates,
    ));

    let state_store = ValidatorNodeStateStore::open(&config.validator_node.state_db_path, db_options)?;

    #[cfg(feature = "metrics")]
    StateStoreMemoryCollector::new(state_store.memory_budget().clone()).register(metrics_registry);

    state_store.with_write_tx(|tx| migrations::migrate(tx, config.network, &consensus_constants))?;

    info!(target: LOG_TARGET, "Epoch manager initializing");
    let epoch_manager_config = EpochManagerConfig {
        base_layer_confirmations: consensus_constants.base_layer_confirmations,
        committee_size: consensus_constants
            .committee_size_per_shard_group
            .try_into()
            .context("committee size must be non-zero")?,
        validator_node_sidechain_id: config.validator_node.sidechain_id.to_byte_type(),
        fee_claim_public_key: config.validator_node.fee_claim_public_key.to_byte_type(),
        num_preshards: consensus_constants.num_preshards,
    };

    // All Tari-side metrics live under the `tari` sub-registry. Sub-registries from this
    // point on (consensus, mempool, epoch_oracle, epoch_manager) are derived from it.
    #[cfg(feature = "metrics")]
    let tari_metrics_registry = metrics_registry.sub_registry_with_prefix("tari");

    // Epoch event scanner. When metrics are enabled, wrap the oracle so every event it
    // produces is counted on its way into the epoch manager.
    let epoch_event_oracle = create_epoch_oracle(&config, global_db.clone(), &consensus_constants).await?;
    #[cfg(feature = "metrics")]
    let epoch_event_oracle = MeteredEpochOracle::new(
        epoch_event_oracle,
        PrometheusEpochOracleMetrics::register(tari_metrics_registry),
    );

    let layer_one_transaction_submitter = FileLayerOneSubmitter::new(config.get_layer_one_transaction_base_path());

    // Epoch manager
    let (epoch_manager, epoch_manager_join_handle) =
        tari_epoch_manager::service::spawn_service::<ValidatorNodeEpochManagerSpec>(
            epoch_manager_config,
            global_db.clone(),
            keypair.public_key().to_byte_type(),
            epoch_event_oracle,
            layer_one_transaction_submitter.clone(),
            shutdown.clone(),
        );

    handles.push(epoch_manager_join_handle);

    // Register the epoch manager's current-epoch collector now that we have a handle. The
    // collector reads the handle's atomic on every scrape, so it always reflects whatever
    // the epoch manager currently believes the epoch to be.
    #[cfg(feature = "metrics")]
    EpochManagerCollector::new(epoch_manager.clone()).register(tari_metrics_registry);

    let validator_node_client_factory = TariValidatorNodeRpcClientFactory::new(networking.clone());

    info!(target: LOG_TARGET, "Template manager initializing");
    // Template manager
    let wasm_cache_dir = config.validator_node.data_dir.join("wasm_cache");
    let template_provider = MemoryCacheTemplateProvider::new(
        tari_engine::wasm::DiskCachedWasmTemplateProvider::open(state_store.clone(), wasm_cache_dir)?,
        &config.validator_node.templates,
    );

    info!(target: LOG_TARGET, "Payload processor initializing");
    // Payload processor

    let (tx_hotstuff_events, _) = broadcast::channel(100);
    // Consensus gossip
    let (consensus_gossip_service, join_handle, rx_consensus_gossip_messages) = consensus_gossip::spawn(
        epoch_manager.subscribe(),
        tx_hotstuff_events.subscribe(),
        networking.clone(),
        rx_consensus_gossip_messages,
    );
    handles.push(join_handle);

    // Messaging
    let message_logger = NopLogger; // SqliteMessageLogger::new(config.validator_node.data_dir.join("message_log.sqlite"));
    let local_address = PeerAddress::from(keypair.public_key().clone());
    let (loopback_sender, loopback_receiver) = mpsc::unbounded_channel();
    let inbound_messaging = ConsensusInboundMessaging::new(
        local_address,
        rx_consensus_messages,
        rx_consensus_gossip_messages,
        loopback_receiver,
        message_logger.clone(),
    );
    let outbound_messaging = ConsensusOutboundMessaging::new(
        loopback_sender,
        consensus_gossip_service.clone(),
        networking.clone(),
        message_logger.clone(),
    );

    // Transaction executor
    let fee_table = get_fee_table_by_network(config.network);
    let transaction_processor = TariTransactionProcessor::new(
        config.network,
        template_provider.clone(),
        fee_table.clone(),
        false,
        Arc::new(TariClaimBurnProofVerifier::new(
            config.network,
            config.validator_node.sidechain_id.as_ref().map(|pk| pk.to_byte_type()),
            global_db.clone(),
        )),
    );
    // The executor resolves the exhaust burn rate for each transaction's execution epoch.
    let transaction_executor = TariBlockTransactionExecutor::new(transaction_processor, consensus_constants.clone());

    let transaction_validator = TariBlockTransactionValidator::new(
        create_node_transaction_validator(config.network, template_provider.clone(), &consensus_constants).boxed(),
        EpochRangeValidator::new().boxed(),
    );

    #[cfg(feature = "metrics")]
    let metrics = PrometheusConsensusMetrics::register(tari_metrics_registry);
    #[cfg(not(feature = "metrics"))]
    let metrics = NoopHooks;

    let sidechain_id = config.validator_node.sidechain_id.as_ref().map(|pk| pk.to_byte_type());

    // Consensus
    let signing_service = consensus::TariSignatureService::new(keypair.clone());
    let (consensus_join_handle, consensus_handle) = consensus::spawn(
        config.network,
        &config.validator_node.consensus,
        sidechain_id,
        state_store.clone(),
        local_address,
        signing_service,
        epoch_manager.clone(),
        inbound_messaging,
        outbound_messaging.clone(),
        validator_node_client_factory.clone(),
        metrics,
        shutdown.clone(),
        transaction_executor,
        transaction_validator,
        tx_hotstuff_events,
        consensus_constants.clone(),
    )
    .await;
    handles.push(consensus_join_handle);

    let (mempool, join_handle) = mempool::spawn(
        epoch_manager.clone(),
        create_mempool_transaction_validator(config.network, template_provider.clone(), &consensus_constants),
        state_store.clone(),
        consensus_handle.clone(),
        networking.clone(),
        rx_transaction_gossip_messages,
        #[cfg(feature = "metrics")]
        tari_metrics_registry,
    );
    handles.push(join_handle);

    let join_handle = spawn_p2p_rpc(
        &config,
        &mut networking,
        epoch_manager.clone(),
        state_store.clone(),
        mempool.clone(),
        consensus_handle.clone(),
    )
    .await?;
    handles.push(join_handle);
    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    save_identities(&config, &keypair)?;

    Ok(Services {
        config,
        keypair,
        networking,
        mempool,
        epoch_manager,
        global_db,
        template_provider,
        consensus_handle,
        consensus_constants,
        state_store,
        handles,
        layer_one_transaction_submitter,
    })
}

fn save_identities(config: &ApplicationConfig, keypair: &RistrettoKeypair) -> Result<(), ExitError> {
    identity_management::save_as_json(&config.validator_node.identity_file, keypair)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save node identity: {}", e)))?;

    Ok(())
}

fn ensure_directories_exist(config: &ApplicationConfig) -> io::Result<()> {
    fs::create_dir_all(&config.validator_node.data_dir)?;
    Ok(())
}
pub struct Services<TStore> {
    pub config: ApplicationConfig,
    pub keypair: RistrettoKeypair,
    pub networking: NetworkingHandle<TariMessagingSpec>,
    pub mempool: MempoolHandle,
    pub epoch_manager: EpochManagerHandle<PeerAddress>,
    pub template_provider: ValidatorTemplateProvider,
    pub consensus_handle: ConsensusHandle,
    pub consensus_constants: ConsensusConstants,
    pub state_store: TStore,
    pub global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    pub layer_one_transaction_submitter: FileLayerOneSubmitter,

    pub handles: Vec<JoinHandle<Result<(), anyhow::Error>>>,
}

impl<TStore> Services<TStore> {
    pub async fn on_any_exit(&mut self) -> Result<(), anyhow::Error> {
        // JoinHandler panics if polled again after reading the Result, we fuse the future to prevent this.
        let fused = self.handles.iter_mut().map(|h| h.fuse());
        let (res, _, _) = future::select_all(fused).await;
        res.unwrap_or_else(|e| Err(anyhow!("Task panicked: {}", e)))
    }

    pub async fn join_all(self) -> Result<(), anyhow::Error> {
        // Handles that have already been polled to completion by `on_any_exit` would
        // panic tokio's JoinHandle invariant ("polled after completion") if we awaited
        // them again. Filter them out — their result is either already surfaced via
        // `on_any_exit` or we're in the shutdown path and don't care about replay.
        let handles: Vec<_> = self.handles.into_iter().filter(|h| !h.is_finished()).collect();
        let results = future::try_join_all(handles).await?;
        for res in results {
            res?;
        }
        Ok(())
    }
}

async fn spawn_p2p_rpc<TStateStore: StateStore + Clone + Send + Sync + 'static>(
    config: &ApplicationConfig,
    networking: &mut NetworkingHandle<TariMessagingSpec>,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    shard_store_store: TStateStore,
    mempool: MempoolHandle,
    consensus: ConsensusHandle,
) -> anyhow::Result<JoinHandle<Result<(), anyhow::Error>>> {
    let rpc_server = RpcServer::builder()
        .with_maximum_simultaneous_sessions(config.validator_node.rpc.max_simultaneous_sessions)
        .with_maximum_sessions_per_client(config.validator_node.rpc.max_sessions_per_client)
        .finish()
        .add_service(create_tari_validator_node_rpc_service(
            epoch_manager,
            shard_store_store,
            mempool,
            consensus,
        ));

    let (notify_tx, notify_rx) = mpsc::unbounded_channel();
    networking
        .add_protocol_notifier(rpc_server.all_protocols().iter().cloned(), notify_tx)
        .await?;
    // The RPC service owns a state store clone, so this handle must be tracked in `Services::handles`:
    // `join_all` awaiting it is what guarantees the store is dropped (and RocksDB's LOCK released) before
    // `run_validator_node` returns. The task ends at shutdown when the networking worker exits and drops
    // the protocol notifier sender, closing `notify_rx`.
    let handle = tokio::spawn(async move { rpc_server.serve(notify_rx).await.map_err(anyhow::Error::from) });
    Ok(handle)
}

/// Builds every validation a validator node applies without an epoch: the structural checks plus the
/// node-local ones.
///
/// `TemplateExistsValidator` depends on lagging local state and can false-reject, so this chain is node-local
/// rather than structural — `TemplateNotFound` is correspondingly not sender fault.
pub fn create_node_transaction_validator<TProvider: TemplateProvider>(
    network: Network,
    template_manager: TProvider,
    constants: &ConsensusConstants,
) -> impl Validator<Transaction, Context = (), Error = TransactionValidationError> + use<TProvider> {
    TransactionNetworkValidator::new(network)
        .and_then(TransactionDryRunValidator)
        .and_then(BasicValidations::new())
        // Bytes before weight: the byte cap is what the gossip message limit is derived from, so a
        // transaction failing it could not have been relayed regardless of what it weighs.
        .and_then(TransactionSizeValidator::new(constants.max_transaction_size_bytes))
        // Blob payloads must be exactly what the instructions reference: bad indices would only
        // fail at execution, and unreferenced blobs would never fail at all.
        .and_then(BlobReferenceValidator::new())
        // Cheap structural check — reject over-weight transactions before verifying signatures.
        .and_then(TransactionWeightValidator::new(constants.max_transaction_weight))
        // Reject transactions whose aggregate stealth-transfer work exceeds the per-transaction caps before
        // verifying signatures or executing.
        .and_then(StealthTransactionLimitsValidator::new())
        .and_then(PublishTemplateLimitValidator::new())
        // Bounds the number of signature verifications the next validator performs.
        .and_then(SignatureLimitValidator::new())
        .and_then(TransactionSignatureValidator)
        .and_then(TemplateExistsValidator::new(template_manager))
}

/// Builds the validations applied when a transaction is admitted to this node's mempool: the node-local chain
/// plus the two epoch-window rules and the ingress-only input check.
///
/// [`InputSubstateValidator`] is deliberately here rather than in [`create_node_transaction_validator`], which
/// also backs `TariBlockTransactionValidator`. A rejection rule in block validation is a consensus rule: an
/// upgraded validator would refuse a block that a non-upgraded one accepts. Refusing at ingress costs the
/// sender and nothing else, and a transaction that slips past an un-upgraded node's mempool still aborts at
/// input resolution as it does today.
pub fn create_mempool_transaction_validator<TProvider: TemplateProvider>(
    network: Network,
    template_manager: TProvider,
    constants: &ConsensusConstants,
) -> impl Validator<Transaction, Context = Epoch, Error = TransactionValidationError> + use<TProvider> {
    WithContext::<Epoch, Transaction, TransactionValidationError>::new()
        .map_context(
            |_| (),
            create_node_transaction_validator(network, template_manager, constants)
                .and_then(InputSubstateValidator::new()),
        )
        .and_then(EpochRangeValidator::new())
        .and_then(TransactionValidityWindowValidator::new(
            constants.max_transaction_validity_epochs,
        ))
}

async fn create_base_layer_client(
    network: Network,
    config: &BaseLayerOracleConfig,
) -> Result<GrpcBaseNodeClient, ExitError> {
    let base_node_address = config.base_node_grpc_url.clone().unwrap_or_else(|| {
        let port = grpc_default_port(ApplicationType::BaseNode, convert_network_to_l1_network(&network));
        format!("http://127.0.0.1:{port}")
            .parse()
            .expect("Default base node GRPC URL is malformed")
    });
    info!(target: LOG_TARGET, "Connecting to base node on GRPC at {}", base_node_address);
    let base_node_client = GrpcBaseNodeClient::connect(base_node_address.clone())
        .await
        .map_err(|error| {
            ExitError::new(
                ExitCode::ConfigError,
                format!(
                    "Could not connect to the Minotari node at address {base_node_address}: {error}. Please ensure \
                     that the Minotari node is running and configured for GRPC."
                ),
            )
        })?;

    Ok(base_node_client)
}

async fn create_epoch_oracle<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Send + Clone + 'static>(
    config: &ApplicationConfig,
    store: TStore,
    consensus_constants: &ConsensusConstants,
) -> anyhow::Result<EpochOracle<TStore>> {
    match config.epoch_oracle.oracle_type {
        EpochOracleType::BaseLayer => {
            let features = BaseLayerEpochOracleFeatures {
                sync_headers: true,
                sync_validator_node_changes: true,
            };
            let oracle = create_base_layer_epoch_oracle(config, store, consensus_constants, features).await?;
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

async fn create_base_layer_epoch_oracle<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Clone + 'static>(
    config: &ApplicationConfig,
    store: TStore,
    consensus_constants: &ConsensusConstants,
    features: BaseLayerEpochOracleFeatures,
) -> anyhow::Result<BaseLayerOracle<TStore>> {
    info!(target: LOG_TARGET, "🔮Base layer epoch oracle: {}", config.epoch_oracle.base_layer);
    let mut base_node_client = create_base_layer_client(config.network, &config.epoch_oracle.base_layer).await?;
    verify_correct_network(&mut base_node_client, config.network).await?;
    Ok(BaseLayerOracle::new(
        store,
        base_node_client,
        BaseLayerEpochOracleConfig {
            start_height: config.epoch_oracle.base_layer.start_height,
            height_lag: consensus_constants.base_layer_confirmations,
            scanning_interval: config.epoch_oracle.base_layer.scanning_interval,
            sidechain_id: config.validator_node.sidechain_id.as_ref().map(|p| p.to_byte_type()),
            features,
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
    info!(target: LOG_TARGET, "🔮Configured epoch oracle: {}", oracle_config);
    let oracle = ConfiguredEpochOracle::create(oracle_config, store)?;
    Ok(oracle)
}

async fn create_hybrid_epoch_oracle<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Clone + Send + 'static>(
    config: &ApplicationConfig,
    store: TStore,
    consensus_constants: &ConsensusConstants,
) -> anyhow::Result<HybridEpochOracle<TStore>> {
    let features = BaseLayerEpochOracleFeatures {
        sync_headers: true,
        // Dont sync validator node changes as they are handled by the configured oracle
        sync_validator_node_changes: false,
    };
    let base_layer_oracle =
        create_base_layer_epoch_oracle(config, store.clone(), consensus_constants, features).await?;
    let oracle_config = config.epoch_oracle.configured.load().await?;

    info!(target: LOG_TARGET, "🔮Hybrid epoch oracle initializing: {}", oracle_config);
    let (ticker, trigger) = mpsc_ticker();
    let configured_oracle = ConfiguredEpochOracle::with_custom_ticker(oracle_config, store, ticker);
    Ok(HybridEpochOracle::new(configured_oracle, base_layer_oracle, trigger))
}
