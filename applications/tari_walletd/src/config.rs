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

use std::{
    fmt::{Display, Formatter},
    net::SocketAddr,
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

use anyhow::anyhow;
use config::Config;
use serde::{Deserialize, Deserializer, Serialize, de};
use tari_common::{ConfigurationError, DefaultConfigLoader, SubConfigPath, configuration::CommonConfig};
use tari_crypto::tari_utilities::SafePassword;
use tari_ootle_address::Network;
use tari_ootle_wallet_sdk::IndexerTrust;
use url::Url;

#[derive(Debug, Clone)]
pub struct ApplicationConfig {
    pub common: CommonConfig,
    pub ootle_wallet_daemon: WalletDaemonConfig,
}

impl ApplicationConfig {
    pub fn load_from(cfg: &Config) -> Result<Self, ConfigurationError> {
        let config = Self {
            common: CommonConfig::load_from(cfg)?,
            ootle_wallet_daemon: WalletDaemonConfig::load_from(cfg)?,
        };
        Ok(config)
    }

    pub fn to_data_dir(&self) -> PathBuf {
        self.common.base_path.join("data")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct WalletDaemonConfig {
    override_from: Option<String>,
    /// Network that is used to send transactions.
    pub network: Network,
    /// Authentication method.
    pub authentication: WalletDaemonAuth,
    /// The wallet daemon listening address
    #[serde(default = "return_default_rpc_address")]
    pub json_rpc_address: SocketAddr,
    /// The signaling server address for the webrtc
    pub signaling_server_address: Option<SocketAddr>,
    /// Allow `webrtc.start`, which hands an app connected through the signaling
    /// server a token carrying the caller's permissions. Off unless an operator
    /// opts in, since any credential holding `webrtc` can open such a session.
    #[serde(default)]
    pub enable_webrtc: bool,
    /// The indexer API URLs, under the key `indexer_api_url`. The wallet uses one at a time, chosen at random at
    /// startup, and moves to the next when the one in use stops responding. Accepts a list, or a string of
    /// comma-separated URLs.
    #[serde(rename = "indexer_api_url", deserialize_with = "deserialize_indexer_urls")]
    pub indexer_api_urls: Vec<Url>,
    /// How far the wallet trusts its indexer. `verify` (the default) commits a transaction's changes only once a proof
    /// of them verifies against the network's validator sets, and needs indexers that verify substate proofs. `trust`
    /// takes the indexer at its word, for development only.
    #[serde(default)]
    pub indexer_trust: IndexerTrust,
    /// Expiration duration of the JWT token
    #[serde(with = "humantime_serde")]
    #[serde(default = "return_default_jwt_expiry")]
    pub jwt_expiry: Duration,
    /// How long a person has to approve a transaction request. The locks
    /// holding the request's inputs are extended past this, so it also decides
    /// how long those funds stay unavailable if nobody ever answers.
    #[serde(default = "return_default_transaction_request_ttl")]
    pub transaction_request_ttl: Duration,
    /// How long a person has to approve a signing request when the requester
    /// does not set its own window. Co-signers of a council transaction are
    /// often not at their wallet, so this is longer than the transaction-request
    /// window.
    #[serde(default = "return_default_signing_request_ttl")]
    pub signing_request_ttl: Duration,
    /// If true, the wallet daemon will allow CORS requests from any origin. This is useful for development and
    /// testing, but should be disabled in production.
    pub enable_permissive_cors: bool,
    /// Host names, beyond `localhost`, IP addresses and the host of `webauthn.rp_origin`, that a request may address
    /// the daemon by: the name a reverse proxy forwards, or a LAN or container host name. Any other name is refused,
    /// so that a web page served under an unrelated name cannot reach the daemon. Accepts a list, or a string of
    /// comma-separated names. `*` accepts every name.
    #[serde(default, deserialize_with = "deserialize_string_list")]
    pub allowed_hosts: Vec<String>,
    pub webauthn: WebAuthnConfig,
    /// The path to the value lookup table binary file used for brute force value lookups. This setting
    /// is only used when attempting to view confidential balances in confidential resources that use a view key
    /// controlled by this wallet. The binary file can be generated using the generate_ristretto_value_lookup
    /// utility. If this is not set, the value lookup table will be generated on the fly which will have a large
    /// performance cost when brute forcing high-value outputs.
    pub value_lookup_table_file: Option<PathBuf>,
    /// The number of contiguous failures to find an account derived from a public key before abandoning recovery and
    /// assuming that there are no further accounts.
    pub recovery_abandon_count: usize,
    /// Directory where burn proof files are read from after a burn transaction completes.
    /// If not set, defaults to the platform data directory (e.g. ~/.local/share/tari/{network}/burn_proofs on Linux,
    /// ~/Library/Application Support/tari/{network}/burn_proofs on macOS).
    pub burn_proof_dir: Option<PathBuf>,
    pub override_keyring_password: Option<SafePassword>,
    /// If true (default), the wallet daemon will watch the burn_proof_dir and automatically submit claim
    /// transactions when new burn proof files are detected, deferring each claim to the next epoch as required.
    #[serde(default = "return_default_auto_claim_burns")]
    pub auto_claim_burns: bool,
    /// How many epochs ahead of the current epoch to stamp `max_epoch` on transactions this wallet
    /// builds, when the caller does not supply one. Every transaction must declare the last epoch it
    /// may be sequenced in; past it the transaction can never land, so this decides how long a built
    /// transaction stays submittable. Must not exceed the network's `max_transaction_validity_epochs`
    /// ceiling — the default is far below it, leaving the long windows to callers that ask for one
    /// explicitly (e.g. offline or multi-party signing).
    #[serde(default = "return_default_transaction_validity_epochs")]
    pub default_transaction_validity_epochs: u64,
    /// The largest `max_fee`, in microtari, that this wallet submits a transaction with. A request
    /// above it is refused before the transaction is built, whichever handler it comes through, and
    /// an automatic burn claim never reveals more than it. Dry runs commit nothing, so they are
    /// exempt. Set to `18446744073709551615` (u64::MAX) to disable.
    #[serde(default = "return_default_max_fee_limit")]
    pub max_fee_limit: u64,
}

impl WalletDaemonConfig {
    /// Rejects a configuration that cannot build a usable transaction.
    ///
    /// A zero window stamps `max_epoch == current_epoch`, so every transaction the daemon builds
    /// must be sequenced within the epoch it was built in and one built near a boundary expires
    /// before it can land. An oversized window is only warned about: it is the network's ceiling
    /// that decides, and this process cannot read it.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        if self.indexer_api_urls.is_empty() {
            return Err(anyhow::anyhow!("indexer_api_url must name at least one indexer"));
        }
        if let Some(url) = self
            .indexer_api_urls
            .iter()
            .find(|url| !matches!(url.scheme(), "http" | "https"))
        {
            return Err(anyhow::anyhow!(
                "indexer_api_url: the indexer is reached over HTTP, but a URL has scheme '{}'",
                url.scheme()
            ));
        }
        if self.default_transaction_validity_epochs == 0 {
            return Err(anyhow::anyhow!(
                "default_transaction_validity_epochs must be at least 1: a zero window expires transactions in the \
                 epoch they are built in"
            ));
        }
        if self.max_fee_limit == 0 {
            return Err(anyhow::anyhow!(
                "max_fee_limit must be at least 1: every transaction pays a fee, so a zero limit refuses them all"
            ));
        }
        if self.default_transaction_validity_epochs > IMPLAUSIBLE_TRANSACTION_VALIDITY_EPOCHS {
            log::warn!(
                target: LOG_TARGET,
                "default_transaction_validity_epochs ({}) is beyond the transaction validity ceiling known to this \
                 build ({}). If the network enforces that ceiling, every transaction this wallet builds will be \
                 refused as out of range.",
                self.default_transaction_validity_epochs,
                IMPLAUSIBLE_TRANSACTION_VALIDITY_EPOCHS,
            );
        }
        Ok(())
    }

    pub fn get_burn_proof_dir(&self, network: Network) -> PathBuf {
        self.burn_proof_dir
            .clone()
            .unwrap_or_else(|| default_burn_proof_dir(network))
    }
}

fn return_default_auto_claim_burns() -> bool {
    true
}

/// Three epochs is roughly an hour at the ~20 minute epoch target: long enough for an interactive
/// approval flow, short enough that an abandoned transaction stops being submittable quickly.
fn return_default_transaction_validity_epochs() -> u64 {
    3
}

/// 100 TARI. Publishing a template at the engine's binary size limit is the costliest transaction on
/// the shipped fee tables, at roughly 90 TARI, so the default admits every transaction those tables price.
fn return_default_max_fee_limit() -> u64 {
    100_000_000
}

/// Mirrors `ConsensusConstants::max_transaction_validity_epochs`. The wallet binary does not carry
/// the consensus crate, so the value is duplicated here and pinned to the real one by a test; it is
/// used only to warn an operator that their configured window looks implausible, never as a hard
/// limit, which would wrongly refuse to start against a network with a larger ceiling.
const IMPLAUSIBLE_TRANSACTION_VALIDITY_EPOCHS: u64 = 2160;

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::config";

fn return_default_transaction_request_ttl() -> Duration {
    Duration::from_secs(30 * 60)
}

fn return_default_signing_request_ttl() -> Duration {
    Duration::from_secs(24 * 60 * 60)
}

fn return_default_jwt_expiry() -> Duration {
    // Suggested expiry for access tokens is between 5min and 1h
    Duration::from_secs(5 * 60)
}

fn deserialize_indexer_urls<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Url>, D::Error> {
    deserialize_string_list(deserializer)?
        .iter()
        .map(|url| Url::parse(url).map_err(|e| de::Error::custom(format!("invalid indexer URL: {e}"))))
        .collect()
}

/// A list of non-empty, trimmed strings, given as a list or as one comma-separated string.
fn deserialize_string_list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        List(Vec<String>),
        CommaSeparated(String),
    }

    let items = match Raw::deserialize(deserializer)? {
        Raw::List(items) => items,
        Raw::CommaSeparated(items) => items.split(',').map(str::to_string).collect(),
    };
    Ok(items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect())
}

fn return_default_rpc_address() -> SocketAddr {
    SocketAddr::from(([127u8, 0, 0, 1], 5100))
}

impl Default for WalletDaemonConfig {
    fn default() -> Self {
        Self {
            override_from: None,
            network: Network::default(),
            authentication: WalletDaemonAuth::default(),
            json_rpc_address: return_default_rpc_address(),
            signaling_server_address: Some(SocketAddr::from(([127u8, 0, 0, 1], 9100))),
            indexer_api_urls: vec![
                "http://127.0.0.1:18300"
                    .parse()
                    .expect("failed to parse default indexer_api_url"),
            ],
            indexer_trust: IndexerTrust::default(),
            jwt_expiry: return_default_jwt_expiry(),
            transaction_request_ttl: return_default_transaction_request_ttl(),
            signing_request_ttl: return_default_signing_request_ttl(),
            enable_permissive_cors: false,
            allowed_hosts: Vec::new(),
            enable_webrtc: false,
            value_lookup_table_file: None,
            recovery_abandon_count: 10,
            webauthn: WebAuthnConfig {
                rp_origin: None,
                rp_id: "localhost".to_string(),
                session_ttl: Duration::from_secs(60 * 60),
            },
            burn_proof_dir: None,
            override_keyring_password: None,
            auto_claim_burns: true,
            default_transaction_validity_epochs: return_default_transaction_validity_epochs(),
            max_fee_limit: return_default_max_fee_limit(),
        }
    }
}

fn default_burn_proof_dir(network: Network) -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tari")
        .join(network.as_key_str())
        .join("burn_proofs")
}

impl SubConfigPath for WalletDaemonConfig {
    fn main_key_prefix() -> &'static str {
        "ootle_wallet_daemon"
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct WebAuthnConfig {
    /// if not set, rp_origin will be set to http://localhost:{web_ui_address.port()}
    pub rp_origin: Option<Url>,
    pub rp_id: String,
    #[serde(with = "humantime_serde")]
    pub session_ttl: Duration,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum WalletDaemonAuth {
    None,
    #[default]
    WebAuthn,
}

impl WalletDaemonAuth {
    pub fn is_none(self) -> bool {
        matches!(self, WalletDaemonAuth::None)
    }
}

impl Display for WalletDaemonAuth {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            WalletDaemonAuth::None => write!(f, "none"),
            WalletDaemonAuth::WebAuthn => write!(f, "webauthn"),
        }
    }
}

impl FromStr for WalletDaemonAuth {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(WalletDaemonAuth::None),
            "webauthn" => Ok(WalletDaemonAuth::WebAuthn),
            _ => Err(anyhow!("Invalid authentication method: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use tari_consensus::consensus_constants::ConsensusConstants;

    use super::*;

    /// The warning threshold is only useful while it tracks the rule it mirrors. If the consensus
    /// ceiling is lowered and this is not, the wallet keeps accepting a window under which every
    /// transaction it builds is unsequenceable.
    #[test]
    fn the_warning_threshold_tracks_the_consensus_ceiling() {
        assert_eq!(
            IMPLAUSIBLE_TRANSACTION_VALIDITY_EPOCHS,
            ConsensusConstants::mainnet().max_transaction_validity_epochs
        );
    }

    fn load(toml: &str) -> Result<WalletDaemonConfig, ConfigurationError> {
        let cfg = Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()
            .unwrap();
        WalletDaemonConfig::load_from(&cfg)
    }

    fn urls(urls: &[&str]) -> Vec<Url> {
        urls.iter().map(|url| Url::parse(url).unwrap()).collect()
    }

    #[test]
    fn a_single_indexer_url_still_loads() {
        let config = load("[ootle_wallet_daemon]\nindexer_api_url = \"http://a.example/\"").unwrap();
        assert_eq!(config.indexer_api_urls, urls(&["http://a.example/"]));
    }

    #[test]
    fn a_list_of_indexer_urls_loads() {
        let config =
            load("[ootle_wallet_daemon]\nindexer_api_url = [\"http://a.example/\", \"http://b.example/\"]").unwrap();
        assert_eq!(
            config.indexer_api_urls,
            urls(&["http://a.example/", "http://b.example/"])
        );
    }

    /// The command line passes its URLs as one comma-separated override.
    #[test]
    fn comma_separated_indexer_urls_load() {
        let cfg = Config::builder()
            .set_override(
                "ootle_wallet_daemon.indexer_api_url",
                "http://a.example/, http://b.example/",
            )
            .unwrap()
            .build()
            .unwrap();
        let config = WalletDaemonConfig::load_from(&cfg).unwrap();
        assert_eq!(
            config.indexer_api_urls,
            urls(&["http://a.example/", "http://b.example/"])
        );
    }

    #[test]
    fn esmeralda_defaults_to_both_tari_indexers() {
        let cfg = Config::builder()
            .add_source(config::File::from_str(
                tari_ootle_app_utilities::configuration::get_default_config()[4],
                config::FileFormat::Toml,
            ))
            .set_override("ootle_wallet_daemon.override_from", "esmeralda")
            .unwrap()
            .build()
            .unwrap();
        let config = WalletDaemonConfig::load_from(&cfg).unwrap();
        assert_eq!(
            config.indexer_api_urls,
            urls(&[
                "https://esme-indexer-a.ootle.tari.com/",
                "https://esme-indexer-b.ootle.tari.com/"
            ])
        );
    }

    #[test]
    fn an_empty_or_non_http_indexer_set_is_refused() {
        let empty = WalletDaemonConfig {
            indexer_api_urls: vec![],
            ..Default::default()
        };
        assert!(empty.validate().is_err());
        let non_http = WalletDaemonConfig {
            indexer_api_urls: urls(&["http://a.example/", "ftp://b.example/"]),
            ..Default::default()
        };
        assert!(non_http.validate().is_err());
    }

    #[test]
    fn a_zero_validity_window_is_refused() {
        let config = WalletDaemonConfig {
            default_transaction_validity_epochs: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_zero_max_fee_limit_is_refused() {
        let config = WalletDaemonConfig {
            max_fee_limit: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn max_fee_limit_loads_from_config() {
        let config =
            load("[ootle_wallet_daemon]\nindexer_api_url = \"http://a.example/\"\nmax_fee_limit = 5000000").unwrap();
        assert_eq!(config.max_fee_limit, 5_000_000);
        let config = load("[ootle_wallet_daemon]\nindexer_api_url = \"http://a.example/\"").unwrap();
        assert_eq!(config.max_fee_limit, return_default_max_fee_limit());
    }

    #[test]
    fn a_window_at_the_ceiling_is_accepted() {
        let config = WalletDaemonConfig {
            default_transaction_validity_epochs: IMPLAUSIBLE_TRANSACTION_VALIDITY_EPOCHS,
            ..Default::default()
        };
        config.validate().unwrap();
    }
}
