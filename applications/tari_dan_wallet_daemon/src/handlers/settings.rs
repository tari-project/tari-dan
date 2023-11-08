//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::{
    apis::{config::ConfigKey, jwt::JrpcPermission},
    network::WalletNetworkInterface,
};
use tari_wallet_daemon_client::types::{SettingsGetResponse, SettingsSetRequest, SettingsSetResponse};

use crate::handlers::HandlerContext;

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<String>,
    _value: serde_json::Value,
) -> Result<SettingsGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let indexer_url = if let Some(indexer_url) = sdk.config_api().get(ConfigKey::IndexerUrl).optional()? {
        indexer_url
    } else {
        sdk.get_config().indexer_jrpc_endpoint.clone()
    };

    Ok(SettingsGetResponse { indexer_url })
}

pub async fn handle_set(
    context: &HandlerContext,
    token: Option<String>,
    req: SettingsSetRequest,
) -> Result<SettingsSetResponse, anyhow::Error> {
    let mut sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    sdk.get_network_interface().set_endpoint(&req.indexer_url)?;
    sdk.config_api().set(ConfigKey::IndexerUrl, &req.indexer_url, false)?;
    Ok(SettingsSetResponse {})
}
