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

use std::time::Duration;

use clap::{Args, Subcommand};
use tari_dan_wallet_sdk::apis::jwt::JrpcPermissions;
use tari_wallet_daemon_client::{
    types::{AuthLoginAcceptRequest, AuthLoginDenyRequest, AuthLoginRequest},
    WalletDaemonClient,
};

#[derive(Debug, Subcommand, Clone)]
pub enum AuthSubcommand {
    #[clap(alias = "request")]
    Request(RequestArgs),
    #[clap(alias = "grant")]
    Grant(GrantArgs),
    #[clap(alias = "deny")]
    Deny(DenyArgs),
}

// TODO: Add permissions
#[derive(Debug, Args, Clone)]
pub struct RequestArgs {
    permissions: JrpcPermissions,
    validity_in_seconds: Option<u64>,
}

// TODO: We have to implement some wallet password for granting access. Only granting and denying access will need
// password, everything else is based on the JWT tokens. Currently you can just call "auth.request" and then grant
// yourself access by calling "auth.accept".
#[derive(Debug, Args, Clone)]
pub struct GrantArgs {
    auth_token: String,
}

#[derive(Debug, Args, Clone)]
pub struct DenyArgs {
    auth_token: String,
}

impl AuthSubcommand {
    pub async fn handle(self, mut client: WalletDaemonClient) -> anyhow::Result<()> {
        #[allow(clippy::enum_glob_use)]
        use AuthSubcommand::*;
        match self {
            Request(args) => {
                if args.permissions.no_permissions() {
                    println!("You forgot add permissions");
                } else {
                    let resp = client
                        .auth_request(AuthLoginRequest {
                            permissions: args.permissions,
                            duration: args.validity_in_seconds.map(|value| Duration::from_secs(value)),
                        })
                        .await?;
                    println!("Auth token {}", resp.auth_token);
                }
            },
            Grant(args) => {
                let resp = client
                    .auth_accept(AuthLoginAcceptRequest {
                        auth_token: args.auth_token,
                    })
                    .await?;
                println!("Access granted. Your JRPC token : {}", resp.permissions_token);
            },
            Deny(args) => {
                client
                    .auth_deny(AuthLoginDenyRequest {
                        auth_token: args.auth_token,
                    })
                    .await?;
                println!("Access denied!");
            },
        }
        Ok(())
    }
}
