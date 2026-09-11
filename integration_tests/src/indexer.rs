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

use std::{path::PathBuf, time::Duration};

use multiaddr::Multiaddr;
use reqwest::Url;
use tari_common::configuration::{CommonConfig, StringList};
use tari_indexer::{
    config::{ApplicationConfig, IndexerConfig},
    ipc::{IpcAddPeer, IpcError, IpcMessage},
    run_indexer,
};
use tari_indexer_client::{
    graphql_client::IndexerGraphQLClient,
    rest_api_client::IndexerRestApiClient,
    types::{
        GetNonFungiblesRequest,
        GetSubstateRequest,
        GetSubstateResponse,
        ListTemplateCatalogueRequest,
        ListTemplateCatalogueResponse,
        NonFungibleSubstate,
        TemplateCatalogueItem,
    },
};
use tari_ootle_address::Network;
use tari_ootle_app_utilities::{epoch_oracle_config::EpochOracleConfig, p2p_config::PeerSeedsConfig};
use tari_shutdown::Shutdown;
use tari_template_lib_types::{TemplateAddress, crypto::RistrettoPublicKeyBytes};
use tokio::task;
use tonic::codegen::tokio_stream;

use crate::{
    TariWorld,
    cucumber_log,
    helpers::{check_join_handle, get_address_from_output, get_os_assigned_ports, wait_listener_on_local_port},
    logging::get_base_dir_for_scenario,
};

#[derive(Debug)]
pub struct IndexerProcess {
    pub name: String,
    pub port: u16,
    pub api_port: u16,
    pub graphql_port: u16,
    pub base_node_grpc_port: u16,
    pub web_ui_port: u16,
    pub handle: task::JoinHandle<anyhow::Result<()>>,
    pub ipc_sender: tokio::sync::mpsc::UnboundedSender<Result<IpcMessage, IpcError>>,
    pub temp_dir_path: String,
    pub shutdown: Shutdown,
    pub db_path: PathBuf,
}

impl IndexerProcess {
    pub async fn add_peer(&self, public_key: RistrettoPublicKeyBytes, addresses: Vec<Multiaddr>) {
        self.ipc_sender
            .send(Ok(IpcAddPeer { public_key, addresses }.into()))
            .unwrap();
    }

    pub async fn get_substate(
        &self,
        world: &TariWorld,
        output_ref: String,
        version: u64,
    ) -> Result<GetSubstateResponse, tari_indexer_client::error::IndexerRestClientError> {
        let address = get_address_from_output(world, output_ref);
        let client = self.get_indexer_client();
        client
            .get_substate(address, GetSubstateRequest {
                version: Some(version),
                local_search_only: false,
            })
            .await
    }

    pub async fn get_non_fungibles(
        &self,
        world: &TariWorld,
        output_ref: String,
        start_index: u64,
        end_index: u64,
    ) -> Vec<NonFungibleSubstate> {
        let address = get_address_from_output(world, output_ref);

        let Some(resource) = address.as_resource_address() else {
            panic!("get_non_fungibles: Substate_id {} is not a resource address", address);
        };

        let params = GetNonFungiblesRequest {
            address: resource,
            start_index,
            end_index,
        };

        let client = self.get_indexer_client();
        let resp = client.get_non_fungibles(params).await.unwrap();
        resp.non_fungibles
    }

    pub fn get_indexer_client(&self) -> IndexerRestApiClient {
        let endpoint: Url = Url::parse(&format!("http://localhost:{}", self.api_port)).unwrap();
        IndexerRestApiClient::connect(endpoint).unwrap()
    }

    pub async fn list_template_catalogue(
        &self,
        name_filter: Option<String>,
        limit: Option<u64>,
        after: Option<TemplateAddress>,
    ) -> ListTemplateCatalogueResponse {
        let client = self.get_indexer_client();
        client
            .list_template_catalogue(ListTemplateCatalogueRequest {
                name_filter,
                limit,
                after,
            })
            .await
            .unwrap()
    }

    pub async fn get_template_catalogue_entry(&self, template_address: TemplateAddress) -> TemplateCatalogueItem {
        let client = self.get_indexer_client();
        client.get_template_catalogue_entry(template_address).await.unwrap()
    }

    pub async fn get_graphql_indexer_client(&self) -> IndexerGraphQLClient {
        let endpoint: Url = Url::parse(&format!("http://localhost:{}", self.graphql_port)).unwrap();
        IndexerGraphQLClient::connect(endpoint).unwrap()
    }
}

pub async fn spawn_indexer(world: &mut TariWorld, indexer_name: String, base_node_name: String) {
    // each spawned indexer will use different ports
    let (port, api_port) = get_os_assigned_ports();
    let (graphql_port, web_ui_port) = get_os_assigned_ports();
    let base_node_grpc_port = world.base_nodes.get(&base_node_name).unwrap().grpc_port;
    let name = indexer_name.clone();

    let base_dir = get_base_dir_for_scenario("indexer", world.current_scenario_name.as_ref().unwrap(), &indexer_name);
    let base_dir_path = base_dir.display().to_string();

    // we need to add all the validator nodes as seed peers
    let peer_seeds: Vec<String> = world
        .all_running_validators_iter()
        .map(|vn| format!("{}::/ip4/127.0.0.1/tcp/{}", vn.public_key, vn.p2p_port))
        .collect();

    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();
    let db_path = base_dir.to_path_buf().join("state.db");

    let (ipc_sender, ipc_receiver) = tokio::sync::mpsc::unbounded_channel();
    let localnet_consensus_constants_file = world.consensus_constants_file.clone();

    let handle = task::spawn(async move {
        let mut config = ApplicationConfig {
            common: CommonConfig::default(),
            peer_seeds: PeerSeedsConfig::default(),
            epoch_oracle: EpochOracleConfig::default(),
            network: Network::LocalNet,
            indexer: IndexerConfig::default(),
        };

        // temporal folder for the VN files (e.g. sqlite file, json files, etc.)
        cucumber_log!("Using indexer temp_dir: {}", base_dir.display());
        config.common.base_path = base_dir.to_path_buf();
        config.indexer.data_dir = base_dir.to_path_buf();
        config.indexer.identity_file = base_dir.join("indexer_id.json");
        config.indexer.localnet_consensus_constants_file = localnet_consensus_constants_file;
        config.epoch_oracle.base_layer.base_node_grpc_url =
            Some(format!("http://127.0.0.1:{}", base_node_grpc_port).parse().unwrap());
        config.indexer.block_scanning_interval = Duration::from_secs(5);
        config.indexer.state_scanning_interval = Duration::from_secs(5);
        config.indexer.p2p.listener_port = port;

        config.indexer.p2p.enable_mdns = false;
        config.indexer.api_listen_address = Some(format!("127.0.0.1:{}", api_port).parse().unwrap());
        // OS-assigned port to avoid contention between concurrently running indexers
        config.indexer.metrics_listen_address = Some("127.0.0.1:0".parse().unwrap());
        config.indexer.web_ui_address = Some(format!("127.0.0.1:{}", web_ui_port).parse().unwrap());
        config.indexer.graphql_address = Some(format!("127.0.0.1:{}", graphql_port).parse().unwrap());

        // store all events in the database using an empty filter
        config.indexer.event_filters = vec![];

        // Add all other VNs as peer seeds
        config.peer_seeds.peer_seeds = StringList::from(peer_seeds);
        let ipc_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(ipc_receiver);

        run_indexer(config, ipc_stream, shutdown_signal).await
    });

    // Wait for node to start up
    let handle = wait_listener_on_local_port("Indexer", handle, api_port).await;
    // Check if the task errored/panicked
    let handle = check_join_handle(&name, handle).await;

    // make the new vn able to be referenced by other processes
    let indexer_process = IndexerProcess {
        name: name.clone(),
        port,
        base_node_grpc_port,
        web_ui_port,
        handle,
        api_port,
        graphql_port,
        ipc_sender,
        temp_dir_path: base_dir_path,
        shutdown,
        db_path,
    };

    crate::cucumber_log!("Indexer {} started", indexer_name);
    world.indexers.insert(name, indexer_process);
}
