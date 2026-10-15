//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{Extension, Json, extract::Query, response::Response};
use tari_indexer_client::types::{GetCommitteeRequest, GetCommitteeResponse};

use crate::rest_api::{
    context::HandlerContext,
    error::ErrorResponse,
    handlers::HandlerResult,
    proofs::committee_for_shard,
};

#[utoipa::path(
    get,
    path = "/committees",
    description = "Get one shard group's committee in an epoch, with its proof against the epoch's validator set hash",
    params(
        ("epoch" = u64, Query, description = "The epoch of the committee"),
        ("shard" = u32, Query, description = "Any shard of the shard group whose committee to return"),
    ),
    responses(
        (status = 200, description = "The committee and its proof", body = GetCommitteeResponse),
        (status = BAD_REQUEST, description = "The epoch has not occurred yet, or the shard is in no shard group", body = ErrorResponse),
        (status = NOT_FOUND, description = "This indexer holds no validator set for the epoch", body = ErrorResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Internal error", body = ErrorResponse),
    )
)]
pub async fn get_committee(
    Extension(context): Extension<HandlerContext>,
    Query(req): Query<GetCommitteeRequest>,
) -> HandlerResult<Response> {
    let current_epoch = context.epoch_manager().get_current_epoch();
    if req.epoch > current_epoch {
        return Err(ErrorResponse::bad_request(format!(
            "Epoch {} has not occurred yet (current epoch is {current_epoch})",
            req.epoch
        )));
    }
    let Some(committee) = committee_for_shard(context.epoch_manager(), req.epoch, req.shard).await? else {
        return Err(ErrorResponse::not_found(format!(
            "This indexer holds no validator set for {}",
            req.epoch
        )));
    };
    Ok(context.apply_cache_control(Json(GetCommitteeResponse { committee }), 30))
}
