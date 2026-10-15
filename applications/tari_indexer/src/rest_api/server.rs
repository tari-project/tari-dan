//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::net::SocketAddr;

use axum::{
    Extension,
    Router,
    middleware,
    routing::{get, post},
};
use log::*;
use tari_ootle_app_utilities::tcp::try_bind_with_fallback;
use tari_shutdown::ShutdownSignal;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[cfg(feature = "metrics")]
use crate::rest_api::metrics;
use crate::{
    bootstrap::Services,
    config::IndexerRateLimitsConfig,
    rest_api::{
        cache::default_no_store,
        context::HandlerContext,
        handlers,
        rate_limit::{
            IpRateLimiter,
            RateLimitConfig,
            SseConnectionLimiter,
            SseConnectionMetrics,
            SseLimitConfig,
            TrustedProxyHeaders,
            rate_limit_middleware,
            sse_limit_middleware,
        },
    },
};

const LOG_TARGET: &str = "tari::ootle::indexer::rest_api::server";

// Limit the body size to 4MB to allow for large transactions (wasm uploads)
const REQUEST_BODY_LIMIT: usize = 4 * 1024 * 1024; // 4 MB

#[derive(OpenApi)]
#[openapi(paths(
    handlers::misc::get_identity,
    handlers::misc::get_info,
    handlers::misc::wait_until_ready,
    handlers::misc::get_epoch_manager_stats,
    handlers::network::get,
    handlers::network::get_economics,
    handlers::network::get_network_sync_stats,
    handlers::network::get_connections,
    handlers::substates::get_substate,
    handlers::substates::fetch_substates,
    handlers::resources::get_tari,
    handlers::resources::get_resource,
    handlers::indexer_events::sse_events,
    handlers::transactions::submit_transaction,
    handlers::transactions::submit_transaction_dry_run,
    handlers::transactions::list_recent_transactions,
    handlers::transactions::get_transaction,
    handlers::transactions::get_transaction_result,
    handlers::templates::get_template_definition,
    handlers::templates::list_template_catalogue,
    handlers::templates::get_template_catalogue_entry,
    handlers::utxos::fetch_utxos,
    handlers::utxos::list_utxos,
    handlers::utxos::stream_utxo_updates,
    handlers::transaction_receipts::list_transaction_receipts,
    handlers::transaction_receipts::get_transaction_receipt,
    handlers::transaction_events::sse_transaction_events,
    handlers::epoch_checkpoints::list_epoch_checkpoints,
    handlers::epoch_checkpoints::get_latest_epoch_checkpoint,
    handlers::validators::list_validators,
    handlers::committees::get_committee
))]
pub struct ApiDoc;

#[derive(Debug, Clone, Copy, Default)]
pub struct Server;

impl Server {
    pub fn new() -> Self {
        Self
    }

    pub async fn spawn(
        self,
        preferred_addr: SocketAddr,
        services: &Services,
        rate_limits: &IndexerRateLimitsConfig,
        #[cfg(feature = "metrics")] registry: &mut prometheus_client::registry::Registry,
        shutdown: ShutdownSignal,
    ) -> anyhow::Result<SocketAddr> {
        let context = HandlerContext::from_services(services);

        #[cfg(feature = "metrics")]
        let api_metrics = metrics::register(registry);
        #[cfg(feature = "metrics")]
        let sse_metrics = api_metrics.sse.clone();
        #[cfg(not(feature = "metrics"))]
        let sse_metrics = SseConnectionMetrics::default();

        let router = api_routes(rate_limits, &sse_metrics)
            .layer(CorsLayer::permissive())
            .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT))
            .merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", ApiDoc::openapi()))
            .layer(Extension(context))
            .layer(TraceLayer::new_for_http());

        // Note: `/_metrics` is deliberately not served here — it runs on its own listener bound to
        // `metrics_listen_address` (see `metrics::spawn_metrics_server`), so internal operational
        // metrics are not exposed on the public REST API.
        #[cfg(feature = "metrics")]
        let router = router.layer(axum::middleware::from_fn_with_state(
            api_metrics.requests,
            metrics::layer,
        ));

        let listener = try_bind_with_fallback(preferred_addr).await?;

        // spawn server
        let listen_addr = listener.local_addr()?;
        info!(target: LOG_TARGET, "🌐 Indexer REST API server listening on {listen_addr}");
        tokio::spawn(async move {
            // `into_make_service_with_connect_info` populates `ConnectInfo<SocketAddr>`
            // for each connection, which the per-IP rate limiter middleware uses to
            // identify the remote peer when no proxy headers are present.
            if let Err(error) = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(shutdown)
                .await
            {
                error!(target: LOG_TARGET, "Wallet query HTTP server error: {error}");
            }
        });

        Ok(listen_addr)
    }
}

/// The API routes and their per-IP limits. Every layer that needs the running node is added by
/// [`Server::spawn`].
#[expect(clippy::too_many_lines)]
fn api_routes(rate_limits: &IndexerRateLimitsConfig, sse_metrics: &SseConnectionMetrics) -> Router {
    // Per-IP rate limiters for specific endpoint groups.
    let trusted_headers = TrustedProxyHeaders {
        forwarded_for: rate_limits.trust_proxy_headers,
        cf_connecting_ip: rate_limits.trust_cf_connecting_ip,
    };
    let tx_submit_limiter = RateLimitConfig {
        enabled: rate_limits.enabled,
        limiter: IpRateLimiter::new(rate_limits.transactions_submit_rate),
        trusted_headers,
        abandoned_request_cost: 0.0,
    };
    let tx_dry_run_limiter = RateLimitConfig {
        enabled: rate_limits.enabled,
        limiter: IpRateLimiter::new(rate_limits.transactions_dry_run_submit_rate),
        trusted_headers,
        abandoned_request_cost: handlers::transactions::DRY_RUN_MAX_COST,
    };
    let transactions_fetch_limiter = RateLimitConfig {
        enabled: rate_limits.enabled,
        limiter: IpRateLimiter::new(rate_limits.transactions_rate),
        trusted_headers,
        abandoned_request_cost: 0.0,
    };
    let substates_fetch_limiter = RateLimitConfig {
        enabled: rate_limits.enabled,
        limiter: IpRateLimiter::new(rate_limits.substates_rate),
        trusted_headers,
        abandoned_request_cost: 0.0,
    };
    let utxos_fetch_limiter = RateLimitConfig {
        enabled: rate_limits.enabled,
        limiter: IpRateLimiter::new(rate_limits.utxos_fetch_rate),
        trusted_headers,
        abandoned_request_cost: 0.0,
    };
    let epoch_checkpoints_limiter = RateLimitConfig {
        enabled: rate_limits.enabled,
        limiter: IpRateLimiter::new(rate_limits.epoch_checkpoints_rate),
        trusted_headers,
        abandoned_request_cost: 0.0,
    };
    let reads_limiter = RateLimitConfig {
        enabled: rate_limits.enabled,
        limiter: IpRateLimiter::new(rate_limits.reads_rate),
        trusted_headers,
        abandoned_request_cost: 0.0,
    };
    // The streaming routes share one per-IP limiter but are counted separately, so each
    // takes its own config carrying that route's gauge handle.
    let sse_connections = SseConnectionLimiter::new(rate_limits.sse_max_connections_per_ip);
    let sse_limiter = |endpoint: &'static str| SseLimitConfig {
        enabled: rate_limits.enabled,
        limiter: sse_connections.clone(),
        trusted_headers,
        active_connections: sse_metrics.endpoint(endpoint),
    };

    Router::new()
            // ----------------------------------------------------------------
            // Unrestricted endpoints (health / ready / identity)
            // ----------------------------------------------------------------
            .route("/health", get(handlers::misc::health))
            .route("/ready", get(handlers::misc::ready))
            .route("/identity", get(handlers::misc::get_identity))
            .route("/info", get(handlers::misc::get_info))
            .route("/wait-until-ready", get(handlers::misc::wait_until_ready))
            .route("/epoch-manager/stats", get(handlers::misc::get_epoch_manager_stats))
            .route("/network", get(handlers::network::get))
            .route("/network/economics", get(handlers::network::get_economics))
            .route("/network/stats", get(handlers::network::get_network_sync_stats))
            .route("/network/connections", get(handlers::network::get_connections))
            // GET /validators – per-IP rate limit (rate_limits.reads_rate)
            .route("/validators", get(handlers::validators::list_validators)
                .route_layer(middleware::from_fn_with_state(reads_limiter.clone(), rate_limit_middleware)))
            // GET /committees – per-IP rate limit (rate_limits.reads_rate)
            .route("/committees", get(handlers::committees::get_committee)
                .route_layer(middleware::from_fn_with_state(reads_limiter.clone(), rate_limit_middleware)))
            // ----------------------------------------------------------------
            // /substates/* – per-IP rate limit (rate_limits.substates_rate)
            // ----------------------------------------------------------------
            .nest("/substates", Router::new()
                .route("/fetch", post(handlers::substates::fetch_substates)
                    .route_layer(middleware::from_fn_with_state(substates_fetch_limiter.clone(), rate_limit_middleware)))
                .route("/watched", get(handlers::watched::list_watched_substates)
                    .route_layer(middleware::from_fn_with_state(substates_fetch_limiter.clone(), rate_limit_middleware)))
                .route("/{substate_id}", get(handlers::substates::get_substate)
                .route_layer(middleware::from_fn_with_state(substates_fetch_limiter, rate_limit_middleware)))
            )
            // ----------------------------------------------------------------
            // Transactions
            // ----------------------------------------------------------------
            .nest("/transactions", Router::new()
                // POST /transactions – per-IP rate limit (rate_limits.transactions_submit_rate)
                .route("/", post(handlers::transactions::submit_transaction)
                    .route_layer(middleware::from_fn_with_state(tx_submit_limiter, rate_limit_middleware)))
                // POST /transactions/dry-run – per-IP rate limit on a separate limiter instance so a
                // burst of dry-runs doesn't starve the submit budget for the same IP
                .route("/dry-run", post(handlers::transactions::submit_transaction_dry_run)
                    .route_layer(middleware::from_fn_with_state(tx_dry_run_limiter, rate_limit_middleware))
                )
                // GET /transactions/recent – per-IP rate limit (rate_limits.transactions_rate)
                .route(
                    "/recent",
                    get(handlers::transactions::list_recent_transactions)
                    .route_layer(middleware::from_fn_with_state(transactions_fetch_limiter.clone(), rate_limit_middleware))
                )
                .route(
                    "/{transaction_id}/result",
                    get(handlers::transactions::get_transaction_result)
                    .route_layer(middleware::from_fn_with_state(transactions_fetch_limiter.clone(), rate_limit_middleware))
                )
                .route(
                    "/{transaction_id}",
                    get(handlers::transactions::get_transaction)
                    .route_layer(middleware::from_fn_with_state(transactions_fetch_limiter.clone(), rate_limit_middleware))
                )
                // GET /transactions/events – per-IP rate limit (rate_limits.transactions_rate)
                .route("/events", get(handlers::transactions::query_transaction_events)
                    .route_layer(middleware::from_fn_with_state(transactions_fetch_limiter.clone(), rate_limit_middleware)))
                // SSE stream – per-IP concurrent connection limit
                .route("/events/stream", get(handlers::transaction_events::sse_transaction_events)
                    .route_layer(middleware::from_fn_with_state(sse_limiter("/transactions/events/stream"), sse_limit_middleware)))
            )
            // /templates/* – per-IP rate limit (rate_limits.reads_rate)
            .nest("/templates", Router::new()
                .route("/watched", get(handlers::watched::list_watched_templates))
                .route("/catalogue", get(handlers::templates::list_template_catalogue))
                .route(
                    "/catalogue/{template_address}",
                    get(handlers::templates::get_template_catalogue_entry),
                )
                .route(
                    "/{template_address}",
                    get(handlers::templates::get_template_definition),
                )
                .route_layer(middleware::from_fn_with_state(reads_limiter.clone(), rate_limit_middleware))
            )
            .nest("/utxos", Router::new()
                // GET /utxos – per-IP rate limit (rate_limits.utxos_fetch_rate)
                .route("/", get(handlers::utxos::list_utxos)
                .route_layer(middleware::from_fn_with_state(utxos_fetch_limiter.clone(), rate_limit_middleware)))
                // POST /utxos/fetch – per-IP rate limit (rate_limits.utxos_fetch_rate)
                .route("/fetch", post(handlers::utxos::fetch_utxos)
                    .route_layer(middleware::from_fn_with_state(utxos_fetch_limiter, rate_limit_middleware)))
                // POST /utxos/stream (SSE-like streaming) – per-IP concurrent connection limit
                .route("/stream", post(handlers::utxos::stream_utxo_updates)
                    .route_layer(middleware::from_fn_with_state(sse_limiter("/utxos/stream"), sse_limit_middleware)))
            )
            .nest(
                "/transaction-receipts",
                Router::new()
                    .route("/", get(handlers::transaction_receipts::list_transaction_receipts)
                        .route_layer(middleware::from_fn_with_state(transactions_fetch_limiter.clone(), rate_limit_middleware)))
                    .route(
                        "/{address}",
                        get(handlers::transaction_receipts::get_transaction_receipt)
                           .route_layer(middleware::from_fn_with_state(transactions_fetch_limiter, rate_limit_middleware)))
            )
            // /resources/* – per-IP rate limit (rate_limits.reads_rate)
            .nest("/resources", Router::new()
                // Convenience Shortcut
                .route("/xtr" , get(handlers::resources::get_tari))
                .route("/tari" , get(handlers::resources::get_tari))
                .route("/{resource_address}" , get(handlers::resources::get_resource))
                .route_layer(middleware::from_fn_with_state(reads_limiter, rate_limit_middleware)))
            // /epoch-checkpoints/* – per-IP rate limit (rate_limits.epoch_checkpoints_rate)
            .nest("/epoch-checkpoints", Router::new()
                .route("/", get(handlers::epoch_checkpoints::list_epoch_checkpoints))
                .route("/latest", get(handlers::epoch_checkpoints::get_latest_epoch_checkpoint))
                .route_layer(middleware::from_fn_with_state(epoch_checkpoints_limiter, rate_limit_middleware))
            )
            // SSE /events – per-IP concurrent connection limit
            .route("/events", get(handlers::indexer_events::sse_events)
                .route_layer(middleware::from_fn_with_state(sse_limiter("/events"), sse_limit_middleware)))
            // Wraps the API routes so that a handler which sets no policy still denies caching. Applied
            // before the Swagger UI is merged in, whose static assets are fine to cache normally.
            .layer(middleware::from_fn(default_no_store))
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn sse_metrics() -> SseConnectionMetrics {
        #[cfg(feature = "metrics")]
        return SseConnectionMetrics::register(&mut prometheus_client::registry::Registry::default());
        #[cfg(not(feature = "metrics"))]
        SseConnectionMetrics::default()
    }

    async fn status_of(addr: SocketAddr, path: &str) -> u16 {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        let status_line = response.lines().next().unwrap();
        status_line.split_whitespace().nth(1).unwrap().parse().unwrap()
    }

    /// The handlers fail without a `HandlerContext`, which does not matter here: the limit is
    /// enforced before the handler runs, so only a 429 shows the route is limited.
    #[tokio::test]
    async fn public_read_routes_are_rate_limited() {
        const ATTEMPTS: usize = 1_000;

        let router = api_routes(&IndexerRateLimitsConfig::default(), &sse_metrics());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap();
        });

        for path in [
            "/epoch-checkpoints?limit=100",
            "/epoch-checkpoints/latest",
            "/resources/tari",
            "/templates/catalogue",
            "/validators",
            "/committees?epoch=1&shard=1",
        ] {
            let mut limited = false;
            for _ in 0..ATTEMPTS {
                if status_of(addr, path).await == 429 {
                    limited = true;
                    break;
                }
            }
            assert!(limited, "{path} served {ATTEMPTS} requests from one IP without a 429");
        }
    }

    #[tokio::test]
    async fn non_fungibles_listing_is_not_served() {
        let router = api_routes(&IndexerRateLimitsConfig::default(), &sse_metrics());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap();
        });

        let path = "/non-fungibles?address=resource_00&start_index=0&end_index=10";
        assert_eq!(status_of(addr, path).await, 404);
    }
}
