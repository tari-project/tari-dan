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

use std::{net::SocketAddr, path::PathBuf};

use clap::{Args, Parser};
use tari_common::configuration::{ConfigOverrideProvider, Network as L1Network};
use tari_common_types::seeds::seed_words::SeedWords;
use tari_crypto::tari_utilities::SafePassword;
use tari_ootle_address::Network;
use tari_ootle_app_utilities::common_cli_args::CommonCliArgs;
use url::Url;

use crate::config::WalletDaemonAuth;

#[derive(Args, Debug)]
pub struct WalletRestoreArgs {
    /// Seed words of a wallet to be restored.
    /// If set, wallet daemon tries to restore your wallet based on these seed words.
    #[clap(long)]
    pub seed_words: Option<SeedWords>,
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCliArgs,
    #[clap(long, alias = "listen-on", env = "JRPC_LISTEN_ON")]
    /// The address to listen on for Web and JSON-RPC requests. Defaults to localhost:5100.
    pub listen_on: Option<SocketAddr>,
    #[clap(long, env = "SIGNALING_SERVER_ADDRESS")]
    pub signaling_server_address: Option<SocketAddr>,
    /// Allow apps to connect to the wallet over WebRTC (`webrtc.start`).
    #[clap(long, env = "ENABLE_WEBRTC")]
    pub enable_webrtc: bool,
    #[clap(
        long = "indexer-api-url",
        short = 'i',
        alias = "indexer-url",
        value_delimiter = ',',
        env = "OOTLE_WALLETD_INDEXER_API_URL"
    )]
    /// Indexer API URL override. Repeat it, or separate URLs with commas, to configure several indexers.
    pub indexer_api_urls: Vec<Url>,
    /// A host name, beyond localhost, IP addresses and the WebAuthn origin's host, that requests may address the
    /// daemon by (e.g. the name a reverse proxy forwards). Repeat it, or separate names with commas. `*` accepts
    /// every name.
    #[clap(long = "allowed-host", value_delimiter = ',', env = "OOTLE_WALLETD_ALLOWED_HOSTS")]
    pub allowed_hosts: Vec<String>,
    #[clap(flatten)]
    pub wallet_restore: WalletRestoreArgs,
    /// The OS keyring is used to store and retrieve a randomly generated password. This is used for wallet encryption.
    /// This setting overrides this functionality, using this password instead of generating and storing one.
    /// This is useful if a keyring is not available on your platform or if there is some other preference to use a
    /// specific password.
    /// NOTE: Once this is set, it must always be set to access the wallet.
    #[clap(long, env = "OOTLE_WALLETD_KEYRING_PASSWORD", hide_env_values = true)]
    pub override_keyring_password: Option<SafePassword>,
    /// The authentication method to use for the wallet daemon. This controls how clients must authenticate when making
    /// requests to the wallet daemon.
    #[clap(long, alias = "auth", env = "OOTLE_WALLETD_AUTHENTICATION")]
    pub authentication: Option<WalletDaemonAuth>,
    /// Whether to watch the burn proof directory and claim new burns automatically (`true` or `false`).
    #[clap(long, env = "OOTLE_WALLETD_AUTO_CLAIM_BURNS")]
    pub auto_claim_burns: Option<bool>,
    /// The largest max_fee, in microtari, that this wallet submits a transaction with.
    #[clap(long, env = "OOTLE_WALLETD_MAX_FEE_LIMIT")]
    pub max_fee_limit: Option<u64>,
    /// Whether to write a local authentication cookie (`<base_path>/.cookie`) on startup (`true` or `false`).
    #[clap(long, env = "OOTLE_WALLETD_COOKIE_AUTH")]
    pub cookie_auth: Option<bool>,
    /// Permission scopes granted to local clients authenticating with `<base_path>/.cookie`. Repeat it, or separate
    /// scopes with commas. Defaults to `admin`.
    #[clap(
        long = "cookie-scopes",
        alias = "cookie-scope",
        value_delimiter = ',',
        env = "OOTLE_WALLETD_COOKIE_SCOPES"
    )]
    pub cookie_scopes: Vec<String>,
    /// The path to the value lookup table binary file used for brute force value lookups. This setting
    /// is only used when attempting to view confidential balances in confidential resources that use a view key
    /// controlled by this wallet. The binary file can be generated using the generate_ristretto_value_lookup
    /// utility. If this is not set, the value lookup table will be generated on the fly which will have a large
    /// performance cost when brute forcing high-value outputs.
    #[clap(long, alias = "lookup-file")]
    pub value_lookup_table_file: Option<PathBuf>,
    /// If set, the wallet daemon will enable permissive CORS, and set webauthn.rp_origin to http://localhost:{enable_vite_dev_port}
    /// to allow authentication from a local development server (e.g. npm run dev).
    /// This is useful for development and testing, but should not be used in production.
    #[clap(long, alias = "vite-dev")]
    pub enable_vite_dev_port: Option<u16>,
    #[clap(subcommand)]
    pub command: Option<Subcommand>,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }

    pub fn network(&self) -> Network {
        self.common.network
    }
}

impl ConfigOverrideProvider for Cli {
    fn get_config_property_overrides(&self, network: &L1Network) -> Vec<(String, String)> {
        let mut overrides = self.common.get_config_property_overrides(network);
        overrides.push(("ootle_wallet_daemon.override_from".to_string(), network.to_string()));
        if let Some(json_rpc_address) = self.listen_on {
            overrides.push((
                format!("{}.ootle_wallet_daemon.json_rpc_address", network),
                json_rpc_address.to_string(),
            ));
        }
        if let Some(ref signaling_server_address) = self.signaling_server_address {
            overrides.push((
                format!("{}.ootle_wallet_daemon.signaling_server_address", network),
                signaling_server_address.to_string(),
            ));
        }
        if !self.indexer_api_urls.is_empty() {
            let urls = self
                .indexer_api_urls
                .iter()
                .map(Url::as_str)
                .collect::<Vec<_>>()
                .join(",");
            overrides.push((format!("{}.ootle_wallet_daemon.indexer_api_url", network), urls));
        }
        if !self.allowed_hosts.is_empty() {
            overrides.push((
                format!("{}.ootle_wallet_daemon.allowed_hosts", network),
                self.allowed_hosts.join(","),
            ));
        }
        if let Some(ref file) = self.value_lookup_table_file {
            overrides.push((
                format!("{}.ootle_wallet_daemon.value_lookup_table_file", network),
                file.display().to_string(),
            ));
        }
        if let Some(ref auth) = self.authentication {
            overrides.push((
                format!("{}.ootle_wallet_daemon.authentication", network),
                auth.to_string(),
            ));
        }
        if self.enable_webrtc {
            overrides.push((
                format!("{}.ootle_wallet_daemon.enable_webrtc", network),
                "true".to_string(),
            ));
        }
        if let Some(auto_claim_burns) = self.auto_claim_burns {
            overrides.push((
                format!("{}.ootle_wallet_daemon.auto_claim_burns", network),
                auto_claim_burns.to_string(),
            ));
        }
        if let Some(max_fee_limit) = self.max_fee_limit {
            overrides.push((
                format!("{}.ootle_wallet_daemon.max_fee_limit", network),
                max_fee_limit.to_string(),
            ));
        }
        if let Some(cookie_auth) = self.cookie_auth {
            overrides.push((
                format!("{}.ootle_wallet_daemon.cookie_auth", network),
                cookie_auth.to_string(),
            ));
        }
        if !self.cookie_scopes.is_empty() {
            overrides.push((
                format!("{}.ootle_wallet_daemon.cookie_scopes", network),
                self.cookie_scopes.join(","),
            ));
        }
        if let Some(port) = self.enable_vite_dev_port {
            overrides.extend([
                (
                    format!("{}.ootle_wallet_daemon.enable_permissive_cors", network),
                    "true".to_string(),
                ),
                (
                    format!("{}.ootle_wallet_daemon.webauthn.rp_origin", network),
                    format!("http://localhost:{port}"),
                ),
            ]);
        }
        overrides
    }
}

#[derive(clap::Subcommand, Debug)]
pub enum Subcommand {
    #[clap(name = "run", about = "Run the wallet daemon")]
    Run,
    #[clap(about = "Generate a new key and output the public key")]
    CreateAccount {
        #[clap(long)]
        name: Option<String>,
        #[clap(long, alias = "key")]
        key_index: Option<u64>,
        #[clap(long)]
        set_active: bool,
        #[clap(long, alias = "output", short = 'o')]
        output_path: Option<PathBuf>,
    },
    #[clap(about = "Generate a key to use for resources with viewable balances")]
    NewViewableBalanceKey {
        #[clap(long, alias = "key")]
        key_index: u64,
        #[clap(long, alias = "output", short = 'o')]
        output_path: Option<PathBuf>,
    },
    #[clap(
        name = "seed-words",
        about = "Get current seed words of wallet (used for wallet retrieval)"
    )]
    SeedWords,
    #[clap(
        name = "reset",
        about = "Reset the wallet by deleting all on-chain state. Intended for testnet resets only. The seed key is \
                 preserved in the OS keyring."
    )]
    Reset {
        /// Skip the interactive confirmation prompt
        #[clap(long)]
        confirm: bool,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use tari_common::DefaultConfigLoader;
    use tari_ootle_app_utilities::configuration::{
        get_default_config,
        load_configuration_with_overrides,
        write_config_to,
    };

    use super::*;
    use crate::config::WalletDaemonConfig;

    /// Loads the wallet section the way the daemon does, from the default config file it writes on first start.
    fn load(cli: &Cli) -> WalletDaemonConfig {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_config_to(&path, &get_default_config()).unwrap();
        let cfg = load_configuration_with_overrides(&path, cli, Some(cli.network())).unwrap();
        WalletDaemonConfig::load_from(&cfg).unwrap()
    }

    fn urls(urls: &[&str]) -> Vec<Url> {
        urls.iter().map(|url| Url::parse(url).unwrap()).collect()
    }

    #[test]
    fn command_line_flags_override_the_esmeralda_preset() {
        let cli = Cli::try_parse_from([
            "tari_ootle_walletd",
            "--network",
            "esmeralda",
            "--listen-on",
            "0.0.0.0:5100",
            "--indexer-api-url",
            "http://a.example/,http://b.example/",
            "--authentication",
            "none",
            "--auto-claim-burns",
            "false",
            "--max-fee-limit",
            "5000000",
            "--cookie-auth",
            "false",
            "--cookie-scopes",
            "account_info,transactions:read",
            "--override-keyring-password",
            "secret",
        ])
        .unwrap();
        assert!(cli.override_keyring_password.is_some());

        let config = load(&cli);
        assert_eq!(config.json_rpc_address, "0.0.0.0:5100".parse::<SocketAddr>().unwrap());
        assert_eq!(
            config.indexer_api_urls,
            urls(&["http://a.example/", "http://b.example/"])
        );
        assert!(config.authentication.is_none());
        assert!(!config.auto_claim_burns);
        assert_eq!(config.max_fee_limit, 5_000_000);
        assert!(!config.cookie_auth);
        assert_eq!(config.cookie_scopes, vec![
            "account_info".to_string(),
            "transactions:read".to_string()
        ]);
    }

    /// Every setting the container image documents must be settable from the environment.
    #[test]
    fn container_settings_have_environment_variables() {
        let command = Cli::command();
        let env_of = |long: &str| {
            command
                .get_arguments()
                .find(|arg| arg.get_long() == Some(long))
                .and_then(|arg| arg.get_env())
                .and_then(|env| env.to_str())
                .map(str::to_string)
        };
        let expected = [
            ("network", "TARI_NETWORK"),
            ("base-path", "TARI_BASE_DIR"),
            ("listen-on", "JRPC_LISTEN_ON"),
            ("indexer-api-url", "OOTLE_WALLETD_INDEXER_API_URL"),
            ("override-keyring-password", "OOTLE_WALLETD_KEYRING_PASSWORD"),
            ("authentication", "OOTLE_WALLETD_AUTHENTICATION"),
            ("auto-claim-burns", "OOTLE_WALLETD_AUTO_CLAIM_BURNS"),
            ("max-fee-limit", "OOTLE_WALLETD_MAX_FEE_LIMIT"),
            ("cookie-auth", "OOTLE_WALLETD_COOKIE_AUTH"),
            ("cookie-scopes", "OOTLE_WALLETD_COOKIE_SCOPES"),
            ("allowed-host", "OOTLE_WALLETD_ALLOWED_HOSTS"),
        ];
        for (long, env) in expected {
            assert_eq!(env_of(long).as_deref(), Some(env), "--{long}");
        }
    }
}
