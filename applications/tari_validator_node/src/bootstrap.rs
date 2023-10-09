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

use std::{
    fs,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    ops::DerefMut,
    sync::Arc,
};

use anyhow::anyhow;
use futures::{future, FutureExt};
use log::info;
use minotari_app_utilities::{identity_management, identity_management::load_from_json};
use serde::Serialize;
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_common_types::types::PublicKey;
use tari_comms::{protocol::rpc::RpcServer, types::CommsPublicKey, CommsNode, NodeIdentity, UnspawnedCommsNode};
use tari_dan_app_utilities::{
    base_layer_scanner,
    consensus_constants::ConsensusConstants,
    template_manager,
    template_manager::{implementation::TemplateManager, interface::TemplateManagerHandle},
    transaction_executor::TariDanTransactionProcessor,
};
use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight, ShardId};
use tari_dan_engine::fees::FeeTable;
use tari_dan_storage::{
    consensus_models::{Block, ExecutedTransaction, SubstateRecord},
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
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_template_lib::{
    constants::{CONFIDENTIAL_TARI_RESOURCE_ADDRESS, PUBLIC_IDENTITY_RESOURCE_ADDRESS},
    models::Metadata,
    prelude::ResourceType,
};
use tari_transaction::Transaction;
use tari_validator_node_rpc::client::TariCommsValidatorNodeClientFactory;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    comms,
    consensus,
    consensus::ConsensusHandle,
    dry_run_transaction_processor::DryRunTransactionProcessor,
    p2p::{
        create_tari_validator_node_rpc_service,
        services::{
            comms_peer_provider::CommsPeerProvider,
            mempool,
            mempool::{
                ClaimFeeTransactionValidator,
                FeeTransactionValidator,
                InputRefsValidator,
                MempoolError,
                MempoolHandle,
                OutputsDontExistLocally,
                TemplateExistsValidator,
                Validator,
            },
            messaging,
            messaging::DanMessageReceivers,
            networking,
            networking::NetworkingHandle,
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
    node_identity: Arc<NodeIdentity>,
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    consensus_constants: ConsensusConstants,
) -> Result<Services, anyhow::Error> {
    let mut handles = Vec::with_capacity(3);
    let mut p2p_config = config.validator_node.p2p.clone();
    p2p_config.transport.tor.identity = load_from_json(&config.validator_node.tor_identity_file)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, e))?;

    ensure_directories_exist(config)?;

    // Connection to base node
    let base_node_client = GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address.unwrap_or_else(|| {
        let port = grpc_default_port(ApplicationType::BaseNode, config.network);
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }));

    // Initialize comms
    let (comms, message_channels) = comms::initialize(node_identity.clone(), config, shutdown.clone()).await?;

    // Spawn messaging
    let (message_senders, message_receivers) = messaging::new_messaging_channel(30);
    let (outbound_messaging, join_handle) = messaging::spawn(
        node_identity.public_key().clone(),
        message_channels,
        message_senders.clone(),
    );
    handles.push(join_handle);

    let DanMessageReceivers {
        rx_consensus_message,
        rx_new_transaction_message,
        rx_network_announce,
    } = message_receivers;

    // Networking
    let peer_provider = CommsPeerProvider::new(comms.peer_manager());
    let (networking, join_handle) = networking::spawn(
        rx_network_announce,
        node_identity.clone(),
        outbound_messaging.clone(),
        peer_provider.clone(),
        comms.connectivity(),
    );
    handles.push(join_handle);

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
        node_identity.public_key().clone(),
        shutdown.clone(),
    );
    handles.push(join_handle);

    // Template manager
    let template_manager = TemplateManager::initialize(global_db.clone(), config.validator_node.templates.clone())?;
    let (template_manager_service, join_handle) =
        template_manager::implementation::spawn(template_manager.clone(), shutdown.clone());
    handles.push(join_handle);

    // Payload processor
    let fee_table = if config.validator_node.no_fees {
        FeeTable::zero_rated()
    } else {
        FeeTable::new(1, 1)
    };
    let payload_processor = TariDanTransactionProcessor::new(template_manager.clone(), fee_table);

    let validator_node_client_factory = TariCommsValidatorNodeClientFactory::new(comms.connectivity());

    // Consensus
    let (tx_executed_transaction, rx_executed_transaction) = mpsc::channel(10);
    let (consensus_join_handle, consensus_handle, rx_consensus_to_mempool) = consensus::spawn(
        state_store.clone(),
        node_identity.clone(),
        epoch_manager.clone(),
        rx_executed_transaction,
        rx_consensus_message,
        outbound_messaging.clone(),
        validator_node_client_factory.clone(),
        shutdown.clone(),
    )
    .await;
    handles.push(consensus_join_handle);

    // Mempool
    let virtual_substate_manager = VirtualSubstateManager::new(state_store.clone(), epoch_manager.clone());
    let scanner = SubstateScanner::new(epoch_manager.clone(), validator_node_client_factory.clone());
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
        node_identity.clone(),
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

    let comms = setup_p2p_rpc(
        config,
        comms,
        peer_provider,
        state_store.clone(),
        mempool.clone(),
        virtual_substate_manager,
    );
    let comms = comms::spawn_comms_using_transport(comms, p2p_config.transport.clone())
        .await
        .map_err(|e| {
            ExitError::new(
                ExitCode::NetworkError,
                format!("Could not spawn using transport: {}", e),
            )
        })?;

    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    save_identities(config, &comms)?;

    // Auto-registration
    if config.validator_node.auto_register {
        let handle = registration::spawn(config.clone(), node_identity.clone(), epoch_manager.clone(), shutdown);
        handles.push(handle);
    } else {
        info!(target: LOG_TARGET, "♽️ Node auto registration is disabled");
    }

    let dry_run_transaction_processor =
        DryRunTransactionProcessor::new(epoch_manager.clone(), payload_processor, substate_resolver);

    Ok(Services {
        comms,
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

fn save_identities(config: &ApplicationConfig, comms: &CommsNode) -> Result<(), ExitError> {
    identity_management::save_as_json(&config.validator_node.identity_file, &*comms.node_identity())
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save node identity: {}", e)))?;

    if let Some(hs) = comms.hidden_service() {
        identity_management::save_as_json(&config.validator_node.tor_identity_file, hs.tor_identity())
            .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save tor identity: {}", e)))?;
    }
    Ok(())
}

fn ensure_directories_exist(config: &ApplicationConfig) -> io::Result<()> {
    fs::create_dir_all(&config.validator_node.data_dir)?;
    fs::create_dir_all(&config.validator_node.p2p.datastore_path)?;
    Ok(())
}

pub struct Services {
    pub comms: CommsNode,
    pub networking: NetworkingHandle,
    pub mempool: MempoolHandle,
    pub epoch_manager: EpochManagerHandle,
    pub template_manager: TemplateManagerHandle,
    pub consensus_handle: ConsensusHandle,
    pub global_db: GlobalDb<SqliteGlobalDbAdapter>,
    pub dry_run_transaction_processor: DryRunTransactionProcessor,
    pub validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    pub state_store: SqliteStateStore<PublicKey>,

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

fn setup_p2p_rpc(
    config: &ApplicationConfig,
    comms: UnspawnedCommsNode,
    peer_provider: CommsPeerProvider,
    shard_store_store: SqliteStateStore<CommsPublicKey>,
    mempool: MempoolHandle,
    virtual_substate_manager: VirtualSubstateManager<SqliteStateStore<PublicKey>, EpochManagerHandle>,
) -> UnspawnedCommsNode {
    let rpc_server = RpcServer::builder()
        .with_maximum_simultaneous_sessions(config.validator_node.p2p.rpc_max_simultaneous_sessions)
        .finish()
        .add_service(create_tari_validator_node_rpc_service(
            peer_provider,
            shard_store_store,
            mempool,
            virtual_substate_manager,
        ));

    comms.add_protocol_extension(rpc_server)
}

// TODO: Figure out the best way to have the engine shard store mirror these bootstrapped states.
fn bootstrap_state<TTx>(tx: &mut TTx) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + DerefMut,
    TTx::Target: StateStoreReadTransaction,
    TTx::Addr: NodeAddressable + Serialize,
{
    let genesis_block = Block::<TTx::Addr>::genesis();
    let address = SubstateAddress::Resource(PUBLIC_IDENTITY_RESOURCE_ADDRESS);
    let shard_id = ShardId::from_address(&address, 0);
    if !SubstateRecord::exists(tx.deref_mut(), &shard_id)? {
        // Create the resource for public identity
        SubstateRecord {
            address,
            version: 0,
            substate_value: Resource::new(ResourceType::NonFungible, "ID".to_string(), Default::default()).into(),
            state_hash: Default::default(),
            created_by_transaction: Default::default(),
            created_justify: *genesis_block.justify().id(),
            created_block: *genesis_block.id(),
            created_height: NodeHeight(0),
            created_at_epoch: Epoch(0),
            destroyed: None,
        }
        .create(tx)?;
    }

    let address = SubstateAddress::Resource(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
    let shard_id = ShardId::from_address(&address, 0);
    if !SubstateRecord::exists(tx.deref_mut(), &shard_id)? {
        SubstateRecord {
            address,
            version: 0,
            substate_value: Resource::new(ResourceType::Confidential, "tXTR2".to_string(), Metadata::new()).into(),
            state_hash: Default::default(),
            created_by_transaction: Default::default(),
            created_justify: *genesis_block.justify().id(),
            created_block: *genesis_block.id(),
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
    template_manager: TemplateManager,
    epoch_manager: EpochManagerHandle,
) -> impl Validator<Transaction, Error = MempoolError> {
    let mut validator = TemplateExistsValidator::new(template_manager)
        .and_then(ClaimFeeTransactionValidator::new(epoch_manager))
        .boxed();
    if !config.no_fees {
        validator = validator.and_then(FeeTransactionValidator).boxed();
    }
    validator
}

fn create_mempool_after_execute_validator(
    store: SqliteStateStore<CommsPublicKey>,
) -> impl Validator<ExecutedTransaction, Error = MempoolError> {
    InputRefsValidator::new().and_then(OutputsDontExistLocally::new(store))
}
