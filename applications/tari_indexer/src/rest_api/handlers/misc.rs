//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{
    Extension,
    Json,
    response::{IntoResponse, Response},
};
use serde_json::json;
use tari_epoch_manager::EpochManagerReader;
use tari_epoch_oracles::store::StoreKey;
use tari_indexer_client::types::{
    GetEpochManagerStatsResponse,
    GetIdentityResponse,
    GetIndexerInfoResponse,
    IndexerReadyResponse,
};

use crate::rest_api::{context::HandlerContext, error::ErrorResponse, handlers::HandlerResult};

#[utoipa::path(
    get,
    path = "/identity",
    description = "Get indexer network identity information",
    responses(
        (status = 200, description = "Indexer network identity info", body = GetIdentityResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to get identity info", body = ErrorResponse),
    ),
)]
pub async fn get_identity(Extension(context): Extension<HandlerContext>) -> HandlerResult<Response> {
    let info = context
        .networking()
        .get_local_peer_info()
        .await
        .map_err(ErrorResponse::anyhow)?;
    let response = GetIdentityResponse {
        peer_id: info.peer_id.to_string(),
        public_key: *context.public_key(),
        // NOTE: we use a string here to avoid depending on multiaddr in the client
        public_addresses: info.listen_addrs.iter().map(|addr| addr.to_string()).collect(),
    };
    Ok(context.apply_cache_control(Json(response), 1000))
}

#[utoipa::path(
    get,
    path = "/info",
    description = "Get this indexer's own configuration, as far as it affects what its API returns. Every field is \
                   local to the node answering the request - two indexers on the same network can disagree on all of \
                   them.",
    responses(
        (status = 200, description = "Indexer configuration", body = GetIndexerInfoResponse),
    ),
)]
pub async fn get_info(Extension(context): Extension<HandlerContext>) -> HandlerResult<Response> {
    let config = context.published_config();
    let network = context.network();

    let response = GetIndexerInfoResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        network,
        network_byte: network.as_byte(),
        sidechain_id: config.sidechain_id,
        current_epoch: context.epoch_manager().get_current_epoch(),
        transaction_retention_epochs: config.transaction_retention_epochs,
        transaction_receipt_retention_epochs: config.transaction_receipt_retention_epochs,
        event_retention_epochs: config.event_retention_epochs,
        index_gossiped_transactions: config.index_gossiped_transactions,
        verify_substate_proofs: config.verify_substate_proofs,
        substate_cache_max_serve_lag_secs: config.substate_cache_max_serve_lag.as_secs(),
        indexes_all_events: config.indexes_all_events,
    };
    // Only `current_epoch` changes at runtime; the rest is fixed for the life of the process.
    Ok(context.apply_cache_control(Json(response), 30))
}

#[utoipa::path(
    get,
    path = "/wait-until-ready",
    description = "Request will return an empty result when the indexer is ready to serve requests. This may take an \
                   indefinite amount of time after startup."
)]
pub async fn wait_until_ready(
    Extension(context): Extension<HandlerContext>,
) -> HandlerResult<Json<IndexerReadyResponse>> {
    // TODO: we should rather return the current state of the indexer and have clients poll
    context
        .epoch_manager()
        .wait_for_initial_scanning_to_complete()
        .await
        .map_err(ErrorResponse::anyhow)?;

    Ok(Json(IndexerReadyResponse {}))
}

#[utoipa::path(get, path = "/epoch-manager/stats", description = "Get epoch manager stats",
    responses(
        (status = 200, description = "Epoch manager stats", body = GetEpochManagerStatsResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to fetch epoch manager stats", body = ErrorResponse),
    ),
)]
pub async fn get_epoch_manager_stats(
    Extension(context): Extension<HandlerContext>,
) -> HandlerResult<Json<GetEpochManagerStatsResponse>> {
    let current_epoch = context.epoch_manager().get_current_epoch();
    let current_epoch_hash = context
        .epoch_manager()
        .get_current_epoch_hash()
        .await
        .map_err(ErrorResponse::anyhow)?;

    let current_block_height = context
        .global_db()
        .create_transaction()
        .and_then(|mut tx| {
            context
                .global_db()
                .metadata(&mut tx)
                .get_metadata::<u64>(StoreKey::BaseLayerLastScannedBlockHeight.as_key_bytes())
        })
        .map_err(ErrorResponse::anyhow)?;

    let response = GetEpochManagerStatsResponse {
        current_epoch,
        consensus_epoch: context.consensus_epoch().current_within(current_epoch),
        current_block_height: current_block_height.unwrap_or(0),
        current_block_hash: current_epoch_hash.into_array().into(),
    };
    Ok(Json(response))
}

#[utoipa::path(get, path = "/health", description = "Health check")]
pub async fn health(Extension(context): Extension<HandlerContext>) -> impl IntoResponse {
    let current_epoch = context.epoch_manager().get_current_epoch();
    let epoch_manager_ok = !context.epoch_manager().is_closed();
    let network_ok = !context.networking().is_closed();
    let db_ok = context
        .global_db()
        .with_read_tx(|tx| tx.execute_sql("SELECT 1"))
        .is_ok();
    let status = if epoch_manager_ok && network_ok && db_ok {
        "ok"
    } else {
        "error"
    };

    let is_ok = epoch_manager_ok && network_ok && db_ok;

    let body = json!({
        "status": status,
        "current_epoch": current_epoch.as_u64(),
        "epoch_manager_ok": epoch_manager_ok,
        "network_ok": network_ok,
        "db_ok": db_ok,
        "version": env!("CARGO_PKG_VERSION"),
    });

    if is_ok {
        (axum::http::StatusCode::OK, Json(body)).into_response()
    } else {
        (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}

#[utoipa::path(get, path = "/ready", description = "Indexer readiness check")]
pub async fn ready(Extension(context): Extension<HandlerContext>) -> HandlerResult<Json<serde_json::Value>> {
    match context.epoch_manager().is_initial_scanning_complete().await {
        Ok(true) => Ok(Json(json!({}))),
        Ok(false) => Err(ErrorResponse::service_unavailable(
            "Indexer is still scanning".to_string(),
        )),
        Err(e) => Err(ErrorResponse::anyhow(e)),
    }
}
