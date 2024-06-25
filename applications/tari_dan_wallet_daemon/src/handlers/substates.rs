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

    let result = sdk
        .get_network_interface()
        .list_substates(req.filter_by_template, req.filter_by_type, req.limit, req.offset)
        .await?;

    let substates = result
        .substates
        .into_iter()
        // TODO: should also add the "timestamp" and "type" fields from the indexer list items?
        .map(|s| WalletSubstateRecord {
            substate_id: s.substate_id,
            // TODO: should we remove the "parent_id" field from the wallet API? is it really needed somewhere?
            parent_id: None,
            version: s.version,
            template_address: s.template_address,
            module_name: s.module_name,
        })
        .collect();

    Ok(SubstatesListResponse { substates })
}
