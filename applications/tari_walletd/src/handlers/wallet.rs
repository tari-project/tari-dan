// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use axum_extra::headers::authorization::Bearer;
use tari_ootle_walletd_client::types::{WalletGetInfoRequest, WalletGetInfoResponse};

use crate::handlers::HandlerContext;

const APP_VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");

pub async fn handle_get_info(
    context: &HandlerContext,
    _token: Option<&Bearer>,
    _req: WalletGetInfoRequest,
) -> Result<WalletGetInfoResponse, anyhow::Error> {
    let network = context.config().network;

    Ok(WalletGetInfoResponse {
        version: APP_VERSION.unwrap_or("unknown").to_string(),
        network: network.to_string(),
        network_byte: network.as_byte(),
    })
}
