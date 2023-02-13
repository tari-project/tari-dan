//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_wallet_daemon_client::types::{KeysCreateRequest, KeysCreateResponse};

use super::context::HandlerContext;

pub async fn handle_create(
    context: &HandlerContext,
    _value: KeysCreateRequest,
) -> Result<KeysCreateResponse, anyhow::Error> {
    let key = context.wallet_sdk().key_manager_api().next_key("wallet")?;
    Ok(KeysCreateResponse { id: key.key_index })
}
