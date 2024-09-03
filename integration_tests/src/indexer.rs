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
    collections::HashMap,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use reqwest::Url;
use tari_common::{
    configuration::{CommonConfig, StringList},
    exit_codes::ExitError,
};
use tari_crypto::tari_utilities::{hex::Hex, message_format::MessageFormat};
use tari_dan_app_utilities::p2p_config::PeerSeedsConfig;
use tari_engine_types::substate::SubstateId;
use tari_indexer::{
    config::{ApplicationConfig, EventFilterConfig, IndexerConfig},
    run_indexer,
};
use tari_indexer_client::{
    graphql_client::IndexerGraphQLClient,
    json_rpc_client::IndexerJsonRpcClient,
    types::{GetNonFungiblesRequest, GetSubstateRequest, GetSubstateResponse, NonFungibleSubstate},
};
use tari_p2p::Network;
use tari_shutdown::Shutdown;
use tari_template_lib::models::ObjectKey;
use tokio::task;

use crate::{
    helpers::{check_join_handle, get_os_assigned_ports, wait_listener_on_local_port},
    logging::get_base_dir_for_scenario,
    TariWorld,
};

#[derive(Debug)]
pub struct IndexerProcess {
    pub name: String,
    pub port: u16,
    pub json_rpc_port: u16,
    pub graphql_port: u16,
    pub base_node_grpc_port: u16,
    pub http_ui_port: u16,
    pub handle: task::JoinHandle<Result<(), ExitError>>,
    pub temp_dir_path: String,
    pub shutdown: Shutdown,
    pub db_path: PathBuf,
}

impl IndexerProcess {
    pub async fn get_substate(&self, world: &TariWorld, output_ref: String, version: u32) -> GetSubstateResponse {
        let address = get_address_from_output(world, output_ref);

        let mut jrpc_client = self.get_jrpc_indexer_client();
        jrpc_client
            .get_substate(GetSubstateRequest {
                address: address.clone(),
                version: Some(version),
                local_search_only: true,
            })
            .await
            .unwrap()
    }

    pub async fn get_non_fungibles(
        &self,
        world: &TariWorld,
        output_ref: String,
        start_index: u64,
        end_index: u64,
    ) -> Vec<NonFungibleSubstate> {
        let address = get_address_from_output(world, output_ref);

        let params = GetNonFungiblesRequest {
            address: address.clone(),
            start_index,
            end_index,
        };

        let mut jrpc_client = self.get_jrpc_indexer_client();
        let resp = jrpc_client.get_non_fungibles(params).await.unwrap();
        resp.non_fungibles
    }

    pub async fn insert_event_mock_data(&mut self) {
        let mut graphql_client = self.get_graphql_indexer_client().await;
        let substate_id = SubstateId::Component(ObjectKey::default().into()).to_string();
        let template_address = [0u8; 32].to_hex();
        let tx_hash = [0u8; 32].to_hex();
        let topic = "my_event".to_string();
        let version = 0;
        let payload = HashMap::<String, String>::from([("my".to_string(), "event".to_string())])
            .to_json()
            .unwrap();
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let query = format!(
            "{{ saveEvent(substateId: {:?}, templateAddress: {:?}, txHash: {:?}, topic: {:?}, payload: {:?}, version: \
             {:?}, timestamp: {}) {{ substateId templateAddress txHash topic payload }} }}",
            substate_id, template_address, tx_hash, topic, payload, version, timestamp
        );
        let res = graphql_client
            .send_request::<HashMap<String, tari_indexer::graphql::model::events::Event>>(&query, None, None)
            .await
            .unwrap_or_else(|e| panic!("Failed to save event via graphql client: {}", e));
        let res = res.get("saveEvent").unwrap();

        assert_eq!(res.substate_id, Some(substate_id.to_string()));
    }

    pub fn get_jrpc_indexer_client(&self) -> IndexerJsonRpcClient {
        let endpoint: Url = Url::parse(&format!("http://localhost:{}", self.json_rpc_port)).unwrap();
        IndexerJsonRpcClient::connect(endpoint).unwrap()
    }

    pub async fn get_graphql_indexer_client(&self) -> IndexerGraphQLClient {
        let endpoint: Url = Url::parse(&format!("http://localhost:{}", self.graphql_port)).unwrap();
        IndexerGraphQLClient::connect(endpoint).unwrap()
    }
}

fn get_address_from_output(world: &TariWorld, output_ref: String) -> &SubstateId {
    world
        .outputs
        .iter()
        .find_map(|(name, outputs)| {
            outputs
                .iter()
                .find(|(child_name, _)| {
                    let fqn = format!("{}/{}", name, child_name);
                    fqn == output_ref
                })
                .map(|(_, addr)| &addr.substate_id)
        })
        .unwrap()
}

pub async fn spawn_indexer(world: &mut TariWorld, indexer_name: String, base_node_name: String) {
    // each spawned indexer will use different ports
    let (port, json_rpc_port) = get_os_assigned_ports();
    let (graphql_port, http_ui_port) = get_os_assigned_ports();
    let base_node_grpc_port = world.base_nodes.get(&base_node_name).unwrap().grpc_port;
    let name = indexer_name.clone();

    let base_dir = get_base_dir_for_scenario("indexer", world.current_scenario_name.as_ref().unwrap(), &indexer_name);
    let base_dir_path = base_dir.display().to_string();

    // we need to add all the validator nodes as seed peers
    let peer_seeds: Vec<String> = world
        .all_validators_iter()
        .map(|vn| format!("{}::/ip4/127.0.0.1/tcp/{}", vn.public_key, vn.port))
        .collect();

    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();
    let db_path = base_dir.to_path_buf().join("state.db");

    let handle = task::spawn(async move {
        let mut config = ApplicationConfig {
            common: CommonConfig::default(),
            peer_seeds: PeerSeedsConfig::default(),
            network: Network::LocalNet,
            indexer: IndexerConfig::default(),
        };

        // temporal folder for the VN files (e.g. sqlite file, json files, etc.)
        println!("Using indexer temp_dir: {}", base_dir.display());
        config.common.base_path = base_dir.to_path_buf();
        config.indexer.data_dir = base_dir.to_path_buf();
        config.indexer.identity_file = base_dir.join("indexer_id.json");
        config.indexer.tor_identity_file = base_dir.join("indexer_tor_id.json");
        config.indexer.base_node_grpc_url = Some(format!("http://127.0.0.1:{}", base_node_grpc_port).parse().unwrap());
        config.indexer.dan_layer_scanning_internal = Duration::from_secs(5);
        config.indexer.p2p.listener_port = port;

        config.indexer.p2p.enable_mdns = false;
        config.indexer.json_rpc_address = Some(format!("127.0.0.1:{}", json_rpc_port).parse().unwrap());
        config.indexer.http_ui_address = Some(format!("127.0.0.1:{}", http_ui_port).parse().unwrap());
        config.indexer.graphql_address = Some(format!("127.0.0.1:{}", graphql_port).parse().unwrap());

        // store all events in the database using an empty filter
        config.indexer.event_filters = vec![EventFilterConfig {
            topic: None,
            entity_id: None,
            substate_id: None,
            template_address: None,
        }];

        // Add all other VNs as peer seeds
        config.peer_seeds.peer_seeds = StringList::from(peer_seeds);

        run_indexer(config, shutdown_signal).await
    });

    // Wait for node to start up
    let handle = wait_listener_on_local_port(handle, json_rpc_port).await;
    // Check if the task errored/panicked
    let handle = check_join_handle(&name, handle).await;

    // make the new vn able to be referenced by other processes
    let indexer_process = IndexerProcess {
        name: name.clone(),
        port,
        base_node_grpc_port,
        http_ui_port,
        handle,
        json_rpc_port,
        graphql_port,
        temp_dir_path: base_dir_path,
        shutdown,
        db_path,
    };
    world.indexers.insert(name, indexer_process);
}
