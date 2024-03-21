//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_crypto::keys::PublicKey as PublicKeyTrait;
use tari_dan_wallet_sdk::apis::{jwt::JrpcPermission, key_manager};
use tari_wallet_daemon_client::types::{
    KeysCreateRequest,
    KeysCreateResponse,
    KeysListRequest,
    KeysListResponse,
    KeysSetActiveRequest,
    KeysSetActiveResponse,
};

use super::context::HandlerContext;

pub async fn handle_create(
    context: &HandlerContext,
    token: Option<String>,
    req: KeysCreateRequest,
) -> Result<KeysCreateResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let key_manager = sdk.key_manager_api();
    let key = req
        .specific_index
        .map(|idx| key_manager.derive_key(req.branch.as_str(), idx))
        .unwrap_or_else(|| key_manager.next_key(req.branch.as_str()))?;
    Ok(KeysCreateResponse {
        id: key.key_index,
        public_key: PublicKey::from_secret_key(&key.key),
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<String>,
    req: KeysListRequest,
) -> Result<KeysListResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::KeyList])?;
    let keys = sdk.key_manager_api().get_all_keys(req.branch.as_str())?;
    Ok(KeysListResponse { keys })
}

pub async fn handle_set_active(
    context: &HandlerContext,
    token: Option<String>,
    req: KeysSetActiveRequest,
) -> Result<KeysSetActiveResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let km = sdk.key_manager_api();
    km.set_active_key(key_manager::TRANSACTION_BRANCH, req.index)?;
    let (_, key) = km.get_active_key(key_manager::TRANSACTION_BRANCH)?;

    Ok(KeysSetActiveResponse {
        public_key: PublicKey::from_secret_key(&key.key),
    })
}
