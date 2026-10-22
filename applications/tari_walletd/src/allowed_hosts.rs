//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The host names a request may address the wallet daemon by.
//!
//! A browser enforces the same-origin policy by host *name*, not by the address the name resolves
//! to. A page whose name resolves to the daemon's address is same-origin with itself, so its
//! requests reach the daemon with no CORS check, carrying that foreign name in `Host`. Serving only
//! names the operator recognises keeps such pages out, whatever the authentication mode.

use std::net::IpAddr;

use axum::{
    extract::{Request, State},
    http::{StatusCode, header::HOST, uri::Authority},
    middleware::Next,
    response::{IntoResponse, Response},
};
use log::*;

use crate::config::WalletDaemonConfig;

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::allowed_hosts";

#[derive(Debug, Clone)]
pub struct AllowedHosts {
    any: bool,
    names: Vec<String>,
}

impl AllowedHosts {
    /// Allows the names listed in `allowed_hosts` (`*` allows every name) and the host of the
    /// WebAuthn relying-party origin, beside the names [`Self::permits`] always accepts.
    pub fn from_config(config: &WalletDaemonConfig) -> Self {
        let mut names = config
            .allowed_hosts
            .iter()
            .map(|host| host.trim().to_ascii_lowercase())
            .filter(|host| !host.is_empty())
            .collect::<Vec<_>>();
        let any = names.iter().any(|host| host == "*");
        if let Some(host) = config.webauthn.rp_origin.as_ref().and_then(|origin| origin.host_str()) {
            names.push(host.to_ascii_lowercase());
        }
        Self { any, names }
    }

    /// True if a request naming `host` (without a port) may be served.
    ///
    /// An IP literal is always accepted: it names no domain whose resolution someone else controls,
    /// so a page reaching the daemon by it is either served from that address or cross-origin and
    /// held to CORS. The same holds for `localhost` and its subdomains, which resolve to loopback by
    /// definition.
    pub fn permits(&self, host: &str) -> bool {
        if self.any {
            return true;
        }
        let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
        if host.parse::<IpAddr>().is_ok() {
            return true;
        }
        let host = host.to_ascii_lowercase();
        host == "localhost" || host.ends_with(".localhost") || self.names.contains(&host)
    }
}

/// Refuses a request that names a host [`AllowedHosts`] does not permit. The host comes from the
/// request URI's authority (HTTP/2, absolute-form) or else the `Host` header. A request carrying
/// neither is served: every browser request carries one.
pub async fn check_host(State(allowed): State<AllowedHosts>, request: Request, next: Next) -> Response {
    let authority = match request.uri().authority() {
        Some(authority) => Some(Ok(authority.clone())),
        None => request
            .headers()
            .get(HOST)
            .map(|value| Authority::try_from(value.as_bytes())),
    };
    match authority {
        None => next.run(request).await,
        Some(Ok(authority)) if allowed.permits(authority.host()) => next.run(request).await,
        Some(Ok(authority)) => {
            warn!(
                target: LOG_TARGET,
                "🚫 Refused a request addressed to '{}'. Add the name to ootle_wallet_daemon.allowed_hosts if this \
                 daemon is meant to be reached by it.",
                authority.host()
            );
            misdirected()
        },
        Some(Err(_)) => misdirected(),
    }
}

fn misdirected() -> Response {
    (
        StatusCode::MISDIRECTED_REQUEST,
        "This wallet daemon does not serve the requested host name",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;

    fn allowed(hosts: &[&str], rp_origin: Option<&str>) -> AllowedHosts {
        let mut config = WalletDaemonConfig::default();
        config.allowed_hosts = hosts.iter().map(|h| h.to_string()).collect();
        config.webauthn.rp_origin = rp_origin.map(|o| Url::parse(o).unwrap());
        AllowedHosts::from_config(&config)
    }

    #[test]
    fn local_names_and_ip_literals_are_permitted() {
        let allowed = allowed(&[], None);
        for host in [
            "localhost",
            "LocalHost",
            "wallet.localhost",
            "127.0.0.1",
            "192.168.1.20",
            "[::1]",
            "::1",
        ] {
            assert!(allowed.permits(host), "{host}");
        }
    }

    #[test]
    fn other_names_are_refused() {
        let allowed = allowed(&[], None);
        for host in [
            "wallet.example.com",
            "localhost.example.com",
            "127.0.0.1.example.com",
            "",
        ] {
            assert!(!allowed.permits(host), "{host}");
        }
    }

    #[test]
    fn configured_names_and_the_rp_origin_are_permitted() {
        let allowed = allowed(&["Wallet.Internal "], Some("https://wallet.example.com"));
        assert!(allowed.permits("wallet.internal"));
        assert!(allowed.permits("wallet.example.com"));
        assert!(!allowed.permits("other.example.com"));
    }

    #[test]
    fn a_wildcard_permits_every_name() {
        assert!(allowed(&["*"], None).permits("wallet.example.com"));
    }
}
