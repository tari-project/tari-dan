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

use std::path::PathBuf;

use clap::Parser;
use multiaddr::{Multiaddr, Protocol};
use tari_ootle_transaction::Network;
use tari_ootle_walletd_client::{
    error::WalletDaemonClientError,
    read_cookie_file,
    try_read_cookie_from_base_path,
    types::EncodedJwtString,
};

use crate::command::Command;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(long, short = 'd', alias = "endpoint", env = "JRPC_ENDPOINT")]
    pub daemon_jrpc_endpoint: Option<Multiaddr>,
    #[clap(long, env = "JSON_WEB_TOKEN")]
    pub token: Option<String>,
    /// Path to the wallet daemon `.cookie` file used for local authentication when `--token` is not supplied
    #[clap(long, alias = "cookie_file", env = "OOTLE_WALLETD_COOKIE_FILE")]
    pub cookie_file: Option<PathBuf>,
    /// Base directory of the wallet daemon (used to locate `.cookie` automatically when `--token` is not supplied)
    #[clap(
        short = 'b',
        long,
        aliases = &["base_path", "base_dir", "base-dir"],
        default_value_os_t = default_base_path(),
        env = "TARI_BASE_DIR"
    )]
    pub base_path: PathBuf,
    /// Network subdirectory under `--base-path` used by the wallet daemon
    #[clap(long, env = "TARI_NETWORK", default_value_t)]
    pub network: Network,
    #[clap(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }

    /// Resolves the bearer token to use for authenticating with the wallet daemon:
    /// 1. Explicit `--token` / `JSON_WEB_TOKEN`
    /// 2. Explicit `--cookie-file` / `OOTLE_WALLETD_COOKIE_FILE`
    /// 3. Automatic discovery of `<base_path>/.cookie` or `<base_path>/<network>/.cookie` (loopback endpoints only)
    pub fn resolve_auth_token(&self) -> Result<Option<EncodedJwtString>, WalletDaemonClientError> {
        if let Some(ref token) = self.token {
            return Ok(Some(EncodedJwtString::from(token.clone())));
        }

        if let Some(ref cookie_file) = self.cookie_file {
            return read_cookie_file(cookie_file).map(Some);
        }

        if !self.is_loopback_endpoint() {
            return Ok(None);
        }

        if let Some(token) = try_read_cookie_from_base_path(&self.base_path)? {
            return Ok(Some(token));
        }

        let network_base_path = self.base_path.join(self.network.to_string());
        try_read_cookie_from_base_path(network_base_path)
    }

    fn is_loopback_endpoint(&self) -> bool {
        let Some(ref endpoint) = self.daemon_jrpc_endpoint else {
            return true;
        };
        match endpoint.iter().next() {
            Some(Protocol::Ip4(ip)) => ip.is_loopback(),
            Some(Protocol::Ip6(ip)) => ip.is_loopback(),
            Some(Protocol::Dns(host) | Protocol::Dns4(host) | Protocol::Dns6(host) | Protocol::Dnsaddr(host)) => {
                host.eq_ignore_ascii_case("localhost")
            },
            _ => false,
        }
    }
}

fn default_base_path() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".tari")
        .join("ootle")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use tari_ootle_walletd_client::cookie_file_path;

    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tari_wallet_cli_{label}_{}_{}", std::process::id(), nanos));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn resolve_auth_token_prefers_explicit_token_over_cookie_file() {
        let dir = unique_temp_dir("explicit_token");
        fs::write(
            cookie_file_path(&dir),
            "tc_cookie_token_that_should_not_be_used_1234567",
        )
        .unwrap();

        let cli = Cli::try_parse_from([
            "tari_ootle_wallet_cli",
            "--token",
            "tw_explicit_api_key",
            "--base-path",
            dir.to_str().unwrap(),
            "keys",
            "list",
        ])
        .unwrap();

        let token = cli.resolve_auth_token().unwrap().unwrap();
        assert_eq!(token.as_str(), "tw_explicit_api_key");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_auth_token_discovers_cookie_in_base_path_or_network_subdir() {
        let dir = unique_temp_dir("auto_cookie");
        let net_dir = dir.join(Network::default().to_string());
        fs::create_dir_all(&net_dir).unwrap();
        fs::write(
            cookie_file_path(&net_dir),
            "tc_network_subdir_cookie_token_0123456789abcdef\n",
        )
        .unwrap();

        let cli = Cli::try_parse_from([
            "tari_ootle_wallet_cli",
            "--base-path",
            dir.to_str().unwrap(),
            "keys",
            "list",
        ])
        .unwrap();

        let token = cli.resolve_auth_token().unwrap().unwrap();
        assert_eq!(token.as_str(), "tc_network_subdir_cookie_token_0123456789abcdef");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_auth_token_reads_explicit_cookie_file() {
        let dir = unique_temp_dir("custom_cookie_file");
        let custom_cookie = dir.join("custom.cookie");
        fs::write(&custom_cookie, "tc_custom_cookie_value_0123456789abcdef012345").unwrap();

        let cli = Cli::try_parse_from([
            "tari_ootle_wallet_cli",
            "--cookie-file",
            custom_cookie.to_str().unwrap(),
            "keys",
            "list",
        ])
        .unwrap();

        let token = cli.resolve_auth_token().unwrap().unwrap();
        assert_eq!(token.as_str(), "tc_custom_cookie_value_0123456789abcdef012345");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_auth_token_skips_auto_discovery_for_non_loopback_endpoints() {
        let dir = unique_temp_dir("loopback_filter");
        fs::write(cookie_file_path(&dir), "tc_local_cookie_token_0123456789abcdef012345").unwrap();

        for loopback in [
            "/ip4/127.0.0.1/tcp/5100",
            "/ip6/::1/tcp/5100",
            "/dns4/localhost/tcp/5100",
        ] {
            let cli = Cli::try_parse_from([
                "tari_ootle_wallet_cli",
                "--daemon-jrpc-endpoint",
                loopback,
                "--base-path",
                dir.to_str().unwrap(),
                "keys",
                "list",
            ])
            .unwrap();
            let token = cli.resolve_auth_token().unwrap().expect("loopback discovers cookie");
            assert_eq!(token.as_str(), "tc_local_cookie_token_0123456789abcdef012345");
        }

        for remote in [
            "/ip4/192.0.2.10/tcp/5100",
            "/ip6/2001:db8::1/tcp/5100",
            "/dns4/wallet.example.com/tcp/5100",
        ] {
            let cli = Cli::try_parse_from([
                "tari_ootle_wallet_cli",
                "--daemon-jrpc-endpoint",
                remote,
                "--base-path",
                dir.to_str().unwrap(),
                "keys",
                "list",
            ])
            .unwrap();
            assert!(
                cli.resolve_auth_token().unwrap().is_none(),
                "remote endpoint {remote} must not auto-discover local cookie"
            );
        }

        // Explicit `--cookie-file` remains an opt-in even for a remote endpoint.
        let explicit_cli = Cli::try_parse_from([
            "tari_ootle_wallet_cli",
            "--daemon-jrpc-endpoint",
            "/dns4/wallet.example.com/tcp/5100",
            "--cookie-file",
            cookie_file_path(&dir).to_str().unwrap(),
            "keys",
            "list",
        ])
        .unwrap();
        let token = explicit_cli.resolve_auth_token().unwrap().unwrap();
        assert_eq!(token.as_str(), "tc_local_cookie_token_0123456789abcdef012345");

        let _ = fs::remove_dir_all(&dir);
    }
}
