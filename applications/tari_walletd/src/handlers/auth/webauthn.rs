// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Debug, sync::Arc};

use anyhow::anyhow;
use tari_ootle_wallet_sdk::storage::WalletStore;
use tari_ootle_walletd_client::types::AuthCredentials;
use webauthn_rs::Webauthn;

use crate::{handlers::auth::Authenticator, services::WebauthnService};

#[derive(Debug, Clone)]
pub struct WebAuthnAuth<TStore> {
    webauthn: Arc<Webauthn>,
    webauthn_service: WebauthnService<TStore>,
}

impl<TStore: WalletStore + Debug + Send + Sync> WebAuthnAuth<TStore> {
    pub fn new(webauthn: Webauthn, webauthn_service: WebauthnService<TStore>) -> Self {
        WebAuthnAuth {
            webauthn: Arc::new(webauthn),
            webauthn_service,
        }
    }

    pub fn webauthn(&self) -> &Webauthn {
        &self.webauthn
    }

    pub fn webauthn_service(&self) -> &WebauthnService<TStore> {
        &self.webauthn_service
    }
}

impl<TStore: WalletStore + Debug + Send + Sync> Authenticator for WebAuthnAuth<TStore> {
    async fn authenticate(&self, credentials: &AuthCredentials) -> Result<(), anyhow::Error> {
        match credentials.as_webauthn() {
            Some(req) => {
                let credential = &req.credential;
                let auth_passkey = self.webauthn_service.auth_passkey(req.session_id.as_str()).await?;
                let result = self
                    .webauthn
                    .finish_passkey_authentication(credential, &auth_passkey)
                    .map_err(|error| anyhow!("Authentication failed: {error:?}"))?;
                self.webauthn_service.record_authentication(&result)?;
                self.webauthn_service
                    .finish_authentication(req.session_id.as_str())
                    .await?;
                Ok(())
            },
            None => Err(anyhow!("No webauthn finish login request present in request!")),
        }
    }
}
