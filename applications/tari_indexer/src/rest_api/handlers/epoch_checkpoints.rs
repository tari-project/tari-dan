//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{Extension, Json};
use tari_indexer_client::types::{
    EpochCheckpointEntry,
    GetLatestEpochCheckpointRequest,
    GetLatestEpochCheckpointResponse,
    ListEpochCheckpointsRequest,
    ListEpochCheckpointsResponse,
};
use tari_ootle_common_types::{Epoch, shard::Shard};
use tari_ootle_storage::consensus_models::EpochCheckpoint;

use crate::rest_api::{
    context::HandlerContext,
    error::ErrorResponse,
    handlers::HandlerResult,
    proofs::{committee_for_shard, to_checkpoint_entry},
};

const DEFAULT_LIMIT: u32 = 20;
/// A checkpoint carries its commit proof and the shard tree summary of its shard group, several KB of CBOR, so the
/// page size bounds the response size. A wallet's catch-up, at most about 500 checkpoints, takes five pages.
const MAX_LIMIT: u32 = 100;

#[utoipa::path(
    get,
    path = "/epoch-checkpoints",
    description = "List epoch checkpoints from storage, ascending by epoch",
    params(
        ("from_epoch" = Option<u64>, Query, description = "Epoch to start listing from (inclusive). Defaults to 0"),
        ("limit" = Option<u32>, Query, description = "Maximum number of checkpoints to return (default: 20, max: 100)"),
        ("shard" = Option<u32>, Query, description = "Only the checkpoints of the shard group containing this shard"),
        (
            "include_committees" = Option<bool>,
            Query,
            description = "Include the next epoch's committee for the shard group containing `shard`, with its proof \
                           against the next epoch's validator set hash. Requires `shard`"
        ),
    ),
    responses(
        (status = 200, description = "List of epoch checkpoints", body = ListEpochCheckpointsResponse),
        (status = BAD_REQUEST, description = "`include_committees` without `shard`", body = ErrorResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Internal error", body = ErrorResponse),
    )
)]
pub async fn list_epoch_checkpoints(
    Extension(context): Extension<HandlerContext>,
    axum::extract::Query(req): axum::extract::Query<ListEpochCheckpointsRequest>,
) -> HandlerResult<Json<ListEpochCheckpointsResponse>> {
    let from_epoch = req.from_epoch.unwrap_or(Epoch(0));
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let committee_shard = committee_shard(req.include_committees, req.shard)?;

    let stored = context
        .read_only_store()
        .epoch_checkpoint_get_all(from_epoch, req.shard, u64::from(limit))
        .await
        .map_err(ErrorResponse::anyhow)?;

    let mut checkpoints = Vec::with_capacity(stored.len());
    for checkpoint in &stored {
        checkpoints.push(checkpoint_entry(&context, checkpoint, committee_shard).await?);
    }

    Ok(Json(ListEpochCheckpointsResponse { checkpoints }))
}

#[utoipa::path(
    get,
    path = "/epoch-checkpoints/latest",
    description = "Get the latest epoch checkpoint",
    params(
        ("shard" = Option<u32>, Query, description = "The latest checkpoint of the shard group containing this shard"),
        (
            "include_committees" = Option<bool>,
            Query,
            description = "Include the next epoch's committee for the shard group containing `shard`, with its proof \
                           against the next epoch's validator set hash. Requires `shard`"
        ),
    ),
    responses(
        (status = 200, description = "The latest epoch checkpoint", body = GetLatestEpochCheckpointResponse),
        (status = BAD_REQUEST, description = "`include_committees` without `shard`", body = ErrorResponse),
        (status = 404, description = "No epoch checkpoints found", body = ErrorResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Internal error", body = ErrorResponse),
    )
)]
pub async fn get_latest_epoch_checkpoint(
    Extension(context): Extension<HandlerContext>,
    axum::extract::Query(req): axum::extract::Query<GetLatestEpochCheckpointRequest>,
) -> HandlerResult<Json<GetLatestEpochCheckpointResponse>> {
    let committee_shard = committee_shard(req.include_committees, req.shard)?;
    let checkpoint = context
        .read_only_store()
        .epoch_checkpoint_get_latest(req.shard)
        .await
        .map_err(ErrorResponse::anyhow)?;

    Ok(Json(GetLatestEpochCheckpointResponse {
        checkpoint: checkpoint_entry(&context, &checkpoint, committee_shard).await?,
    }))
}

/// The shard whose next-epoch committee to include, if committees were asked for.
fn committee_shard(include_committees: bool, shard: Option<Shard>) -> Result<Option<Shard>, ErrorResponse> {
    match (include_committees, shard) {
        (false, _) => Ok(None),
        (true, Some(shard)) => Ok(Some(shard)),
        (true, None) => Err(ErrorResponse::bad_request(
            "include_committees requires shard: the next epoch's committee is the one whose shard group contains it",
        )),
    }
}

/// `checkpoint` in its client form, with the next epoch's committee for `committee_shard` when given. A committee is
/// only of use proved against a set the checkpoint names, so a checkpoint that names none gets none.
async fn checkpoint_entry(
    context: &HandlerContext,
    checkpoint: &EpochCheckpoint,
    committee_shard: Option<Shard>,
) -> Result<EpochCheckpointEntry, ErrorResponse> {
    let next_epoch_committee = match committee_shard {
        Some(shard) if checkpoint.next_epoch_validator_set_hash().is_some() => {
            committee_for_shard(context.epoch_manager(), checkpoint.epoch() + Epoch(1), shard).await?
        },
        _ => None,
    };
    to_checkpoint_entry(checkpoint, next_epoch_committee)
}
