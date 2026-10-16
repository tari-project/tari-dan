//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{Extension, Json, extract::Query, response::Response};
use tari_indexer_client::types::{GetCommitteeRequest, GetCommitteeResponse, ValidatorSetCommittee};
use tari_ootle_common_types::{Epoch, ValidatorSetHash, shard::Shard};

use crate::rest_api::{
    context::HandlerContext,
    error::ErrorResponse,
    handlers::HandlerResult,
    proofs::{committee_for_shard, committee_proved_against},
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
        Some(hash) => committee_proved_against(&context, req.epoch, hash, req.shard).await?,
        None => {
            committee_for_shard(
                context.epoch_manager(),
                context.num_preshards().await?,
                req.epoch,
                req.shard,
            )
            .await?
        },
    };
    let Some(committee) = committee else {
        return Err(ErrorResponse::not_found(format!(
            "This indexer holds no validator set for {}",
            req.epoch
        )));
    };
    let is_certified = is_certified(certified_hash, &committee);
    let body = Json(GetCommitteeResponse { committee });
    if is_certified {
        Ok(context.apply_settled_cache_control(body))
    } else {
        Ok(context.apply_cache_control(body, 30))
    }
}

/// Whether `committee` proves against the validator set hash a stored checkpoint certifies, which fixes it for good.
fn is_certified(certified_hash: Option<ValidatorSetHash>, committee: &ValidatorSetCommittee) -> bool {
    certified_hash == Some(committee.validator_set_hash)
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

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::ShardGroup;

    use super::*;

    fn committee(hash: u8) -> ValidatorSetCommittee {
        ValidatorSetCommittee {
            epoch: Epoch(5),
            shard_group: ShardGroup::new(0u32, 255u32),
            validator_set_hash: ValidatorSetHash::from_array([hash; 32]),
            members: vec![],
            proof: vec![],
        }
    }

    #[test]
    fn only_a_committee_proving_against_the_certified_hash_is_certified() {
        let certified = Some(ValidatorSetHash::from_array([1; 32]));
        assert!(is_certified(certified, &committee(1)));
        assert!(!is_certified(certified, &committee(2)));
        assert!(!is_certified(None, &committee(1)));
    }
}
