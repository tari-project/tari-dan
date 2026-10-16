//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{Extension, Json, extract::Query, response::Response};
use tari_indexer_client::types::{GetCommitteeRequest, GetCommitteeResponse};
use tari_ootle_common_types::{Epoch, ValidatorSetHash, shard::Shard};

use crate::rest_api::{
    context::HandlerContext,
    error::ErrorResponse,
    handlers::HandlerResult,
    proofs::{committee_for_shard, committee_proved_against, num_preshards},
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
    let certified_hash = certified_validator_set_hash(&context, req.epoch, req.shard).await?;
    let committee = match &certified_hash {
        Some(hash) => {
            committee_proved_against(
                context.committee_proofs(),
                context.epoch_manager(),
                req.epoch,
                hash,
                req.shard,
            )
            .await?
        },
        None => {
            let num_preshards = num_preshards(context.epoch_manager()).await?;
            committee_for_shard(context.epoch_manager(), num_preshards, req.epoch, req.shard).await?
        },
    };
    let Some(committee) = committee else {
        return Err(ErrorResponse::not_found(format!(
            "This indexer holds no validator set for {}",
            req.epoch
        )));
    };
    let is_certified = certified_hash == Some(committee.validator_set_hash);
    let body = Json(GetCommitteeResponse { committee });
    if is_certified {
        Ok(context.apply_immutable_cache_control(body))
    } else {
        Ok(context.apply_cache_control(body, 30))
    }
}

/// The validator set hash of `epoch` that the stored checkpoint of the epoch before it names. Every shard group's
/// checkpoint names the same hash, so `shard` only selects which one to read.
async fn certified_validator_set_hash(
    context: &HandlerContext,
    epoch: Epoch,
    shard: Shard,
) -> Result<Option<ValidatorSetHash>, ErrorResponse> {
    let Some(prev_epoch) = epoch.as_u64().checked_sub(1).map(Epoch) else {
        return Ok(None);
    };
    let checkpoints = context
        .read_only_store()
        .epoch_checkpoint_get_all(prev_epoch, Some(shard), 1)
        .await
        .map_err(ErrorResponse::anyhow)?;
    Ok(checkpoints
        .first()
        .filter(|checkpoint| checkpoint.epoch() == prev_epoch)
        .and_then(|checkpoint| checkpoint.next_epoch_validator_set_hash()))
}
