//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{array, collections::HashMap};

use axum::{
    Extension,
    Json,
    extract::{Path, Query},
};
use tari_engine_types::substate::SubstateId;
use tari_indexer_client::types::{GetSubstateRequest, GetSubstateResponse, GetSubstatesRequest, GetSubstatesResponse};
use tari_indexer_lib::{cached_substate_manager::ProvenSubstate, error::IndexerError};
use tari_ootle_common_types::{SubstateRequirementRef, optional::IsNotFoundError};

use crate::{
    rest_api::{
        context::HandlerContext,
        error::ErrorResponse,
        handlers::HandlerResult,
        proofs::{require_proof_verification, to_substate_proof},
    },
    substate_manager::{FetchedSubstate, SubstateManagerError},
};

/// Maps a lookup failure to a status the caller can act on.
///
/// A substate that is not there, and a version that has since been spent, are both answers about the
/// thing that was asked for rather than failures of the indexer. A caller has to be able to tell
/// those from the indexer being unable to answer at all, which is the only case left as a server
/// error.
pub(super) fn substate_lookup_error(e: SubstateManagerError) -> ErrorResponse {
    // A down is never undone, so naming a spent version is permanent for that version: the caller
    // has to resolve the substate again rather than retry what it asked for.
    if matches!(e, SubstateManagerError::InputSubstateIsDown { .. }) || e.is_not_found_error() {
        return ErrorResponse::not_found(e.to_string());
    }
    // No committee answers for the substate's shard group at this epoch, or its members disagree on
    // the substate and none can prove it. Both pass as the network settles, so the caller is told to
    // come back rather than that the indexer broke.
    if matches!(
        e,
        SubstateManagerError::IndexerError(
            IndexerError::NoCommitteeMembers { .. } | IndexerError::InvalidSubstateState
        )
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
        ("version" = Option<u64>, Query, description = "Minimum version of the substate to fetch"),
        (
            "include_proof" = Option<bool>,
            Query,
            description = "If true, include the proof the substate was verified with"
        ),
    ),
    responses(
        (status = 200, description = "Substate details", body = GetSubstateResponse),
        (
            status = BAD_REQUEST,
            description = "A proof was requested together with local_search_only, or from an indexer that does not verify proofs",
            body = ErrorResponse
        ),
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
    if req.include_proof {
        if req.local_search_only {
            return Err(ErrorResponse::bad_request(
                "include_proof cannot be combined with local_search_only: a proof the indexer does not hold is \
                 fetched from the committee",
            ));
        }
        require_proof_verification(manager.verifies_substates())?;
    }
    let maybe_substate = if req.include_proof {
        manager
            .fetch_substate_with_proof(requirement)
            .await
            .map_err(substate_lookup_error)?
    } else if req.local_search_only {
        manager
            .get_cached_substates(array::from_ref(requirement.substate_id()))
            .await
            .map(|a| {
                a.into_iter()
                    .find(|(_, substate)| req.version.is_none_or(|v| substate.version() == v))
                    .map(|(_, substate)| FetchedSubstate {
                        substate,
                        verified: false,
                        proof: None,
                    })
            })
            .map_err(substate_lookup_error)?
    } else {
        manager
            .fetch_substate(requirement)
            .await
            .map_err(substate_lookup_error)?
    };

    let Some(fetched) = maybe_substate else {
        return Err(ErrorResponse::not_found(format!("Substate {} not found", substate_id)));
    };
    let proof = fetched.proof.map(to_substate_proof).transpose()?;
    Ok(Json(GetSubstateResponse {
        version: fetched.substate.version(),
        substate: fetched.substate.into_substate_value(),
        // True when this value was checked against the committee before being accepted. False
        // for local-only lookups, when verification is disabled, or when no committee member
        // could supply a proof yet (e.g. nothing committed since an epoch change).
        verified: fetched.verified,
        proof,
    }))
}

#[utoipa::path(
    post,
    path = "/substates/fetch",
    description = "Fetches several substates by their IDs",
    responses(
        (status = 200, description = "Substates details", body = GetSubstatesResponse),
        (
            status = BAD_REQUEST,
            description = "Too many substates requested, or proofs requested together with cached_only or from an \
                           indexer that does not verify proofs",
            body = ErrorResponse
        ),
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

    let GetSubstatesRequest {
        requests,
        cached_only,
        include_proofs,
    } = req;

    if requests.len() > MAX_REQUESTS {
        return Err(ErrorResponse::bad_request(format!(
            "Cannot request more than {MAX_REQUESTS} substates at once"
        )));
    }

    if include_proofs {
        if cached_only {
            return Err(ErrorResponse::bad_request(
                "include_proofs cannot be combined with cached_only: the proofs are fetched from the committee",
            ));
        }
        require_proof_verification(context.substate_manager().verifies_substates())?;
    }

    if cached_only {
        let substates = context
            .substate_manager()
            .get_cached_substates(requests.as_slice())
            .await
            .map_err(|e| ErrorResponse::internal_error(format!("Error getting substate: {}", e)))?;

        return Ok(Json(GetSubstatesResponse {
            substates,
            proofs: HashMap::new(),
        }));
    }

    let fetched = context
        .substate_manager()
        .fetch_and_cache_substates(requests.as_slice())
        .await
        .map_err(substate_lookup_error)?;

    let mut substates = HashMap::with_capacity(fetched.len());
    let mut proofs = HashMap::new();
    for (id, ProvenSubstate { substate, proof }) in fetched {
        if include_proofs && let Some(proof) = proof {
            proofs.insert(id.clone(), to_substate_proof(proof)?);
        }
        substates.insert(id, substate);
    }

    Ok(Json(GetSubstatesResponse { substates, proofs }))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use tari_ootle_common_types::SubstateVersion;
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
            version: SubstateVersion::ZERO,
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

    #[test]
    fn a_committee_that_cannot_agree_is_unavailable() {
        let resp = substate_lookup_error(SubstateManagerError::IndexerError(IndexerError::InvalidSubstateState));
        assert_eq!(resp.status, StatusCode::SERVICE_UNAVAILABLE);
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
