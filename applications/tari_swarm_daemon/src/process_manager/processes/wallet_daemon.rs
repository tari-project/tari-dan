//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_wallet_daemon_client::{
    types::{AccountGetResponse, AuthLoginAcceptRequest, AuthLoginRequest, AuthLoginResponse},
    WalletDaemonClient,
};

use crate::process_manager::Instance;

pub struct WalletDaemonProcess {
    instance: Instance,
}

impl WalletDaemonProcess {
    pub fn new(instance: Instance) -> Self {
        Self { instance }
    }

    async fn connect_client(&self) -> anyhow::Result<WalletDaemonClient> {
        let port = self
            .instance
            .allocated_ports()
            .get("jrpc")
            .ok_or_else(|| anyhow!("No wallet JSON-RPC port allocated"))?;
        let mut client = WalletDaemonClient::connect(format!("http://localhost:{port}"), None)?;
        let AuthLoginResponse { auth_token, .. } = client
            .auth_request(AuthLoginRequest {
                permissions: vec!["Admin".to_string()],
                duration: None,
            })
            .await
            .unwrap();
        let auth_response = client
            .auth_accept(AuthLoginAcceptRequest {
                auth_token,
                name: "Testing Token".to_string(),
            })
            .await
            .unwrap();
        client.set_auth_token(auth_response.permissions_token);

        Ok(client)
    }

    pub async fn get_account_public_key(&self, name: String) -> anyhow::Result<RistrettoPublicKey> {
        let mut client = self.connect_client().await?;
        let AccountGetResponse { public_key, .. } = client.accounts_get(name.into()).await?;
        Ok(public_key)
    }

    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }
}
