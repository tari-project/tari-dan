//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{
    Extension,
    Json,
    extract::{Path, Query},
    response::Response,
};
use log::*;
use tari_indexer_client::types::{
    GetTransactionResponse,
    GetTransactionResultResponse,
    ListRecentTransactionsRequest,
    ListRecentTransactionsResponse,
    QueryTransactionEventsRequest,
    QueryTransactionEventsResponse,
    SubmitTransactionDryRunResponse,
    SubmitTransactionRequest,
    SubmitTransactionResponse,
};
use tari_ootle_common_types::{displayable::Displayable, optional::Optional};
use tari_ootle_transaction::TransactionId;
use tari_rpc_framework::RpcStatusCode;

use crate::{
    network_client::NetworkClientError,
    rest_api::{context::HandlerContext, error::ErrorResponse, handlers::HandlerResult},
    transaction_manager::error::TransactionManagerError,
};

const LOG_TARGET: &str = "tari::indexer::rest_api::handlers::transactions";

#[utoipa::path(
    post,
    path = "/transactions",
    description = "Submit a transaction to validators responsible for the involved shards",
    responses(
        (status = 200, description = "Transaction submitted successfully", body = SubmitTransactionResponse),
        (status = BAD_REQUEST, description = "Invalid transaction or request parameters", body = ErrorResponse),
        (
            status = SERVICE_UNAVAILABLE,
            description = "All validators failed to process the transaction, or a shard group it involves has no committee",
            body = ErrorResponse
        ),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to submit transaction due to an internal error", body = ErrorResponse),
    )
)]
pub async fn submit_transaction(
    Extension(context): Extension<HandlerContext>,
    Json(req): Json<SubmitTransactionRequest>,
) -> HandlerResult<Json<SubmitTransactionResponse>> {
    let request: SubmitTransactionRequest = req;
    let transaction = request
        .transaction
        .decode()
        .map_err(|e| ErrorResponse::bad_request(format!("Failed to decode transaction: {}", e)))?;

    if transaction.is_dry_run() {
        return Err(ErrorResponse::bad_request(
            "Dry-run transactions must be submitted to the /transactions/dry-run endpoint".to_string(),
        ));
    }

    let transaction_id = context
        .transaction_manager()
        .submit_transaction(transaction)
        .await
        .map_err(|e| match e {
            TransactionManagerError::NetworkClientError(net_err) => match *net_err {
                NetworkClientError::AllValidatorsFailed {
                    last_error: Some(last_err),
                    committee_size,
                } => match last_err.status() {
                    Some(status) => match status.as_status_code() {
                        RpcStatusCode::BadRequest => {
                            ErrorResponse::bad_request(format!("Bad request: {}", status.details()))
                        },
                        RpcStatusCode::NotFound => ErrorResponse::not_found(format!("Not found: {}", status.details())),
                        _ => ErrorResponse::general_error(format!(
                            "Rpc error: ({} members) {}",
                            committee_size,
                            status.details()
                        )),
                    },
                    None => {
                        ErrorResponse::general_error(format!("Rpc error: ({} members) {}", committee_size, last_err))
                    },
                },
                e @ NetworkClientError::AllValidatorsFailed { .. } => {
                    ErrorResponse::service_unavailable(format!("All validators failed: {}", e))
                },
                e @ NetworkClientError::NoCommitteeMembers |
                e @ NetworkClientError::NoCommitteeForShardGroup { .. } => {
                    ErrorResponse::service_unavailable(e.to_string())
                },
                e => ErrorResponse::anyhow(e),
            },
            TransactionManagerError::InvalidTransaction {
                transaction_id,
                details,
            } => ErrorResponse::bad_request(format!("Transaction {} is invalid: {}", transaction_id, details)),
            e => ErrorResponse::anyhow(e),
        })?;

    info!(target: LOG_TARGET, "✅ Transaction submitted: {}", transaction_id);

    Ok(Json(SubmitTransactionResponse { transaction_id }))
}

#[utoipa::path(
    post,
    path = "/transactions/dry-run",
    description = "Submit a transaction as a dry-run",
    responses(
        (status = 200, description = "Dry-run transaction processed successfully", body = SubmitTransactionDryRunResponse),
        (status = BAD_REQUEST, description = "Invalid transaction or request parameters", body = ErrorResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to process dry-run transaction due to an internal error", body = ErrorResponse),
    )
)]
pub async fn submit_transaction_dry_run(
    Extension(context): Extension<HandlerContext>,
    Json(req): Json<SubmitTransactionRequest>,
) -> HandlerResult<Json<SubmitTransactionDryRunResponse>> {
    let request: SubmitTransactionRequest = req;
    let transaction = request
        .transaction
        .decode()
        .map_err(|e| ErrorResponse::bad_request(format!("Failed to decode transaction: {}", e)))?;

    if !transaction.is_dry_run() {
        return Err(ErrorResponse::bad_request(
            "Non-dry-run transactions must be submitted to the /transactions endpoint".to_string(),
        ));
    }
    // NOTE that we do not validate the signatures for dry-run transactions. Invalid signatures are permissible for
    // dry-runs.

    let transaction_id = transaction.calculate_id();
    let exec_result = context
        .dry_run_transaction_processor()
        .process_transaction(transaction)
        .await
        .map_err(ErrorResponse::anyhow)?;

    Ok(Json(SubmitTransactionDryRunResponse {
        result: exec_result,
        transaction_id,
    }))
}

#[utoipa::path(
    get,
    path = "/transactions/recent",
    description = "List recent transactions, newest first.\n\n\
        BEST EFFORT: this lists what this indexer happens to hold — transactions submitted through it, plus those it \
        observed on the network gossip topic while it was running and subscribed. It is not a complete record of \
        network activity and absence from this listing does not mean a transaction does not exist. An indexer misses \
        a transaction if it was offline or still starting up when it was gossiped, if its inbound gossip queue was \
        full, or if the transaction has since aged past its retention window (see `transaction_retention_epochs` on \
        /info).\n\n\
        Transaction receipts are different: they are synced from network state rather than gossip, are complete from \
        genesis, and are recovered after downtime. A committed transaction whose gossip this indexer missed therefore \
        has a receipt but no entry here — list it from /transaction-receipts instead. Transactions that never \
        committed (mempool-rejected, aborted or expired) never get a receipt, so gossip is the only source for them \
        and coverage is best effort with no fallback.",
    responses(
        (status = 200, description = "List of recent transactions", body = ListRecentTransactionsResponse),
        (status = BAD_REQUEST, description = "Invalid request parameters", body = ErrorResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to list recent transactions", body = ErrorResponse),
    )
)]
pub async fn list_recent_transactions(
    Extension(context): Extension<HandlerContext>,
    Query(req): Query<ListRecentTransactionsRequest>,
) -> HandlerResult<Response> {
    let limit = req.limit.unwrap_or(100);
    if limit > 100 {
        return Err(ErrorResponse::bad_request(
            "Limit cannot be greater than 100".to_string(),
        ));
    }

    let transactions = context
        .transaction_manager()
        .list_recent_transactions(req.last_id, limit as usize, req.source)
        .await
        .map_err(ErrorResponse::anyhow)?;

    Ok(context.apply_cache_control(Json(ListRecentTransactionsResponse { transactions }), 30))
}

#[utoipa::path(
    get,
    path = "/transactions/{transaction_id}",
    description = "Get a transaction (including its instructions, fee instructions and signatures) by transaction ID.\n\n\
        BEST EFFORT: this reads the indexer's local transaction store, which holds transactions submitted through \
        this indexer and — when gossip indexing is enabled, see `index_gossiped_transactions` on /info — those it \
        observed on the network gossip topic. A 404 means this indexer has no body for the transaction, not that the \
        transaction does not exist: it may have been gossiped while the indexer was offline, dropped from a full \
        inbound queue, or pruned once past its retention window.\n\n\
        Two endpoints answer authoritatively where this one cannot. \
        /transactions/{transaction_id}/result queries the transaction's committee, so it resolves the outcome of any \
        transaction regardless of where it was submitted. /transaction-receipts is synced from network state rather \
        than gossip and is complete from genesis, so a committed transaction always has a receipt there even when its \
        body is missing here.",
    responses(
        (status = 200, description = "Transaction found", body = GetTransactionResponse),
        (status = 404, description = "Transaction not known to this indexer", body = ErrorResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to fetch transaction", body = ErrorResponse),
    )
)]
pub async fn get_transaction(
    Extension(context): Extension<HandlerContext>,
    Path(transaction_id): Path<TransactionId>,
) -> HandlerResult<Json<GetTransactionResponse>> {
    let transaction = context
        .transaction_manager()
        .get_transaction(transaction_id)
        .await
        .map_err(ErrorResponse::anyhow)?
        .ok_or_else(|| {
            ErrorResponse::not_found(format!("Transaction {transaction_id} is not known to this indexer"))
        })?;

    Ok(Json(GetTransactionResponse { transaction }))
}

#[utoipa::path(
    get,
    path = "/transactions/{transaction_id}/result",
    description = "Get the result of a submitted transaction (by transaction ID)",
    responses(
        (status = 200, description = "Transaction result found", body = GetTransactionResultResponse),
        (status = 404, description = "Transaction result not found", body = ErrorResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to fetch transaction result", body = ErrorResponse),
    )
)]
pub async fn get_transaction_result(
    Extension(context): Extension<HandlerContext>,
    Path(transaction_id): Path<TransactionId>,
) -> HandlerResult<Json<GetTransactionResultResponse>> {
    let result = context
        .transaction_manager()
        .get_transaction_result(transaction_id)
        .await
        .optional()
        .map_err(ErrorResponse::anyhow)?
        .ok_or_else(|| ErrorResponse::not_found(format!("Transaction {transaction_id} not found")))?;

    Ok(Json(GetTransactionResultResponse { result }))
}

#[utoipa::path(
    get,
    path = "/transactions/events",
    description = "Query and filter transaction events by substate ID, topic, and/or resource address.",
    responses(
        (status = 200, description = "List of transaction events matching the filters", body = QueryTransactionEventsResponse),
        (status = BAD_REQUEST, description = "Invalid request parameters", body = ErrorResponse),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to query transaction events", body = ErrorResponse),
    )
)]
pub async fn query_transaction_events(
    Extension(context): Extension<HandlerContext>,
    Query(req): Query<QueryTransactionEventsRequest>,
) -> HandlerResult<Json<QueryTransactionEventsResponse>> {
    let limit = req.limit.unwrap_or(100);
    if limit == 0 {
        return Ok(Json(QueryTransactionEventsResponse::default()));
    }

    if limit > 1000 {
        return Err(ErrorResponse::bad_request("Limit cannot be greater than 1000"));
    }

    let offset = req.offset.unwrap_or(0);

    debug!(target: LOG_TARGET,
        "Querying transaction events with filters - substate_id: {}, topic: {}, resource_address: {}, offset: {}, limit: {}",
        req.substate_id.display(), req.topic.display(), req.resource_address.display(), offset, limit
    );

    let events = context
        .read_only_store()
        .get_events(
            req.substate_id.as_ref(),
            req.topic.as_deref().map(|t| t.trim()).filter(|t| !t.is_empty()),
            req.resource_address.as_ref(),
            offset,
            limit,
        )
        .await
        .map_err(|e| {
            error!(target: LOG_TARGET, "DB error when fetching events: {}", e);
            ErrorResponse::internal_error("DB error when fetching events")
        })?;

    debug!(target: LOG_TARGET, "Found {} events", events.len());
    Ok(Json(QueryTransactionEventsResponse { events }))
}
