//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{array, iter};

use axum::{
    Extension,
    Json,
    extract::{Path, Query},
};
use tari_engine_types::substate::SubstateId;
use tari_indexer_client::types::{GetSubstateRequest, GetSubstateResponse, GetSubstatesRequest, GetSubstatesResponse};
use tari_indexer_lib::error::IndexerError;
use tari_ootle_common_types::{SubstateRequirementRef, optional::IsNotFoundError};

use crate::{
    rest_api::{context::HandlerContext, error::ErrorResponse, handlers::HandlerResult},
    substate_manager::{FetchedSubstate, SubstateManagerError},
};

/// Maps a lookup failure to a status the caller can act on.
///
/// A substate that is not there, and a version that has since been spent, are both answers about the
/// thing that was asked for rather than failures of the indexer. A caller has to be able to tell
/// those from the indexer being unable to answer at all, which is the only case left as a server
/// error.
fn substate_lookup_error(e: SubstateManagerError) -> ErrorResponse {
    // A down is never undone, so naming a spent version is permanent for that version: the caller
    // has to resolve the substate again rather than retry what it asked for.
    if matches!(e, SubstateManagerError::InputSubstateIsDown { .. }) || e.is_not_found_error() {
        return ErrorResponse::not_found(e.to_string());
    }
    // No committee answers for the substate's shard group at this epoch. The network will have one
    // again, so the caller is told to come back rather than that the indexer broke.
    if matches!(
        e,
        SubstateManagerError::IndexerError(IndexerError::NoCommitteeMembers { .. })
    ) {
        return ErrorResponse::service_unavailable(e.to_string());
    }
    ErrorResponse::internal_error(format!("Error getting substate: {e}"))
}

#[utoipa::path(
    get,
    path = "/substates/{substate_id}",
    description = "Fetches a substate by ID",
    params(
        ("substate_id" = String, Path, description = "The substate ID to fetch"),
        ("local_search_only" = bool, Query, description = "If true, only search local storage for the substate"),
        ("version" = Option<u32>, Query, description = "Minimum version of the substate to fetch"),
    ),
    responses(
        (status = 200, description = "Substate details", body = GetSubstateResponse),
        (
            status = 404,
            description = "No such substate, or the version asked for has been spent",
            body = ErrorResponse
        ),
        (
            status = SERVICE_UNAVAILABLE,
            description = "Indexer is still syncing, or no committee answers for the substate's shard group",
            body = ErrorResponse
        ),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to fetch substate", body = ErrorResponse),
    )
)]
pub async fn get_substate(
    Extension(context): Extension<HandlerContext>,
    Path(substate_id): Path<SubstateId>,
    Query(req): Query<GetSubstateRequest>,
) -> HandlerResult<Json<GetSubstateResponse>> {
    if !context
        .epoch_manager()
        .is_initial_scanning_complete()
        .await
        .map_err(ErrorResponse::anyhow)?
    {
        return Err(ErrorResponse::service_unavailable(
            "Indexer is still syncing. Please try again later.",
        ));
    }
    let requirement = SubstateRequirementRef::new(&substate_id, req.version);

    let manager = context.substate_manager();
    let maybe_substate = if req.local_search_only {
        manager
            .get_cached_substates(array::from_ref(requirement.substate_id()))
            .await
            .map(|a| {
                a.into_iter()
                    .find(|(_, substate)| req.version.is_none_or(|v| substate.version() == v))
                    .map(|(_, substate)| FetchedSubstate {
                        substate,
                        verified: false,
                    })
            })
            .map_err(substate_lookup_error)?
    } else {
        manager
            .get_substates(iter::once(requirement))
            .await
            .map(|a| a.into_iter().next().map(|(_, fetched)| fetched))
            .map_err(substate_lookup_error)?
    };

    match maybe_substate {
        Some(fetched) => Ok(Json(GetSubstateResponse {
            version: fetched.substate.version(),
            substate: fetched.substate.into_substate_value(),
            // True when this value was checked against the committee before being accepted. False
            // for local-only lookups, when verification is disabled, or when no committee member
            // could supply a proof yet (e.g. nothing committed since an epoch change).
            verified: fetched.verified,
        })),
        None => Err(ErrorResponse::not_found(format!("Substate {} not found", substate_id))),
    }
}

#[utoipa::path(
    post,
    path = "/substates/fetch",
    description = "Fetches several substates by their IDs",
    responses(
        (status = 200, description = "Substates details", body = GetSubstatesResponse),
        (status = BAD_REQUEST, description = "Too many substates requested", body = ErrorResponse),
        (
            status = SERVICE_UNAVAILABLE,
            description = "Indexer is still syncing, or no committee answers for the substate's shard group",
            body = ErrorResponse
        ),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to fetch substates", body = ErrorResponse),
    ),
)]
pub async fn fetch_substates(
    Extension(context): Extension<HandlerContext>,
    Json(req): Json<GetSubstatesRequest>,
) -> HandlerResult<Json<GetSubstatesResponse>> {
    const MAX_REQUESTS: usize = 20;

    let GetSubstatesRequest { requests, cached_only } = req;

    if requests.len() > MAX_REQUESTS {
        return Err(ErrorResponse::bad_request(format!(
            "Cannot request more than {MAX_REQUESTS} substates at once"
        )));
    }

    if cached_only {
        let substates = context
            .substate_manager()
            .get_cached_substates(requests.as_slice())
            .await
            .map_err(|e| ErrorResponse::internal_error(format!("Error getting substate: {}", e)))?;

        return Ok(Json(GetSubstatesResponse { substates }));
    }

    let substates = context
        .substate_manager()
        .fetch_and_cache_substates(requests.as_slice())
        .await
        .map_err(substate_lookup_error)?;

    Ok(Json(GetSubstatesResponse { substates }))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use tari_ootle_storage::StorageError;

    use super::*;

    fn substate() -> SubstateId {
        format!("component_{:064x}", 1).parse().unwrap()
    }

    /// Naming a version that has since been superseded says something about the substate, not about
    /// the indexer, so the caller is told what it asked for is gone rather than that we broke.
    #[test]
    fn a_spent_version_is_not_found() {
        let e = SubstateManagerError::InputSubstateIsDown {
            substate_id: substate(),
            version: 0,
        };
        let resp = substate_lookup_error(e);
        assert_eq!(resp.status, StatusCode::NOT_FOUND);
        // The version the caller named has to survive into the message for it to re-resolve.
        assert!(resp.error.contains("v0"), "{}", resp.error);
    }

    #[test]
    fn a_substate_that_does_not_exist_is_not_found() {
        let resp = substate_lookup_error(SubstateManagerError::InputSubstateDoesNotExist {
            substate_id: substate(),
        });
        assert_eq!(resp.status, StatusCode::NOT_FOUND);
    }

    /// A shard group without a committee is a state the network leaves on its own, so the caller is
    /// told to come back rather than that the indexer is broken.
    #[test]
    fn a_shard_group_without_a_committee_is_unavailable() {
        let resp = substate_lookup_error(SubstateManagerError::IndexerError(IndexerError::NoCommitteeMembers {
            details: "no validators are assigned to ShardGroup(129-256)".to_string(),
        }));
        assert_eq!(resp.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(resp.error.contains("ShardGroup(129-256)"), "{}", resp.error);
    }

    /// Only the indexer being unable to answer is a server error, or a caller cannot tell the two
    /// apart and retries something that will never succeed.
    #[test]
    fn a_failure_to_answer_stays_a_server_error() {
        let resp = substate_lookup_error(SubstateManagerError::StorageError(StorageError::QueryError {
            reason: "boom".to_string(),
        }));
        assert_eq!(resp.status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
