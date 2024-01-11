//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::apis::{config::ConfigKey, jwt::JrpcPermission};
use tari_wallet_daemon_client::types::{SettingsGetResponse, SettingsSetRequest, SettingsSetResponse};

use crate::handlers::HandlerContext;

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<String>,
    _value: serde_json::Value,
) -> Result<SettingsGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let indexer_url = sdk
        .config_api()
        .get(ConfigKey::IndexerUrl)
        .optional()?
        .unwrap_or_else(|| sdk.get_network_interface().get_endpoint().to_string());

    Ok(SettingsGetResponse { indexer_url })
}

pub async fn handle_set(
    context: &HandlerContext,
    token: Option<String>,
    req: SettingsSetRequest,
) -> Result<SettingsSetResponse, anyhow::Error> {
    let mut sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    sdk.get_network_interface_mut().set_endpoint(&req.indexer_url)?;
    sdk.config_api().set(ConfigKey::IndexerUrl, &req.indexer_url, false)?;
    Ok(SettingsSetResponse {})
}
