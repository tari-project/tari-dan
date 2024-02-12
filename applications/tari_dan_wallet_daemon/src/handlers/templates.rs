//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_sdk::{apis::jwt::JrpcPermission, network::WalletNetworkInterface};
use tari_wallet_daemon_client::types::{TemplatesGetRequest, TemplatesGetResponse};

use crate::handlers::HandlerContext;

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<String>,
    req: TemplatesGetRequest,
) -> Result<TemplatesGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::TemplatesRead])?;

    let template_definition = sdk
        .get_network_interface()
        .fetch_template_definition(req.template_address)
        .await?;

    Ok(TemplatesGetResponse { template_definition })
}
