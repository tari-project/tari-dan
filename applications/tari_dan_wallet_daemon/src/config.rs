//  Copyright 2022. The Tari Project
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

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use config::Config;
use serde::{Deserialize, Serialize};
use tari_common::{configuration::CommonConfig, ConfigurationError, DefaultConfigLoader, SubConfigPath};
use tari_dan_common_types::crypto::create_secret;

#[derive(Debug, Clone)]
pub struct ApplicationConfig {
    pub common: CommonConfig,
    pub dan_wallet_daemon: WalletDaemonConfig,
}

impl ApplicationConfig {
    pub fn load_from(cfg: &Config) -> Result<Self, ConfigurationError> {
        let config = Self {
            common: CommonConfig::load_from(cfg)?,
            dan_wallet_daemon: WalletDaemonConfig::load_from(cfg)?,
        };
        Ok(config)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct WalletDaemonConfig {
    override_from: Option<String>,
    /// The wallet daemon listening address
    pub json_rpc_address: Option<SocketAddr>,
    /// The jrpc address where the UI should connect (it can be the same as the json_rpc_addr, but doesn't have to be),
    /// if this will be None, then the listen_addr will be used.
    pub ui_connect_address: Option<String>,
    /// The signaling server address for the webrtc
    pub signaling_server_address: Option<SocketAddr>,
    /// The validator nodes jrpc endpoint url
    pub indexer_node_json_rpc_url: String,
    /// Expiration duration of the JWT token
    #[serde(with = "humantime_serde::option")]
    pub jwt_expiry: Option<Duration>,
    /// Secret key for the JWT token.
    pub jwt_secret_key: Option<String>,
    /// The address of the HTTP UI
    pub http_ui_address: Option<SocketAddr>,
    /// The path to the value lookup table binary file used for brute force value lookups. This setting
    /// is only used when attempting to view confidential balances in confidential resources that use a view key
    /// controlled by this wallet. The binary file can be generated using the generate_ristretto_value_lookup
    /// utility. If this is not set, the value lookup table will be generated on the fly which will have a large
    /// performance cost when brute forcing high-value outputs.
    pub value_lookup_table_file: Option<PathBuf>,
}

impl Default for WalletDaemonConfig {
    fn default() -> Self {
        Self {
            override_from: None,
            json_rpc_address: Some(SocketAddr::from(([127u8, 0, 0, 1], 9000))),
            ui_connect_address: None,
            signaling_server_address: Some(SocketAddr::from(([127u8, 0, 0, 1], 9100))),
            indexer_node_json_rpc_url: "http://127.0.0.1:18300/json_rpc".to_string(),
            // TODO: Come up with a reasonable default value
            jwt_expiry: Some(Duration::from_secs(500 * 60)),
            jwt_secret_key: Some(create_secret()),
            http_ui_address: Some("127.0.0.1:5100".parse().unwrap()),
            value_lookup_table_file: None,
        }
    }
}

impl SubConfigPath for WalletDaemonConfig {
    fn main_key_prefix() -> &'static str {
        "dan_wallet_daemon"
    }
}
