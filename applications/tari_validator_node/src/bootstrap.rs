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
use tari_app_utilities::{identity_management, identity_management::load_from_json};
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_comms::{protocol::rpc::RpcServer, CommsNode, NodeIdentity, UnspawnedCommsNode};
use tari_dan_app_utilities::{
    base_layer_scanner,
    base_node_client::GrpcBaseNodeClient,
    epoch_manager::EpochManagerHandle,
    template_manager::TemplateManagerHandle,
};
use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight, PayloadId, QuorumCertificate, ShardId, TreeNodeHash};
use tari_dan_core::{
    consensus_constants::ConsensusConstants,
    models::{Payload, SubstateShardData},
    storage::{
        shard_store::{ShardStore, ShardStoreReadTransaction, ShardStoreWriteTransaction},
        StorageError,
    },
    workers::events::{EventSubscription, HotStuffEvent},
};
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::{global::SqliteGlobalDbAdapter, sqlite_shard_store_factory::SqliteShardStore};
use tari_engine_types::{
    resource::Resource,
    substate::{Substate, SubstateAddress},
};
use tari_shutdown::ShutdownSignal;
use tari_template_lib::{
    constants::{CONFIDENTIAL_TARI_RESOURCE_ADDRESS, PUBLIC_IDENTITY_RESOURCE_ADDRESS},
    models::Metadata,
    prelude::ResourceType,
    resource::TOKEN_SYMBOL,
};
use tokio::task::JoinHandle;

use crate::{
    comms,
    dry_run_transaction_processor::DryRunTransactionProcessor,
    p2p::{
        create_validator_node_rpc_service,
        services::{
            comms_peer_provider::CommsPeerProvider,
            epoch_manager,
            hotstuff,
            mempool,
            mempool::MempoolHandle,
            messaging,
            messaging::DanMessageReceivers,
            networking,
            networking::NetworkingHandle,
            rpc_client::TariCommsValidatorNodeClientFactory,
            template_manager,
            template_manager::TemplateManager,
        },
    },
    payload_processor::TariDanPayloadProcessor,
    registration,
    ApplicationConfig,
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
    let (comms, message_channel) = comms::initialize(node_identity.clone(), config, shutdown.clone()).await?;

    // Spawn messaging
    let (message_senders, message_receivers) = messaging::new_messaging_channel(10);
    let (outbound_messaging, join_handle) = messaging::spawn(
        node_identity.public_key().clone(),
        message_channel,
        message_senders.clone(),
    );
    handles.push(join_handle);

    let DanMessageReceivers {
        rx_consensus_message,
        rx_vote_message,
        rx_new_transaction_message,
        rx_network_announce,
        rx_recovery_message,
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
    let shard_store = SqliteShardStore::try_create(config.validator_node.state_db_path())?;
    shard_store.with_write_tx(|tx| bootstrap_state(tx))?;

    // Epoch manager
    let validator_node_client_factory = TariCommsValidatorNodeClientFactory::new(comms.connectivity());
    let (epoch_manager, join_handle) = epoch_manager::spawn(
        global_db.clone(),
        shard_store.clone(),
        base_node_client.clone(),
        consensus_constants.clone(),
        shutdown.clone(),
        node_identity.clone(),
        validator_node_client_factory.clone(),
    );
    handles.push(join_handle);

    // Template manager
    let template_manager = TemplateManager::new(global_db.clone(), config.validator_node.templates.clone());
    let (template_manager_service, join_handle) = template_manager::spawn(template_manager.clone(), shutdown.clone());
    handles.push(join_handle);

    // Mempool
    let (mempool, join_handle) = mempool::spawn(
        rx_new_transaction_message,
        outbound_messaging.clone(),
        epoch_manager.clone(),
        node_identity.clone(),
        template_manager.clone(),
    );
    handles.push(join_handle);

    // Base Node scanner
    let join_handle = base_layer_scanner::spawn(
        global_db,
        base_node_client.clone(),
        epoch_manager.clone(),
        template_manager_service.clone(),
        shutdown.clone(),
        consensus_constants,
        shard_store.clone(),
        config.validator_node.scan_base_layer,
        config.validator_node.base_layer_scanning_interval,
    );
    handles.push(join_handle);

    // Payload processor
    let payload_processor = TariDanPayloadProcessor::new(template_manager);

    // Consensus
    let (hotstuff_events, waiter_join_handle, service_join_handle) = hotstuff::try_spawn(
        node_identity.clone(),
        shard_store.clone(),
        outbound_messaging,
        epoch_manager.clone(),
        mempool.clone(),
        payload_processor.clone(),
        rx_consensus_message,
        rx_recovery_message,
        rx_vote_message,
        shutdown.clone(),
    );
    handles.push(waiter_join_handle);
    handles.push(service_join_handle);

    let dry_run_transaction_processor = DryRunTransactionProcessor::new(
        epoch_manager.clone(),
        payload_processor,
        shard_store.clone(),
        validator_node_client_factory,
        node_identity.clone(),
    );

    let comms = setup_p2p_rpc(config, comms, peer_provider, shard_store.clone(), mempool.clone());
    let comms = comms::spawn_comms_using_transport(comms, p2p_config.transport.clone())
        .await
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Could not spawn using transport: {}", e)))?;

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

    Ok(Services {
        comms,
        networking,
        mempool,
        epoch_manager,
        template_manager: template_manager_service,
        hotstuff_events,
        shard_store,
        dry_run_transaction_processor,
        handles,
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
    pub hotstuff_events: EventSubscription<HotStuffEvent>,
    pub shard_store: SqliteShardStore,
    pub dry_run_transaction_processor: DryRunTransactionProcessor,
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
    shard_store_store: SqliteShardStore,
    mempool: MempoolHandle,
) -> UnspawnedCommsNode {
    let rpc_server = RpcServer::builder()
        .with_maximum_simultaneous_sessions(config.validator_node.p2p.rpc_max_simultaneous_sessions)
        .finish()
        .add_service(create_validator_node_rpc_service(
            peer_provider,
            shard_store_store,
            mempool,
        ));

    comms.add_protocol_extension(rpc_server)
}

// TODO: Figure out the best way to have the engine shard store mirror these bootstrapped states.
fn bootstrap_state<T, TAddr, TPayload>(tx: &mut T) -> Result<(), StorageError>
where
    T: ShardStoreWriteTransaction<TAddr, TPayload> + DerefMut,
    T::Target: ShardStoreReadTransaction<TAddr, TPayload>,
    TAddr: NodeAddressable,
    TPayload: Payload,
{
    let genesis_payload = PayloadId::new([0u8; 32]);

    let address = SubstateAddress::Resource(PUBLIC_IDENTITY_RESOURCE_ADDRESS);
    let shard_id = ShardId::from_address(&address, 0);
    if tx.get_substate_states(&[shard_id])?.is_empty() {
        // Create the resource for public identity
        tx.insert_substates(SubstateShardData::new(
            shard_id,
            address,
            0,
            Substate::new(0, Resource::new(ResourceType::NonFungible, Default::default())),
            NodeHeight(0),
            None,
            TreeNodeHash::zero(),
            None,
            genesis_payload,
            None,
            QuorumCertificate::genesis(Epoch(0), genesis_payload, shard_id),
            None,
        ))?;
    }

    let address = SubstateAddress::Resource(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
    let shard_id = ShardId::from_address(&address, 0);
    if tx.get_substate_states(&[shard_id])?.is_empty() {
        // Create the second layer tari resource
        let mut metadata = Metadata::new();
        // TODO: decide on symbol for L2 tari
        metadata.insert(TOKEN_SYMBOL, "tXTR".to_string());

        tx.insert_substates(SubstateShardData::new(
            shard_id,
            address,
            0,
            Substate::new(0, Resource::new(ResourceType::Confidential, metadata)),
            NodeHeight(0),
            None,
            TreeNodeHash::zero(),
            None,
            genesis_payload,
            None,
            QuorumCertificate::genesis(Epoch(0), genesis_payload, shard_id),
            None,
        ))?;
    }

    Ok(())
}
