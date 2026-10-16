//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum_extra::headers::authorization::Bearer;
use tari_engine_types::substate::Substate;
use tari_ootle_common_types::optional::Optional;
use tari_ootle_walletd_client::{
    permissions::{Permission, ReadOnly},
    types::{
        SubstatesGetRequest,
        SubstatesGetResponse,
        SubstatesListRequest,
        SubstatesListResponse,
        WalletSubstateInfo,
    },
};

use crate::handlers::{HandlerContext, helpers::not_found};

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: SubstatesGetRequest,
) -> Result<SubstatesGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    context.authorize(token, &[Permission::Substates(ReadOnly::Read)])?;

    let record = sdk.substate_api().get_substate(&req.substate_id).optional()?;

    let substate = sdk
        .substate_api()
        .fetch_substate_from_network(&req.substate_id, record.as_ref().map(|r| r.substate_id.version()))
        .await
        .optional()?;

    if record.is_none() && substate.is_none() {
        return Err(not_found(format!("Substate with ID {} not found", req.substate_id)));
    }

    Ok(SubstatesGetResponse {
        local_record: record.map(|record| WalletSubstateInfo {
            version: record.substate_id.version(),
            substate_id: record.substate_id.into_substate_id(),
            parent_id: record.parent_address,
            module_name: record.module_name,
            template_address: record.template_address,
        }),
        substate_from_remote: substate.map(|s| Substate::new(s.id.version(), s.substate)),
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: SubstatesListRequest,
) -> Result<SubstatesListResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    context.authorize(token, &[Permission::Substates(ReadOnly::Read)])?;
    let substates = sdk.substate_api().list_substates(
        req.filter_by_type,
        req.filter_by_template.as_ref(),
        req.limit,
        req.offset,
    )?;

    let substates = substates
        .into_iter()
        .map(|s| WalletSubstateInfo {
            version: s.substate_id.version(),
            substate_id: s.substate_id.into_substate_id(),
            parent_id: s.parent_address,
            template_address: s.template_address,
            module_name: s.module_name,
        })
        .collect();

    Ok(SubstatesListResponse { substates })
}
