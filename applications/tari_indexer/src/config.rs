//  Copyright 2023. The Tari Project
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

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use config::Config;
use serde::{Deserialize, Serialize};
use tari_common::{
    configuration::{serializers, CommonConfig, Network},
    ConfigurationError,
    DefaultConfigLoader,
    SubConfigPath,
};
use tari_comms::multiaddr::Multiaddr;
use tari_engine_types::substate::SubstateAddress;
use tari_p2p::{P2pConfig, PeerSeedsConfig};

#[derive(Debug, Clone)]
pub struct ApplicationConfig {
    pub common: CommonConfig,
    pub indexer: IndexerConfig,
    pub peer_seeds: PeerSeedsConfig,
    pub network: Network,
}

impl ApplicationConfig {
    pub fn load_from(cfg: &Config) -> Result<Self, ConfigurationError> {
        let mut config = Self {
            common: CommonConfig::load_from(cfg)?,
            indexer: IndexerConfig::load_from(cfg)?,
            peer_seeds: PeerSeedsConfig::load_from(cfg)?,
            network: cfg.get("network")?,
        };
        config.indexer.set_base_path(config.common.base_path());
        Ok(config)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct IndexerConfig {
    override_from: Option<String>,
    /// A path to the file that stores your node identity and secret key
    pub identity_file: PathBuf,
    /// A path to the file that stores the tor hidden service private key, if using the tor transport
    pub tor_identity_file: PathBuf,
    /// The node's publicly-accessible hostname
    pub public_address: Option<Multiaddr>,
    /// The Tari base node's GRPC address
    pub base_node_grpc_address: Option<SocketAddr>,
    /// How often do we want to scan the base layer for changes
    #[serde(with = "serializers::seconds")]
    pub base_layer_scanning_interval: Duration,
    /// The relative path to store persistent data
    pub data_dir: PathBuf,
    /// The p2p configuration settings
    pub p2p: P2pConfig,
    /// JSON-RPC address of the indexer application
    pub json_rpc_address: Option<SocketAddr>,
    /// The address of the HTTP UI
    pub http_ui_address: Option<SocketAddr>,
    /// Substate addresses to keep watching
    pub address_watchlist: Vec<SubstateAddress>,
    /// How often do we want to scan the second layer for new versions
    #[serde(with = "serializers::seconds")]
    pub dan_layer_scanning_internal: Duration,
}

impl IndexerConfig {
    pub fn state_db_path(&self) -> PathBuf {
        self.data_dir.join("state.db")
    }

    pub fn set_base_path<P: AsRef<Path>>(&mut self, base_path: P) {
        if !self.identity_file.is_absolute() {
            self.identity_file = base_path.as_ref().join(&self.identity_file);
        }
        if !self.tor_identity_file.is_absolute() {
            self.tor_identity_file = base_path.as_ref().join(&self.tor_identity_file);
        }
        if !self.data_dir.is_absolute() {
            self.data_dir = base_path.as_ref().join(&self.data_dir);
        }
        self.p2p.set_base_path(base_path);
    }
}

impl Default for IndexerConfig {
    fn default() -> Self {
        let p2p = P2pConfig {
            datastore_path: PathBuf::from("data/peer_db"),
            ..Default::default()
        };

        Self {
            override_from: None,
            identity_file: PathBuf::from("indexer_id.json"),
            tor_identity_file: PathBuf::from("indexer_tor_id.json"),
            public_address: None,
            base_node_grpc_address: None,
            base_layer_scanning_interval: Duration::from_secs(10),
            data_dir: PathBuf::from("data/indexer"),
            p2p,
            json_rpc_address: Some("127.0.0.1:18200".parse().unwrap()),
            http_ui_address: Some("127.0.0.1:15000".parse().unwrap()),
            address_watchlist: vec![],
            dan_layer_scanning_internal: Duration::from_secs(10),
        }
    }
}

impl SubConfigPath for IndexerConfig {
    fn main_key_prefix() -> &'static str {
        "indexer"
    }
}
