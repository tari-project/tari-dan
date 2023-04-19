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

use std::{net::SocketAddr, time::Duration};

use config::Config;
use serde::{Deserialize, Serialize};
use tari_common::{configuration::CommonConfig, ConfigurationError, DefaultConfigLoader, SubConfigPath};

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
    pub listen_addr: Option<SocketAddr>,
    /// The signaling server address for the webrtc
    pub signaling_server_addr: Option<SocketAddr>,
    /// The validator nodes jrpc endpoint url
    pub validator_node_endpoint: Option<String>,
    /// Expiration duration of the JWT token
    pub jwt_expiration: Option<Duration>,
    /// Secret key for the JWT token.
    pub secret_key: Option<String>,
}

impl Default for WalletDaemonConfig {
    fn default() -> Self {
        Self {
            override_from: None,
            listen_addr: Some(SocketAddr::from(([127u8, 0, 0, 1], 9000))),
            signaling_server_addr: Some(SocketAddr::from(([127u8, 0, 0, 1], 9100))),
            validator_node_endpoint: Some("http://127.0.0.1:18200/json_rpc".to_string()),
            // TODO: Come up with a reasonable default value
            jwt_expiration: Some(Duration::from_secs(5 * 60)),
            // TODO: Generate a random secret key at start if not set by hand. Otherwise anyone can generate a JWT token
            // when they know the secret_key.
            secret_key: Some("secret_key".to_string()),
        }
    }
}

impl SubConfigPath for WalletDaemonConfig {
    fn main_key_prefix() -> &'static str {
        "dan_wallet_daemon"
    }
}
