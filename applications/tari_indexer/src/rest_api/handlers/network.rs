//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    ops::Deref,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{Extension, Json, response::Response};
use tari_consensus::hotstuff::ConsensusCurrentState;
use tari_indexer_client::{
    types,
    types::{
        ConnectionDirection,
        GetConnectionsResponse,
        GetNetworkEconomicsResponse,
        GetNetworkInfoResponse,
        GetNetworkSyncStateResponse,
        NetworkDescription,
        ScheduledBurnRate,
        SyncProgress,
        ValidatorConsensusState,
        ValidatorProbeError,
        ValidatorProbeErrorKind,
        ValidatorStatus,
        ValidatorStatusSnapshot,
    },
};
use tari_ootle_common_types::optional::Optional;
use tari_template_lib_types::Amount;

use crate::{
    exhaust_burn_rate::resolve_burn_rate_outlook,
    network_state_sync::ProbeFailure,
    rest_api::{context::HandlerContext, error::ErrorResponse, handlers::HandlerResult},
};

#[utoipa::path(get, path = "/network", description = "Get network info")]
pub async fn get(Extension(context): Extension<HandlerContext>) -> HandlerResult<Json<GetNetworkInfoResponse>> {
    let epoch = context.epoch_manager().get_current_epoch();
    let network = context.network();

    let response = GetNetworkInfoResponse {
        epoch,
        network_byte: network.as_byte(),
        network,
    };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/network/economics",
    description = "Get network-wide TARI economic totals (claimed, burned, fee volume, supply, fees claimable by validators, target rate and the rate changes scheduled ahead)",
    responses(
        (status = 200, description = "Network economic totals", body = GetNetworkEconomicsResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to fetch network economics", body = ErrorResponse),
    ),
)]
pub async fn get_economics(Extension(context): Extension<HandlerContext>) -> HandlerResult<Response> {
    let current_epoch = context.epoch_manager().get_current_epoch();
    let econ = context
        .read_only_store()
        .get_tari_economics()
        .await
        .map_err(ErrorResponse::anyhow)?;

    let outlook = resolve_burn_rate_outlook(context.substate_manager(), context.network(), current_epoch).await;
    // Supply nets the receipt-sourced burn against claimed (both advance on the state-sync frontier), matching
    // `get_xtr_total_supply`; `total_exhaust_burned` (header) is reported alongside as a cross-check.
    let total_supply = econ
        .total_claimed
        .checked_sub(econ.receipt_exhaust_burned)
        .unwrap_or_else(Amount::zero);

    Ok(context.apply_cache_control(
        Json(GetNetworkEconomicsResponse {
            current_epoch,
            total_claimed: econ.total_claimed,
            total_exhaust_burned: econ.total_exhaust_burned,
            fee_volume: econ.fee_volume,
            receipt_exhaust_burned: econ.receipt_exhaust_burned,
            total_supply,
            transaction_receipt_count: econ.transaction_receipt_count,
            validator_claimable_fees: econ.validator_claimable_fees,
            target_burn_rate_bps: outlook.current.as_bps(),
            scheduled_burn_rates: outlook
                .scheduled
                .into_iter()
                .map(|change| ScheduledBurnRate {
                    activation_epoch: change.activation_epoch,
                    rate_bps: change.rate_bps,
                })
                .collect(),
            burn_rate_retired_from: outlook.retired_from,
        }),
        60,
    ))
}

#[utoipa::path(get, path = "/network/stats", description = "Get network sync stats",
    responses(
        (status = 200, description = "Network sync stats", body = GetNetworkSyncStateResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to fetch network sync stats", body = ErrorResponse),
    ),
)]
pub async fn get_network_sync_stats(
    Extension(context): Extension<HandlerContext>,
) -> HandlerResult<Json<GetNetworkSyncStateResponse>> {
    let network_desc = context
        .epoch_manager()
        .get_network_description()
        .await
        .map_err(ErrorResponse::anyhow)?;
    let sync_progress = context
        .read_only_store()
        .get_sync_progress()
        .await
        .optional()
        .map_err(ErrorResponse::anyhow)?;

    let validators = context
        .validator_status()
        .records()
        .await
        .into_iter()
        .map(|(peer, record)| ValidatorStatus {
            peer_id: peer.to_string(),
            shard_group: record.shard_group,
            probed_at_unix_s: to_unix_secs(record.probed_at),
            probe_error: record.error.map(to_client_probe_error),
            snapshot: record.snapshot.map(|snapshot| ValidatorStatusSnapshot {
                epoch: snapshot.epoch,
                height: snapshot.height.as_u64(),
                state: to_client_consensus_state(snapshot.state),
                observed_at_unix_s: to_unix_secs(snapshot.observed_at),
            }),
        })
        .collect();

    let response = GetNetworkSyncStateResponse {
        network_desc: NetworkDescription {
            epoch: network_desc.epoch,
            shard_groups: network_desc
                .shard_groups
                .into_iter()
                .map(|(shard_group, info)| (shard_group, info.num_members))
                .collect(),
            num_preshards: network_desc.num_preshards,
        },
        sync_progress: sync_progress.map(|p| SyncProgress {
            last_epoch: p.last_epoch,
            checkpoint_progress: p.checkpoint_progress.into_iter().collect(),
            last_state_versions: p.last_state_versions.into_iter().collect(),
        }),
        validators,
    };
    Ok(Json(response))
}

fn to_client_probe_error(failure: ProbeFailure) -> ValidatorProbeError {
    match failure {
        ProbeFailure::StatusUnavailable(message) => ValidatorProbeError {
            kind: ValidatorProbeErrorKind::StatusUnavailable,
            message,
        },
        ProbeFailure::InvalidProof(message) => ValidatorProbeError {
            kind: ValidatorProbeErrorKind::InvalidProof,
            message,
        },
    }
}

fn to_unix_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

fn to_client_consensus_state(state: ConsensusCurrentState) -> ValidatorConsensusState {
    match state {
        ConsensusCurrentState::Initialising => ValidatorConsensusState::Initialising,
        ConsensusCurrentState::Idle => ValidatorConsensusState::Idle,
        ConsensusCurrentState::CheckSync => ValidatorConsensusState::CheckSync,
        ConsensusCurrentState::Syncing => ValidatorConsensusState::Syncing,
        ConsensusCurrentState::Running => ValidatorConsensusState::Running,
        ConsensusCurrentState::Sleeping => ValidatorConsensusState::Sleeping,
        ConsensusCurrentState::Shutdown => ValidatorConsensusState::Shutdown,
    }
}

#[utoipa::path(get, path = "/network/connections", description = "Get active peer connections",
    responses(
        (status = 200, description = "Active peer connections", body = GetConnectionsResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to fetch peer connections", body = ErrorResponse),
    ),
)]
pub async fn get_connections(Extension(context): Extension<HandlerContext>) -> HandlerResult<Response> {
    let active_connections = context
        .networking()
        .get_active_connections()
        .await
        .map_err(ErrorResponse::anyhow)?;

    let connections = active_connections
        .into_iter()
        .map(|conn| types::Connection {
            connection_id: conn.connection_id.to_string(),
            peer_id: conn.peer_id.to_string(),
            direction: if conn.endpoint.is_dialer() {
                ConnectionDirection::Outbound
            } else {
                ConnectionDirection::Inbound
            },
            age: conn.age(),
            ping_latency: conn.ping_latency,
            user_agent: conn.user_agent.map(|arc| arc.deref().clone()),
        })
        .collect();

    Ok(context.apply_cache_control(Json(GetConnectionsResponse { connections }), 10))
}
