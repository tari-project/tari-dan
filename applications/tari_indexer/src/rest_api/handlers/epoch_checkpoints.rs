//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{Extension, Json, response::Response};
use tari_indexer_client::types::{
    EpochCheckpointEntry,
    GetLatestEpochCheckpointRequest,
    GetLatestEpochCheckpointResponse,
    GetLatestShardGroupCheckpointsResponse,
    ListEpochCheckpointsRequest,
    ListEpochCheckpointsResponse,
};
use tari_ootle_common_types::{Epoch, optional::Optional, shard::Shard};
use tari_ootle_storage::consensus_models::EpochCheckpoint;

use crate::{
    rest_api::{
        context::HandlerContext,
        error::ErrorResponse,
        handlers::HandlerResult,
        proofs::{committee_proved_against, to_checkpoint_entry},
    },
    storage_sqlite::SqliteIndexerStore,
    store::ReadOnlyStore,
};

const DEFAULT_LIMIT: u32 = 20;
/// A checkpoint carries its commit proof and the shard tree summary of its shard group, several KB of CBOR, so the
/// page size bounds the response size. A wallet's catch-up, at most about 500 checkpoints, takes five pages.
const MAX_LIMIT: u32 = 100;
/// The `max-age` of a response that a new checkpoint can change, so that a proxy serves the wallets that poll for the
/// next checkpoint. A client sees a new checkpoint up to this many seconds late; a shared proxy holds it for about half
/// as long (`s-maxage`).
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
    let (from_epoch, limit) = page_bounds(&req);
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

    let settled = is_settled_page(&req, &checkpoints);
    let body = Json(ListEpochCheckpointsResponse { checkpoints });
    if settled {
        Ok(context.apply_settled_cache_control(body))
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
    let checkpoint = latest_stored_checkpoint(context.read_only_store(), req.shard).await?;

    let body = Json(GetLatestEpochCheckpointResponse {
        checkpoint: checkpoint_entry(&context, &checkpoint, committee_shard).await?,
    });
    Ok(context.apply_cache_control(body, RECENT_MAX_AGE_SECS))
}

#[utoipa::path(
    get,
    path = "/epoch-checkpoints/latest/shard-groups",
    description = "Get the latest epoch checkpoint of the shard group holding each shard, without committees",
    responses(
        (status = 200, description = "The latest epoch checkpoint of each shard group", body = GetLatestShardGroupCheckpointsResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Internal error", body = ErrorResponse),
    )
)]
pub async fn get_latest_shard_group_checkpoints(
    Extension(context): Extension<HandlerContext>,
) -> HandlerResult<Response> {
    let checkpoints = latest_shard_group_checkpoint_entries(context.read_only_store()).await?;
    let body = Json(GetLatestShardGroupCheckpointsResponse { checkpoints });
    Ok(context.apply_cache_control(body, RECENT_MAX_AGE_SECS))
}

/// The latest stored checkpoint of the shard group holding each shard, in its client form. Every wallet asks for the
/// same response within an epoch, so none carries a committee.
async fn latest_shard_group_checkpoint_entries(
    store: &ReadOnlyStore<SqliteIndexerStore>,
) -> Result<Vec<EpochCheckpointEntry>, ErrorResponse> {
    store
        .epoch_checkpoint_get_latest_of_each_shard_group()
        .await
        .map_err(ErrorResponse::anyhow)?
        .iter()
        .map(|checkpoint| to_checkpoint_entry(checkpoint, None))
        .collect()
}

/// The latest stored checkpoint, of the shard group containing `shard` when given. Not found while the store holds
/// none, as before the network ends its first epoch.
async fn latest_stored_checkpoint(
    store: &ReadOnlyStore<SqliteIndexerStore>,
    shard: Option<Shard>,
) -> Result<EpochCheckpoint, ErrorResponse> {
    store
        .epoch_checkpoint_get_latest(shard)
        .await
        .optional()
        .map_err(ErrorResponse::anyhow)?
        .ok_or_else(|| ErrorResponse::not_found("This indexer holds no epoch checkpoint"))
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
            committee_proved_against(context, checkpoint.epoch() + Epoch(1), &validator_set_hash, shard).await?
        },
        _ => None,
    };
    to_checkpoint_entry(checkpoint, next_epoch_committee)
}

/// The first epoch and the clamped page size that `req` lists.
fn page_bounds(req: &ListEpochCheckpointsRequest) -> (Epoch, u32) {
    (
        req.from_epoch.unwrap_or(Epoch(0)),
        req.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT),
    )
}

/// Whether `checkpoints`, the page that `req` lists, can no longer change. Only a page of one shard's checkpoints can
/// be: stored checkpoints are never replaced, and each epoch has at most one for the shard group containing a shard,
/// so a full page of consecutive epochs from `from_epoch` has no gap for a later checkpoint to fill. With committees,
/// each checkpoint that names a next-epoch set must also carry the committee that proves against it, since a missing
/// or mismatched one is filled in or corrected once this indexer holds the set.
fn is_settled_page(req: &ListEpochCheckpointsRequest, checkpoints: &[EpochCheckpointEntry]) -> bool {
    let (from_epoch, limit) = page_bounds(req);
    let is_full = limit > 0 && checkpoints.len() == limit as usize;
    let is_consecutive = checkpoints
        .iter()
        .enumerate()
        .all(|(i, entry)| entry.epoch.as_u64() == from_epoch.as_u64() + i as u64);
    let has_committees = !req.include_committees ||
        checkpoints.iter().all(
            |entry| match (&entry.next_epoch_validator_set_hash, &entry.next_epoch_committee) {
                (None, _) => true,
                (Some(hash), Some(committee)) => committee.validator_set_hash == *hash,
                (Some(_), None) => false,
            },
        );
    req.shard.is_some() && is_full && is_consecutive && has_committees
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

    fn req(
        from_epoch: u64,
        limit: Option<u32>,
        shard: Option<u32>,
        include_committees: bool,
    ) -> ListEpochCheckpointsRequest {
        ListEpochCheckpointsRequest {
            from_epoch: Some(Epoch(from_epoch)),
            limit,
            shard: shard.map(Shard::from),
            include_committees,
        }
    }

    #[tokio::test]
    async fn an_empty_store_has_no_latest_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteIndexerStore::try_create(dir.path().join("indexer.db")).unwrap();
        let store = ReadOnlyStore::new(store);

        for shard in [None, Some(Shard::from(1u32))] {
            let err = latest_stored_checkpoint(&store, shard).await.unwrap_err();
            assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND, "{}", err.error);
        }
    }

    #[tokio::test]
    async fn the_latest_checkpoint_of_each_shard_group_is_served_without_committees() {
        use crate::{
            storage_sqlite::{epoch_checkpoint, temp_store},
            store::{IndexerStore, IndexerStoreWriteTransaction},
        };

        let (_dir, store) = temp_store().await;
        let low = ShardGroup::new(1u32, 127u32);
        let high = ShardGroup::new(128u32, 255u32);
        store
            .with_write_tx(move |tx| {
                for checkpoint in [
                    epoch_checkpoint(1, low, true),
                    epoch_checkpoint(1, high, true),
                    epoch_checkpoint(2, low, true),
                    epoch_checkpoint(2, high, true),
                ] {
                    tx.insert_or_ignore_epoch_checkpoint(&checkpoint)?;
                }
                Ok::<_, tari_ootle_storage::StorageError>(())
            })
            .await
            .unwrap();

        let entries = latest_shard_group_checkpoint_entries(&ReadOnlyStore::new(store))
            .await
            .unwrap();
        let keys = entries
            .iter()
            .map(|entry| (entry.epoch, entry.shard_group))
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![(Epoch(2), low), (Epoch(2), high)]);
        assert!(entries.iter().all(|entry| entry.next_epoch_committee.is_none()));
    }

    #[test]
    fn a_full_page_of_consecutive_epochs_is_settled() {
        let page = [
            entry(4, Some(1), Some(1)),
            entry(5, None, None),
            entry(6, Some(2), Some(2)),
        ];
        assert!(is_settled_page(&req(4, Some(3), Some(0), true), &page));
    }

    #[test]
    fn a_page_of_every_shard_is_not_settled() {
        let page = [entry(4, None, None), entry(5, None, None)];
        assert!(!is_settled_page(&req(4, Some(2), None, false), &page));
    }

    #[test]
    fn a_page_is_full_at_the_clamped_limit() {
        let page = (0..u64::from(MAX_LIMIT))
            .map(|i| entry(4 + i, None, None))
            .collect::<Vec<_>>();
        assert!(is_settled_page(&req(4, Some(MAX_LIMIT + 50), Some(0), false), &page));
        let default_page = (0..u64::from(DEFAULT_LIMIT))
            .map(|i| entry(4 + i, None, None))
            .collect::<Vec<_>>();
        assert!(is_settled_page(&req(4, None, Some(0), false), &default_page));
    }

    #[test]
    fn a_page_that_a_later_checkpoint_can_change_is_not_settled() {
        let page = [entry(4, None, None), entry(5, None, None)];
        assert!(!is_settled_page(&req(4, Some(3), Some(0), false), &page), "short page");
        assert!(
            !is_settled_page(&req(3, Some(2), Some(0), false), &page),
            "gap at the start"
        );
        let gapped = [entry(4, None, None), entry(6, None, None)];
        assert!(
            !is_settled_page(&req(4, Some(2), Some(0), false), &gapped),
            "gap inside"
        );
        assert!(!is_settled_page(&req(4, Some(0), Some(0), false), &[]), "empty page");
    }

    #[test]
    fn a_page_missing_a_committee_it_can_still_gain_is_not_settled() {
        let missing = [entry(4, Some(1), None)];
        assert!(!is_settled_page(&req(4, Some(1), Some(0), true), &missing));
        assert!(is_settled_page(&req(4, Some(1), Some(0), false), &missing));
        let mismatched = [entry(4, Some(1), Some(2))];
        assert!(!is_settled_page(&req(4, Some(1), Some(0), true), &mismatched));
    }
}
