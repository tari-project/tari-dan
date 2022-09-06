//  Copyright 2021. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::sync::Arc;

use tari_app_utilities::{identity_management, identity_management::load_from_json};
use tari_common::exit_codes::{ExitCode, ExitError};
use tari_comms::{protocol::rpc::RpcServer, NodeIdentity, UnspawnedCommsNode};
use tari_comms_dht::Dht;
use tari_dan_core::services::mempool::service::MempoolServiceHandle;
use tari_p2p::{
    comms_connector::{pubsub_connector, SubscriptionFactory},
    initialization::{spawn_comms_using_transport, P2pInitializer},
};
use tari_service_framework::{ServiceHandles, StackBuilder};
use tari_shutdown::ShutdownSignal;

use crate::{
    config::ApplicationConfig,
    p2p::services::{
        epoch_manager::initializer::EpochManagerInitializer,
        hotstuff::initializer::HotstuffServiceInitializer,
        mempool::initializer::MempoolInitializer,
    },
};

pub async fn build_service_and_comms_stack(
    config: &ApplicationConfig,
    shutdown: ShutdownSignal,
    node_identity: Arc<NodeIdentity>,
    mempool: MempoolServiceHandle,
) -> Result<(ServiceHandles, Arc<SubscriptionFactory>), ExitError> {
    let (publisher, peer_message_subscriptions) = pubsub_connector(100, 50);
    let peer_message_subscriptions = Arc::new(peer_message_subscriptions);

    let mut p2p_config = config.validator_node.p2p.clone();
    p2p_config.transport.tor.identity = load_from_json(&config.validator_node.tor_identity_file)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, e))?;

    let mut handles = StackBuilder::new(shutdown.clone())
        .add_initializer(P2pInitializer::new(
            p2p_config.clone(),
            config.peer_seeds.clone(),
            config.network,
            node_identity.clone(),
            publisher,
        ))
        .add_initializer(EpochManagerInitializer {})
        .add_initializer(HotstuffServiceInitializer {
            node_identity: node_identity.clone(),
        })
        .add_initializer(MempoolInitializer::new(
            mempool.clone(),
            peer_message_subscriptions.clone(),
        ))
        .build()
        .await
        .map_err(|err| ExitError::new(ExitCode::ConfigError, err.to_string()))?;

    let comms = handles
        .take_handle::<UnspawnedCommsNode>()
        .expect("P2pInitializer was not added to the stack or did not add UnspawnedCommsNode");

    let comms = setup_p2p_rpc(config, comms, &handles, mempool);

    let comms = spawn_comms_using_transport(comms, p2p_config.transport.clone())
        .await
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Could not spawn using transport: {}", e)))?;

    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    identity_management::save_as_json(&config.validator_node.identity_file, &*comms.node_identity())
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save node identity: {}", e)))?;
    if let Some(hs) = comms.hidden_service() {
        identity_management::save_as_json(&config.validator_node.tor_identity_file, hs.tor_identity())
            .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save tor identity: {}", e)))?;
    }

    handles.register(comms);
    Ok((handles, peer_message_subscriptions))
}

fn setup_p2p_rpc(
    config: &ApplicationConfig,
    comms: UnspawnedCommsNode,
    handles: &ServiceHandles,
    _mempool: MempoolServiceHandle,
) -> UnspawnedCommsNode {
    let dht = handles.expect_handle::<Dht>();
    let rpc_server = RpcServer::builder()
        .with_maximum_simultaneous_sessions(config.validator_node.p2p.rpc_max_simultaneous_sessions)
        .finish()
        .add_service(dht.rpc_service());
    // .add_service(create_validator_node_rpc_service(mempool, db_factory));

    comms.add_protocol_extension(rpc_server)
}
