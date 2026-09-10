//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{str::FromStr, sync::Arc};

use axum::{
    Extension,
    Router,
    http::{HeaderValue, Response, StatusCode, Uri, header},
    response::IntoResponse,
    routing::get,
};
use include_dir::{Dir, include_dir};
use log::*;
use tari_state_store_rocksdb::column_families;
use tower_http::cors::CorsLayer;

use super::{LOG_TARGET, context::HandlerContext, handlers, handlers::slugify_type_name};

macro_rules! add_cf_route {
    ($api:expr, $cf:expr) => {
        let slug = slugify_type_name($cf);
        $api = $api.route(
            &format!("/databases/{{db_name}}/column-families/{slug}"),
            get(handlers::tables::list(|| $cf)),
        );
    };
}
macro_rules! add_cf_routes {
    ($api:expr, $($cf:expr),+) => {
        $(add_cf_route!($api, $cf);)+
    };
}

pub async fn run(context: HandlerContext) -> anyhow::Result<()> {
    let bind_address = context.config().webserver.bind_address;

    let mut api = Router::new()
        .route("/databases", get(handlers::databases::list))
        .route(
            "/databases/{db_name}/column-families",
            get(handlers::column_families::list),
        )
        // Special cases
        .route(
            &format!("/databases/{{db_name}}/column-families/{}", slugify_type_name(column_families::block::BlockCf)),
            get(handlers::blocks::list),
        )
        .route(
            &format!("/databases/{{db_name}}/column-families/{}", slugify_type_name(column_families::state_transition::StateTransitionCf)),
            get(handlers::state_transitions::list),
        )
        .route(
            &format!("/databases/{{db_name}}/column-families/{}", slugify_type_name(column_families::block_diff::BlockDiffCf)),
            get(handlers::block_diff::list),
        )
        .route(
            &format!("/databases/{{db_name}}/column-families/{}", slugify_type_name(column_families::block_diff::SubstateIdIndex)),
            get(handlers::block_diff_substate_id_index::list),
        )
        .route(
            "/databases/{db_name}/column-families/bookkeeping",
            get(handlers::bookkeeping::list),
        )
        .route(
            &format!("/databases/{{db_name}}/column-families/{}", slugify_type_name(column_families::state_tree::StateTreeCf)),
            get(handlers::state_tree::list),
        )
        .route(
            &format!("/databases/{{db_name}}/column-families/{}", slugify_type_name(column_families::foreign_substate_pledge::ForeignSubstatePledgeCf)),
            get(handlers::foreign_substate_pledges::list),
        );

    add_cf_routes!(
        api,
        column_families::chain::PendingChainIndex,
        column_families::chain::CommittedParentChildChainIndex,
        column_families::chain::PendingParentChildIndex,
        column_families::foreign_proposal::ForeignProposalCf,
        column_families::foreign_proposal::ProposedInBlockIndex,
        column_families::foreign_proposal::EpochIndex,
        column_families::foreign_proposal::UnconfirmedIndex,
        column_families::block::EpochHeightIndex,
        // column_families::block_diff::BlockDiffModel,
        // column_families::block_diff::SubstateIdIndex,
        column_families::certificates::proposal::ProposalCertificateCf,
        column_families::certificates::timeout::TimeoutCertificateCf,
        column_families::block_transaction_execution::BlockTransactionExecutionCf,
        column_families::block_transaction_execution::BlockIndex,
        column_families::finalized_transaction::FinalizedTransactionLinkCf,
        column_families::transaction::TransactionCf,
        column_families::transaction_pool::TransactionPoolCf,
        column_families::transaction_pool_state_update::TransactionPoolStateUpdateCf,
        column_families::transaction_pool_state_update::TransactionPoolStateUpdateDebugHistoryCf,
        column_families::missing_transactions::MissingTransactionCf,
        column_families::parked_block::ParkedBlockCf,
        column_families::foreign_parked_blocks::ForeignParkedBlockCf,
        column_families::foreign_parked_blocks::MissingTransactionsModel,
        column_families::substate_locks::SubstateLockModel,
        column_families::substate_locks::HeadIndex,
        column_families::substate_locks::BlockIdIndex,
        column_families::substate_locks::SubstateIdIndex,
        column_families::substate::SubstateCf,
        column_families::substate::HeadIndex,
        column_families::substate::UnprunedDownedValuesIndex,
        // column_families::state_transition::StateTransitionModel,
        // column_families::foreign_substate_pledge::ForeignSubstatePledgeModel,
        column_families::pending_state_tree_diff::PendingStateTreeDiffCf,
        // column_families::state_tree::StateTreeCf,
        column_families::state_tree::StateTreeStaleNodesCf,
        column_families::state_tree_shard_versions::StateTreeShardVersionCf,
        column_families::epoch_checkpoint::EpochCheckpointCf,
        column_families::lock_conflict::LockConflictCf,
        column_families::lock_conflict::LockConflictBlockIdIndex,
        column_families::validator_node_epoch_stats::ValidatorNodeEpochStatsCf,
        column_families::diagnostic_no_vote::DiagnosticsNoVoteCf
    );

    let api = api.fallback(handlers::not_found);

    let router = Router::new()
        .nest("/api", api)
        .fallback(fallback_handler)
        .layer(CorsLayer::permissive())
        .layer(Extension(Arc::new(context)));

    let listener = match tokio::net::TcpListener::bind(bind_address).await {
        Ok(l) => l,
        Err(e) => {
            warn!(
                target: LOG_TARGET,
                "🕸️ Failed to bind on preferred address {} ({e}). Trying OS-assigned", bind_address
            );
            tokio::net::TcpListener::bind(("127.0.0.1", 0u16)).await?
        },
    };
    let server = axum::serve(listener, router);
    info!(target: LOG_TARGET, "🕸️ Webserver listening on http://{}", server.local_addr()?);
    server.await?;

    Ok(())
}

static WEB_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web_ui/dist");

async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path();

    // If path starts with /, strip it.
    let path = path.strip_prefix('/').unwrap_or(path);

    // If the path is a file, return it. Otherwise, use index.html (SPA)
    if let Some(body) = WEB_DIR
        .get_file(path)
        .or_else(|| WEB_DIR.get_file("index.html"))
        .and_then(|file| file.contents_utf8())
    {
        let mime_type = mime_guess::from_path(path).first_or_else(|| mime_guess::Mime::from_str("text/html").unwrap());
        return Response::builder()
            .header(header::CONTENT_TYPE, HeaderValue::from_str(mime_type.as_ref()).unwrap())
            .status(StatusCode::OK)
            .body(body.to_owned())
            .unwrap();
    }
    log::warn!(target: LOG_TARGET, "Not found {:?}", path);
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(String::new())
        .unwrap()
}

pub fn register_all_cfs(context: &mut HandlerContext) -> &mut HandlerContext {
    context
        .register_cf(column_families::block::BlockCf)
        .register_cf(column_families::block::EpochHeightIndex)
        .register_cf(column_families::block_diff::BlockDiffCf)
        .register_cf(column_families::block_diff::SubstateIdIndex)
        .register_cf(column_families::block_transaction_execution::BlockIndex)
        .register_cf(column_families::block_transaction_execution::BlockTransactionExecutionCf)
        .register_cf(column_families::certificates::proposal::ProposalCertificateCf)
        .register_cf(column_families::certificates::timeout::TimeoutCertificateCf)
        .register_cf(column_families::chain::CommittedParentChildChainIndex)
        .register_cf(column_families::chain::PendingChainIndex)
        .register_cf(column_families::chain::PendingParentChildIndex)
        .register_cf(column_families::diagnostic_no_vote::DiagnosticsNoVoteCf)
        .register_cf(column_families::epoch_checkpoint::EpochCheckpointCf)
        .register_cf(column_families::finalized_transaction::FinalizedTransactionLinkCf)
        .register_cf(column_families::foreign_parked_blocks::ForeignParkedBlockCf)
        .register_cf(column_families::foreign_parked_blocks::MissingTransactionsModel)
        .register_cf(column_families::foreign_proposal::ForeignProposalCf)
        .register_cf(column_families::foreign_proposal::EpochIndex)
        .register_cf(column_families::foreign_proposal::ProposedInBlockIndex)
        .register_cf(column_families::foreign_proposal::UnconfirmedIndex)
        .register_cf(column_families::foreign_substate_pledge::ForeignSubstatePledgeCf)
        .register_cf(column_families::lock_conflict::LockConflictBlockIdIndex)
        .register_cf(column_families::lock_conflict::LockConflictCf)
        .register_cf(column_families::missing_transactions::MissingTransactionCf)
        .register_cf(column_families::parked_block::ParkedBlockCf)
        .register_cf(column_families::pending_state_tree_diff::PendingStateTreeDiffCf)
        .register_cf(column_families::state_transition::StateTransitionCf)
        .register_cf(column_families::state_tree::StateTreeCf)
        .register_cf(column_families::state_tree::StateTreeStaleNodesCf)
        .register_cf(column_families::state_tree_shard_versions::StateTreeShardVersionCf)
        .register_cf(column_families::substate::HeadIndex)
        .register_cf(column_families::substate::SubstateCf)
        .register_cf(column_families::substate::UnprunedDownedValuesIndex)
        .register_cf(column_families::substate_locks::BlockIdIndex)
        .register_cf(column_families::substate_locks::HeadIndex)
        .register_cf(column_families::substate_locks::SubstateIdIndex)
        .register_cf(column_families::substate_locks::SubstateLockModel)
        .register_cf(column_families::transaction::TransactionCf)
        .register_cf(column_families::transaction_pool::TransactionPoolCf)
        .register_cf(column_families::transaction_pool_state_update::TransactionPoolStateUpdateCf)
        .register_cf(column_families::transaction_pool_state_update::TransactionPoolStateUpdateDebugHistoryCf)
        .register_cf(column_families::validator_node_epoch_stats::ValidatorNodeEpochStatsCf)
}
