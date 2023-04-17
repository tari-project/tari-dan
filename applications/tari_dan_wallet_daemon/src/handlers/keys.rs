//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_crypto::keys::PublicKey as PublicKeyTrait;
use tari_dan_wallet_sdk::apis::key_manager;
use tari_wallet_daemon_client::types::{
    KeysCreateRequest, KeysCreateResponse, KeysListRequest, KeysListResponse, KeysSetActiveRequest,
    KeysSetActiveResponse,
};

use super::context::HandlerContext;

pub async fn handle_create(
    context: &HandlerContext,
    _value: KeysCreateRequest,
) -> Result<KeysCreateResponse, anyhow::Error> {
    let key = context
        .wallet_sdk()
        .key_manager_api()
        .next_key(key_manager::TRANSACTION_BRANCH)?;
    Ok(KeysCreateResponse {
        id: key.key_index,
        public_key: PublicKey::from_secret_key(&key.k),
    })
}

pub async fn handle_list(context: &HandlerContext, _value: KeysListRequest) -> Result<KeysListResponse, anyhow::Error> {
    let keys = context
        .wallet_sdk()
        .key_manager_api()
        .get_all_keys(key_manager::TRANSACTION_BRANCH)?;
    Ok(KeysListResponse { keys })
}

pub async fn handle_set_active(
    context: &HandlerContext,
    req: KeysSetActiveRequest,
) -> Result<KeysSetActiveResponse, anyhow::Error> {
    let km = context.wallet_sdk().key_manager_api();
    km.set_active_key(key_manager::TRANSACTION_BRANCH, req.index)?;
    let (_, key) = km.get_active_key(key_manager::TRANSACTION_BRANCH)?;

    Ok(KeysSetActiveResponse {
        public_key: PublicKey::from_secret_key(&key.k),
    })
}
