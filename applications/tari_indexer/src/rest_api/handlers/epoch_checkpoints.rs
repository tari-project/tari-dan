//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{Extension, Json, response::Response};
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
    proofs::{committee_proved_against, to_checkpoint_entry},
};

const DEFAULT_LIMIT: u32 = 20;
/// A checkpoint carries its commit proof and the shard tree summary of its shard group, several KB of CBOR, so the
/// page size bounds the response size. A wallet's catch-up, at most about 500 checkpoints, takes five pages.
const MAX_LIMIT: u32 = 100;
/// A response that a new checkpoint can change is cached briefly, so that a proxy serves the wallets that poll for the
/// next checkpoint, at the cost of seeing it up to this many seconds late.
const RECENT_MAX_AGE_SECS: u32 = 10;

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
) -> HandlerResult<Response> {
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

    let settled = req.shard.is_some() && is_settled_page(from_epoch, limit, committee_shard.is_some(), &checkpoints);
    let body = Json(ListEpochCheckpointsResponse { checkpoints });
    if settled {
        Ok(context.apply_immutable_cache_control(body))
    } else {
        Ok(context.apply_cache_control(body, RECENT_MAX_AGE_SECS))
    }
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
) -> HandlerResult<Response> {
    let committee_shard = committee_shard(req.include_committees, req.shard)?;
    let checkpoint = context
        .read_only_store()
        .epoch_checkpoint_get_latest(req.shard)
        .await
        .map_err(ErrorResponse::anyhow)?;

    let body = Json(GetLatestEpochCheckpointResponse {
        checkpoint: checkpoint_entry(&context, &checkpoint, committee_shard).await?,
    });
    Ok(context.apply_cache_control(body, RECENT_MAX_AGE_SECS))
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
    let next_epoch_committee = match (committee_shard, checkpoint.next_epoch_validator_set_hash()) {
        (Some(shard), Some(validator_set_hash)) => {
            committee_proved_against(
                context.committee_proofs(),
                context.epoch_manager(),
                checkpoint.epoch() + Epoch(1),
                &validator_set_hash,
                shard,
            )
            .await?
        },
        _ => None,
    };
    to_checkpoint_entry(checkpoint, next_epoch_committee)
}

/// Whether a page of one shard's checkpoints, listed from `from_epoch` with `limit`, can no longer change. Stored
/// checkpoints are never replaced, and each epoch has at most one for the shard group containing a shard, so a full
/// page of consecutive epochs from `from_epoch` has no gap for a later checkpoint to fill. With committees, each
/// checkpoint that names a next-epoch set must also carry the committee that proves against it, since a missing or
/// mismatched one is filled in or corrected once this indexer holds the set.
fn is_settled_page(from_epoch: Epoch, limit: u32, with_committees: bool, checkpoints: &[EpochCheckpointEntry]) -> bool {
    let is_full = limit > 0 && checkpoints.len() == limit as usize;
    let is_consecutive = checkpoints
        .iter()
        .enumerate()
        .all(|(i, entry)| entry.epoch.as_u64() == from_epoch.as_u64() + i as u64);
    let has_committees = !with_committees ||
        checkpoints.iter().all(
            |entry| match (&entry.next_epoch_validator_set_hash, &entry.next_epoch_committee) {
                (None, _) => true,
                (Some(hash), Some(committee)) => committee.validator_set_hash == *hash,
                (Some(_), None) => false,
            },
        );
    is_full && is_consecutive && has_committees
}

#[cfg(test)]
mod tests {
    use tari_indexer_client::types::ValidatorSetCommittee;
    use tari_ootle_common_types::{ShardGroup, ValidatorSetHash};

    use super::*;

    fn entry(epoch: u64, next_hash: Option<u8>, committee_hash: Option<u8>) -> EpochCheckpointEntry {
        let shard_group = ShardGroup::new(0u32, 255u32);
        EpochCheckpointEntry {
            epoch: Epoch(epoch),
            shard_group,
            next_epoch_validator_set_hash: next_hash.map(|b| ValidatorSetHash::from_array([b; 32])),
            checkpoint: vec![],
            next_epoch_committee: committee_hash.map(|b| ValidatorSetCommittee {
                epoch: Epoch(epoch + 1),
                shard_group,
                validator_set_hash: ValidatorSetHash::from_array([b; 32]),
                members: vec![],
                proof: vec![],
            }),
        }
    }

    #[test]
    fn a_full_page_of_consecutive_epochs_is_settled() {
        let page = [
            entry(4, Some(1), Some(1)),
            entry(5, None, None),
            entry(6, Some(2), Some(2)),
        ];
        assert!(is_settled_page(Epoch(4), 3, true, &page));
    }

    #[test]
    fn a_page_that_a_later_checkpoint_can_change_is_not_settled() {
        let page = [entry(4, None, None), entry(5, None, None)];
        assert!(!is_settled_page(Epoch(4), 3, false, &page), "short page");
        assert!(!is_settled_page(Epoch(3), 2, false, &page), "gap at the start");
        let gapped = [entry(4, None, None), entry(6, None, None)];
        assert!(!is_settled_page(Epoch(4), 2, false, &gapped), "gap inside");
        assert!(!is_settled_page(Epoch(4), 0, false, &[]), "empty page");
    }

    #[test]
    fn a_page_missing_a_committee_it_can_still_gain_is_not_settled() {
        let missing = [entry(4, Some(1), None)];
        assert!(!is_settled_page(Epoch(4), 1, true, &missing));
        assert!(is_settled_page(Epoch(4), 1, false, &missing));
        let mismatched = [entry(4, Some(1), Some(2))];
        assert!(!is_settled_page(Epoch(4), 1, true, &mismatched));
    }
}
