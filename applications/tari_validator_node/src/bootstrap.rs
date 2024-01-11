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
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_common_types::types::PublicKey;
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
    transaction_executor::TariDanTransactionProcessor, signature_service::TariSignatureService,
};
use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight, PeerAddress, ShardId};
use tari_dan_engine::fees::FeeTable;
use tari_dan_storage::{
    consensus_models::{Block, BlockId, ExecutedTransaction, ForeignReceiveCounters, SubstateRecord},
    global::GlobalDb,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_engine_types::{resource::Resource, substate::SubstateAddress};
use tari_epoch_manager::base_layer::{EpochManagerConfig, EpochManagerHandle};
use tari_indexer_lib::substate_scanner::SubstateScanner;
use tari_networking::{NetworkingHandle, SwarmConfig};
use tari_rpc_framework::RpcServer;
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_template_lib::{
    auth::ResourceAccessRules,
    constants::{CONFIDENTIAL_TARI_RESOURCE_ADDRESS, PUBLIC_IDENTITY_RESOURCE_ADDRESS},
    crypto::RistrettoPublicKeyBytes,
    models::Metadata,
    prelude::{OwnerRule, ResourceType},
    resource::TOKEN_SYMBOL,
};
use tari_transaction::Transaction;
use tari_validator_node_rpc::{client::TariValidatorNodeRpcClientFactory, proto};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    consensus,
    consensus::ConsensusHandle,
    dry_run_transaction_processor::DryRunTransactionProcessor,
    p2p::{
        create_tari_validator_node_rpc_service,
        services::{
            mempool,
            mempool::{
                ClaimFeeTransactionValidator,
                EpochRangeValidator,
                FeeTransactionValidator,
                HasInputs,
                HasInvolvedShards,
                InputRefsValidator,
                MempoolError,
                MempoolHandle,
                OutputsDontExistLocally,
                TemplateExistsValidator,
                TransactionSignatureValidator,
                Validator,
            },
            message_dispatcher,
        },
    },
    registration,
    substate_resolver::TariSubstateResolver,
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
) -> Result<Services, anyhow::Error> {
    let mut handles = Vec::with_capacity(8);

    ensure_directories_exist(config)?;

    // Connection to base node
    let base_node_client =
        GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address.clone().unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::BaseNode, config.network);
            format!("127.0.0.1:{port}")
        }));

    // Networking
    let (message_senders, message_receivers) = message_dispatcher::new_messaging_channel(30);
    let (tx_messages, rx_messages) = mpsc::channel(100);
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
    let (mut networking, join_handle) = tari_networking::spawn::<proto::network::Message>(
        identity,
        tx_messages,
        tari_networking::Config {
            listener_port: config.validator_node.p2p.listener_port,
            swarm: SwarmConfig {
                protocol_version: format!("/tari/{}/0.0.1", config.network).parse().unwrap(),
                user_agent: "/tari/validator/0.0.1".to_string(),
                enable_mdns: config.validator_node.p2p.enable_mdns,
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

    // Spawn messaging
    let message_logger = SqliteMessageLogger::new(config.validator_node.data_dir.join("message_log.sqlite"));
    let (outbound_messaging, join_handle) =
        message_dispatcher::spawn(networking.clone(), rx_messages, message_senders, message_logger);
    handles.push(join_handle);

    let message_dispatcher::DanMessageReceivers {
        rx_consensus_message,
        rx_new_transaction_message,
    } = message_receivers;

    // Connect to shard db
    let state_store =
        SqliteStateStore::connect(&format!("sqlite://{}", config.validator_node.state_db_path().display()))?;
    state_store.with_write_tx(|tx| bootstrap_state(tx))?;

    // Epoch manager
    let (epoch_manager, join_handle) = tari_epoch_manager::base_layer::spawn_service(
        // TODO: We should be able to pass consensus constants here. However, these are currently located in dan_core
        // which depends on epoch_manager, so would be a circular dependency.
        EpochManagerConfig {
            base_layer_confirmations: consensus_constants.base_layer_confirmations,
            committee_size: consensus_constants.committee_size,
        },
        global_db.clone(),
        base_node_client.clone(),
        keypair.public_key().clone(),
        shutdown.clone(),
    );
    handles.push(join_handle);

    // Create registration file
    create_registration_file(config, &epoch_manager, &keypair).await?;

    // Template manager
    let template_manager = TemplateManager::initialize(global_db.clone(), config.validator_node.templates.clone())?;
    let (template_manager_service, join_handle) =
        template_manager::implementation::spawn(template_manager.clone(), shutdown.clone());
    handles.push(join_handle);

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
    let payload_processor = TariDanTransactionProcessor::new(template_manager.clone(), fee_table);

    let validator_node_client_factory = TariValidatorNodeRpcClientFactory::new(networking.clone());

    // Consensus
    let (tx_executed_transaction, rx_executed_transaction) = mpsc::channel(10);
    let foreign_receive_counter = state_store.with_read_tx(|tx| ForeignReceiveCounters::get(tx))?;
    let (consensus_join_handle, consensus_handle, rx_consensus_to_mempool) = consensus::spawn(
        state_store.clone(),
        keypair.clone(),
        epoch_manager.clone(),
        rx_executed_transaction,
        rx_consensus_message,
        outbound_messaging.clone(),
        validator_node_client_factory.clone(),
        foreign_receive_counter,
        shutdown.clone(),
    )
    .await;
    handles.push(consensus_join_handle);

    // substate cache
    let substate_cache_dir = config.common.base_path.join("substate_cache");
    let substate_cache = SubstateFileCache::new(substate_cache_dir)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Substate cache error: {}", e)))?;

    // Signature service
    let signing_service = TariSignatureService::new(keypair.clone());

    // Mempool
    let virtual_substate_manager = VirtualSubstateManager::new(state_store.clone(), epoch_manager.clone());
    let scanner = SubstateScanner::new(
        epoch_manager.clone(),
        validator_node_client_factory.clone(),
        substate_cache,
        signing_service
    );
    let substate_resolver = TariSubstateResolver::new(
        state_store.clone(),
        scanner,
        epoch_manager.clone(),
        virtual_substate_manager.clone(),
    );
    let (mempool, join_handle) = mempool::spawn(
        rx_new_transaction_message,
        outbound_messaging,
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
    );
    handles.push(join_handle);

    // Base Node scanner
    let join_handle = base_layer_scanner::spawn(
        global_db.clone(),
        base_node_client.clone(),
        epoch_manager.clone(),
        template_manager_service.clone(),
        shutdown.clone(),
        consensus_constants,
        state_store.clone(),
        config.validator_node.scan_base_layer,
        config.validator_node.base_layer_scanning_interval,
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

    // Auto-registration
    if config.validator_node.auto_register {
        let handle = registration::spawn(config.clone(), keypair.clone(), epoch_manager.clone(), shutdown);
        handles.push(handle);
    } else {
        info!(target: LOG_TARGET, "♽️ Node auto registration is disabled");
    }

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
    #[derive(Serialize)]
    struct ValidatorRegistrationFile {
        signature: ValidatorNodeSignature,
        public_key: PublicKey,
        claim_public_key: PublicKey,
    }
    let registration = ValidatorRegistrationFile {
        signature,
        public_key: keypair.public_key().clone(),
        claim_public_key: fee_claim_public_key,
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
    pub networking: NetworkingHandle<proto::network::Message>,
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
        match res {
            Ok(res) => res,
            Err(e) => Err(anyhow!("Task panicked: {}", e)),
        }
    }
}

async fn spawn_p2p_rpc(
    config: &ApplicationConfig,
    networking: &mut NetworkingHandle<proto::network::Message>,
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
fn bootstrap_state<TTx>(tx: &mut TTx) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + DerefMut,
    TTx::Target: StateStoreReadTransaction,
    TTx::Addr: NodeAddressable + Serialize,
{
    let genesis_block = Block::genesis();
    let address = SubstateAddress::Resource(PUBLIC_IDENTITY_RESOURCE_ADDRESS);
    let shard_id = ShardId::from_address(&address, 0);
    let mut metadata: Metadata = Default::default();
    metadata.insert(TOKEN_SYMBOL, "ID".to_string());
    if !SubstateRecord::exists(tx.deref_mut(), &shard_id)? {
        // Create the resource for public identity
        SubstateRecord {
            address,
            version: 0,
            substate_value: Resource::new(
                ResourceType::NonFungible,
                RistrettoPublicKeyBytes::default(),
                OwnerRule::None,
                ResourceAccessRules::new(),
                metadata,
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

    let address = SubstateAddress::Resource(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
    let shard_id = ShardId::from_address(&address, 0);
    let mut metadata = Metadata::new();
    metadata.insert(TOKEN_SYMBOL, "tXTR2".to_string());
    if !SubstateRecord::exists(tx.deref_mut(), &shard_id)? {
        SubstateRecord {
            address,
            version: 0,
            substate_value: Resource::new(
                ResourceType::Confidential,
                RistrettoPublicKeyBytes::default(),
                OwnerRule::None,
                ResourceAccessRules::new(),
                metadata,
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
    HasInvolvedShards::new()
        .and_then(InputRefsValidator::new())
        .and_then(OutputsDontExistLocally::new(store))
}
