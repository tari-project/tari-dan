//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_sdk::{apis::jwt::JrpcPermission, network::WalletNetworkInterface};
use tari_wallet_daemon_client::types::{
    SubstatesGetRequest,
    SubstatesGetResponse,
    SubstatesListRequest,
    SubstatesListResponse,
    WalletSubstateRecord,
};

use crate::handlers::HandlerContext;

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<String>,
    req: SubstatesGetRequest,
) -> Result<SubstatesGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::SubstatesRead])?;

    let record = sdk.substate_api().get_substate(&req.substate_id)?;

    let substate = sdk
        .get_network_interface()
        .query_substate(&record.address.substate_id, Some(record.address.version), false)
        .await?;

    Ok(SubstatesGetResponse {
        record: WalletSubstateRecord {
            substate_id: record.address.substate_id,
            parent_id: record.parent_address,
            module_name: record.module_name,
            version: record.address.version,
            template_address: record.template_address,
        },
        value: substate.substate,
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<String>,
    req: SubstatesListRequest,
) -> Result<SubstatesListResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::SubstatesRead])?;

    // TODO: pagination
    let substates =
        sdk.substate_api()
            .list_substates(req.filter_by_type, req.filter_by_template.as_ref(), None, None)?;

    let substates = substates
        .into_iter()
        .map(|substate| WalletSubstateRecord {
            substate_id: substate.address.substate_id,
            parent_id: substate.parent_address,
            version: substate.address.version,
            template_address: substate.template_address,
            module_name: substate.module_name,
        })
        .collect();

    Ok(SubstatesListResponse { substates })
}
