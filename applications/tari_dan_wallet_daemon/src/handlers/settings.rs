//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::{
    apis::{config::ConfigKey, jwt::JrpcPermission},
    network::WalletNetworkInterface,
};
use tari_wallet_daemon_client::types::{
    SettingsGetIndexerUrlResponse,
    SettingsSetIndexerUrlRequest,
    SettingsSetIndexerUrlResponse,
};

use crate::handlers::HandlerContext;

pub async fn handle_get_indexer_url(
    context: &HandlerContext,
    token: Option<String>,
    _value: serde_json::Value,
) -> Result<SettingsGetIndexerUrlResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let indexer_url = if let Some(indexer_url) = sdk.config_api().get(ConfigKey::IndexerUrl).optional()? {
        indexer_url
    } else {
        sdk.get_config().indexer_jrpc_endpoint.clone()
    };

    Ok(SettingsGetIndexerUrlResponse { indexer_url })
}

pub async fn handle_set_indexer_url(
    context: &HandlerContext,
    token: Option<String>,
    req: SettingsSetIndexerUrlRequest,
) -> Result<SettingsSetIndexerUrlResponse, anyhow::Error> {
    let mut sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    sdk.get_network_interface().set_endpoint(&req.indexer_url)?;
    sdk.config_api().set(ConfigKey::IndexerUrl, &req.indexer_url, false)?;
    Ok(SettingsSetIndexerUrlResponse {})
}
