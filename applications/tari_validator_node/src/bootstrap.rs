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

use std::{fs, io, ops::DerefMut, str::FromStr};

use anyhow::anyhow;
use futures::{future, FutureExt};
use libp2p::identity;
use log::info;
use minotari_app_utilities::identity_management;
use serde::Serialize;
use sqlite_message_logger::SqliteMessageLogger;
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common::{
    configuration::Network,
    exit_codes::{ExitCode, ExitError},
};
#[cfg(not(feature = "metrics"))]
use tari_consensus::traits::hooks::NoopHooks;
use tari_core::transactions::transaction_components::ValidatorNodeSignature;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_app_utilities::{
    base_layer_scanner,
    consensus_constants::ConsensusConstants,
    keypair::RistrettoKeypair,
    seed_peer::SeedPeer,
    substate_file_cache::SubstateFileCache,
    template_manager,
    template_manager::{implementation::TemplateManager, interface::TemplateManagerHandle},
    transaction_executor::TariDanTransactionProcessor,
};
use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight, PeerAddress, SubstateAddress};
use tari_dan_engine::fees::FeeTable;
use tari_dan_p2p::TariMessagingSpec;
use tari_dan_storage::{
    consensus_models::{Block, BlockId, ExecutedTransaction, SubstateRecord},
    global::GlobalDb,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_engine_types::{resource::Resource, substate::SubstateId};
use tari_epoch_manager::base_layer::{EpochManagerConfig, EpochManagerHandle};
use tari_indexer_lib::substate_scanner::SubstateScanner;
use tari_networking::{MessagingMode, NetworkingHandle, RelayCircuitLimits, RelayReservationLimits, SwarmConfig};
use tari_rpc_framework::RpcServer;
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_template_lib::{
    auth::ResourceAccessRules,
    constants::{CONFIDENTIAL_TARI_RESOURCE_ADDRESS, PUBLIC_IDENTITY_RESOURCE_ADDRESS},
    models::Metadata,
    prelude::{OwnerRule, ResourceType},
    resource::TOKEN_SYMBOL,
};
use tari_transaction::Transaction;
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;
use tokio::{sync::mpsc, task::JoinHandle};

#[cfg(feature = "metrics")]
use crate::consensus::metrics::PrometheusConsensusMetrics;
use crate::{
    consensus::{self, ConsensusHandle, TariDanBlockTransactionExecutorBuilder},
    dry_run_transaction_processor::DryRunTransactionProcessor,
    p2p::{
        create_tari_validator_node_rpc_service,
        services::{
            mempool::{
                self,
                ClaimFeeTransactionValidator,
                EpochRangeValidator,
                FeeTransactionValidator,
                HasInputs,
                HasInvolvedShards,
                MempoolError,
                MempoolHandle,
                OutputsDontExistLocally,
                TemplateExistsValidator,
                TransactionSignatureValidator,
                Validator,
            },
            messaging::{ConsensusInboundMessaging, ConsensusOutboundMessaging, Gossip},
        },
    },
    substate_resolver::TariSubstateResolver,
    validator_registration_file::ValidatorRegistrationFile,
    virtual_substate::VirtualSubstateManager,
    ApplicationConfig,
    ValidatorNodeConfig,
};

const LOG_TARGET: &str = "tari::validator_node::bootstrap";

#[allow(clippy::too_many_lines)]
pub async fn spawn_services(
    config: &ApplicationConfig,
    shutdown: ShutdownSignal,
    keypair: RistrettoKeypair,
    global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    consensus_constants: ConsensusConstants,
    base_node_client: GrpcBaseNodeClient,
    #[cfg(feature = "metrics")] metrics_registry: &prometheus::Registry,
) -> Result<Services, anyhow::Error> {
    let mut handles = Vec::with_capacity(8);

    ensure_directories_exist(config)?;

    // Networking
    let (tx_consensus_messages, rx_consensus_messages) = mpsc::unbounded_channel();
    let (tx_gossip_messages, rx_gossip_messages) = mpsc::unbounded_channel();
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
        .flat_map(|p| {
            let peer_id = p.to_peer_id();
            p.addresses.into_iter().map(move |a| (peer_id, a))
        })
        .collect();
    let (mut networking, join_handle) = tari_networking::spawn(
        identity,
        MessagingMode::Enabled {
            tx_messages: tx_consensus_messages,
            tx_gossip_messages,
        },
        tari_networking::Config {
            listener_port: config.validator_node.p2p.listener_port,
            swarm: SwarmConfig {
                protocol_version: format!("/tari/{}/0.0.1", config.network).parse().unwrap(),
                user_agent: "/tari/validator/0.0.1".to_string(),
                enable_mdns: config.validator_node.p2p.enable_mdns,
                enable_relay: true,
                // TODO: allow node operator to configure
                relay_circuit_limits: RelayCircuitLimits::high(),
                relay_reservation_limits: RelayReservationLimits::high(),
                ..Default::default()
            },
            reachability_mode: config.validator_node.p2p.reachability_mode.into(),
            announce: true,
            ..Default::default()
        },
        seed_peers,
        shutdown.clone(),
    )?;
    handles.push(join_handle);

    info!(target: LOG_TARGET, "Message logging initializing");
    // Spawn messaging
    let message_logger = SqliteMessageLogger::new(config.validator_node.data_dir.join("message_log.sqlite"));

    info!(target: LOG_TARGET, "State store initializing");
    // Connect to shard db
    let state_store =
        SqliteStateStore::connect(&format!("sqlite://{}", config.validator_node.state_db_path().display()))?;
    state_store.with_write_tx(|tx| bootstrap_state(tx, config.network))?;

    info!(target: LOG_TARGET, "Epoch manager initializing");
    // Epoch manager
    let (epoch_manager, join_handle) = tari_epoch_manager::base_layer::spawn_service(
        // TODO: We should be able to pass consensus constants here. However, these are currently located in dan_core
        // which depends on epoch_manager, so would be a circular dependency.
        EpochManagerConfig {
            base_layer_confirmations: consensus_constants.base_layer_confirmations,
            committee_size: consensus_constants.committee_size,
            validator_node_sidechain_id: config.validator_node.validator_node_sidechain_id.clone(),
        },
        global_db.clone(),
        base_node_client.clone(),
        keypair.public_key().clone(),
        shutdown.clone(),
    );
    handles.push(join_handle);

    // Create registration file
    create_registration_file(config, &epoch_manager, &keypair).await?;

    info!(target: LOG_TARGET, "Template manager initializing");
    // Template manager
    let template_manager = TemplateManager::initialize(global_db.clone(), config.validator_node.templates.clone())?;
    let (template_manager_service, join_handle) =
        template_manager::implementation::spawn(template_manager.clone(), shutdown.clone());
    handles.push(join_handle);

    info!(target: LOG_TARGET, "Payload processor initializing");
    // Payload processor
    let fee_table = if config.validator_node.no_fees {
        FeeTable::zero_rated()
    } else {
        FeeTable {
            per_module_call_cost: 1,
            per_byte_storage_cost: 1,
            per_event_cost: 1,
            per_log_cost: 1,
        }
    };
    let payload_processor = TariDanTransactionProcessor::new(config.network, template_manager.clone(), fee_table);

    let validator_node_client_factory = TariValidatorNodeRpcClientFactory::new(networking.clone());

    // Consensus
    let (tx_executed_transaction, rx_executed_transaction) = mpsc::channel(10);

    let local_address = PeerAddress::from(keypair.public_key().clone());
    let (loopback_sender, loopback_receiver) = mpsc::unbounded_channel();
    let inbound_messaging = ConsensusInboundMessaging::new(
        local_address,
        rx_consensus_messages,
        loopback_receiver,
        message_logger.clone(),
    );
    let outbound_messaging =
        ConsensusOutboundMessaging::new(loopback_sender, networking.clone(), message_logger.clone());

    let transaction_executor_builder =
        TariDanBlockTransactionExecutorBuilder::new(epoch_manager.clone(), payload_processor.clone());

    #[cfg(feature = "metrics")]
    let metrics = PrometheusConsensusMetrics::new(state_store.clone(), metrics_registry);
    #[cfg(not(feature = "metrics"))]
    let metrics = NoopHooks;

    let (consensus_join_handle, consensus_handle, rx_consensus_to_mempool) = consensus::spawn(
        config.network,
        state_store.clone(),
        keypair.clone(),
        epoch_manager.clone(),
        rx_executed_transaction,
        inbound_messaging,
        outbound_messaging.clone(),
        validator_node_client_factory.clone(),
        metrics,
        shutdown.clone(),
        transaction_executor_builder,
        consensus_constants.clone(),
        config.validator_node.is_listener_mode
    )
    .await;
    handles.push(consensus_join_handle);

    // substate cache
    let substate_cache_dir = config.common.base_path.join("substate_cache");
    let substate_cache = SubstateFileCache::new(substate_cache_dir)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Substate cache error: {}", e)))?;

    // Mempool
    let virtual_substate_manager = VirtualSubstateManager::new(state_store.clone(), epoch_manager.clone());
    let scanner = SubstateScanner::new(
        epoch_manager.clone(),
        validator_node_client_factory.clone(),
        substate_cache,
    );
    let substate_resolver = TariSubstateResolver::new(
        state_store.clone(),
        scanner,
        epoch_manager.clone(),
        virtual_substate_manager.clone(),
    );

    let gossip = Gossip::new(networking.clone(), rx_gossip_messages);

    let (mempool, join_handle) = mempool::spawn(
        gossip,
        tx_executed_transaction,
        epoch_manager.clone(),
        payload_processor.clone(),
        substate_resolver.clone(),
        create_mempool_before_execute_validator(
            &config.validator_node,
            template_manager.clone(),
            epoch_manager.clone(),
        ),
        create_mempool_after_execute_validator(state_store.clone()),
        state_store.clone(),
        rx_consensus_to_mempool,
        consensus_handle.clone(),
        #[cfg(feature = "metrics")]
        metrics_registry,
    );
    handles.push(join_handle);

    // Base Node scanner
    let join_handle = base_layer_scanner::spawn(
        config.network,
        global_db.clone(),
        base_node_client.clone(),
        epoch_manager.clone(),
        template_manager_service.clone(),
        shutdown.clone(),
        consensus_constants,
        state_store.clone(),
        config.validator_node.scan_base_layer,
        config.validator_node.base_layer_scanning_interval,
        config.validator_node.validator_node_sidechain_id.clone(),
        config.validator_node.template_sidechain_id.clone(),
        config.validator_node.burnt_utxo_sidechain_id.clone(),
    );
    handles.push(join_handle);

    spawn_p2p_rpc(
        config,
        &mut networking,
        state_store.clone(),
        mempool.clone(),
        virtual_substate_manager,
    )
    .await?;
    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    save_identities(config, &keypair)?;

    let dry_run_transaction_processor =
        DryRunTransactionProcessor::new(epoch_manager.clone(), payload_processor, substate_resolver);

    Ok(Services {
        keypair,
        networking,
        mempool,
        epoch_manager,
        template_manager: template_manager_service,
        consensus_handle,
        global_db,
        state_store,
        dry_run_transaction_processor,
        handles,
        validator_node_client_factory,
    })
}

async fn create_registration_file(
    config: &ApplicationConfig,
    epoch_manager: &EpochManagerHandle<PeerAddress>,
    keypair: &RistrettoKeypair,
) -> Result<(), anyhow::Error> {
    let fee_claim_public_key = config.validator_node.fee_claim_public_key.clone();
    epoch_manager
        .set_fee_claim_public_key(fee_claim_public_key.clone())
        .await?;

    let signature = ValidatorNodeSignature::sign(keypair.secret_key(), &fee_claim_public_key, b"");

    let registration = ValidatorRegistrationFile {
        signature,
        public_key: keypair.public_key().clone(),
        claim_fees_public_key: fee_claim_public_key,
    };
    fs::write(
        config.common.base_path.join("registration.json"),
        serde_json::to_string(&registration)?,
    )
    .map_err(|e| ExitError::new(ExitCode::UnknownError, e))?;
    Ok(())
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

pub struct Services {
    pub keypair: RistrettoKeypair,
    pub networking: NetworkingHandle<TariMessagingSpec>,
    pub mempool: MempoolHandle,
    pub epoch_manager: EpochManagerHandle<PeerAddress>,
    pub template_manager: TemplateManagerHandle,
    pub consensus_handle: ConsensusHandle,
    pub global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    pub dry_run_transaction_processor: DryRunTransactionProcessor,
    pub validator_node_client_factory: TariValidatorNodeRpcClientFactory,
    pub state_store: SqliteStateStore<PeerAddress>,

    pub handles: Vec<JoinHandle<Result<(), anyhow::Error>>>,
}

impl Services {
    pub async fn on_any_exit(&mut self) -> Result<(), anyhow::Error> {
        // JoinHandler panics if polled again after reading the Result, we fuse the future to prevent this.
        let fused = self.handles.iter_mut().map(|h| h.fuse());
        let (res, _, _) = future::select_all(fused).await;
        res.unwrap_or_else(|e| Err(anyhow!("Task panicked: {}", e)))
    }
}

async fn spawn_p2p_rpc(
    config: &ApplicationConfig,
    networking: &mut NetworkingHandle<TariMessagingSpec>,
    shard_store_store: SqliteStateStore<PeerAddress>,
    mempool: MempoolHandle,
    virtual_substate_manager: VirtualSubstateManager<SqliteStateStore<PeerAddress>, EpochManagerHandle<PeerAddress>>,
) -> anyhow::Result<()> {
    let rpc_server = RpcServer::builder()
        .with_maximum_simultaneous_sessions(config.validator_node.rpc.max_simultaneous_sessions)
        .with_maximum_sessions_per_client(config.validator_node.rpc.max_sessions_per_client)
        .finish()
        .add_service(create_tari_validator_node_rpc_service(
            shard_store_store,
            mempool,
            virtual_substate_manager,
        ));

    let (notify_tx, notify_rx) = mpsc::unbounded_channel();
    networking
        .add_protocol_notifier(rpc_server.all_protocols().iter().cloned(), notify_tx)
        .await?;
    tokio::spawn(rpc_server.serve(notify_rx));
    Ok(())
}

// TODO: Figure out the best way to have the engine shard store mirror these bootstrapped states.
fn bootstrap_state<TTx>(tx: &mut TTx, network: Network) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + DerefMut,
    TTx::Target: StateStoreReadTransaction,
    TTx::Addr: NodeAddressable + Serialize,
{
    let genesis_block = Block::genesis(network);
    let substate_id = SubstateId::Resource(PUBLIC_IDENTITY_RESOURCE_ADDRESS);
    let substate_address = SubstateAddress::from_address(&substate_id, 0);
    let mut metadata: Metadata = Default::default();
    metadata.insert(TOKEN_SYMBOL, "ID".to_string());
    if !SubstateRecord::exists(tx.deref_mut(), &substate_address)? {
        // Create the resource for public identity
        SubstateRecord {
            substate_id,
            version: 0,
            substate_value: Resource::new(
                ResourceType::NonFungible,
                None,
                OwnerRule::None,
                ResourceAccessRules::new(),
                metadata,
                None,
                None,
            )
            .into(),
            state_hash: Default::default(),
            created_by_transaction: Default::default(),
            created_justify: *genesis_block.justify().id(),
            created_block: BlockId::genesis(),
            created_height: NodeHeight(0),
            created_at_epoch: Epoch(0),
            destroyed: None,
        }
        .create(tx)?;
    }

    let substate_id = SubstateId::Resource(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
    let substate_address = SubstateAddress::from_address(&substate_id, 0);
    let mut metadata = Metadata::new();
    metadata.insert(TOKEN_SYMBOL, "tXTR2".to_string());
    if !SubstateRecord::exists(tx.deref_mut(), &substate_address)? {
        SubstateRecord {
            substate_id,
            version: 0,
            substate_value: Resource::new(
                ResourceType::Confidential,
                None,
                OwnerRule::None,
                ResourceAccessRules::new(),
                metadata,
                None,
                None,
            )
            .into(),
            state_hash: Default::default(),
            created_by_transaction: Default::default(),
            created_justify: *genesis_block.justify().id(),
            created_block: BlockId::genesis(),
            created_height: NodeHeight(0),
            created_at_epoch: Epoch(0),
            destroyed: None,
        }
        .create(tx)?;
    }

    Ok(())
}

fn create_mempool_before_execute_validator(
    config: &ValidatorNodeConfig,
    template_manager: TemplateManager<PeerAddress>,
    epoch_manager: EpochManagerHandle<PeerAddress>,
) -> impl Validator<Transaction, Error = MempoolError> {
    let mut validator = TransactionSignatureValidator
        .and_then(TemplateExistsValidator::new(template_manager))
        .and_then(EpochRangeValidator::new(epoch_manager.clone()))
        .and_then(ClaimFeeTransactionValidator::new(epoch_manager))
        .boxed();
    if !config.no_fees {
        // A transaction without fee payment may have 0 inputs.
        validator = HasInputs::new()
            .and_then(validator)
            .and_then(FeeTransactionValidator)
            .boxed();
    }
    validator
}

fn create_mempool_after_execute_validator<TAddr: NodeAddressable>(
    store: SqliteStateStore<TAddr>,
) -> impl Validator<ExecutedTransaction, Error = MempoolError> {
    HasInvolvedShards::new().and_then(OutputsDontExistLocally::new(store))
}
