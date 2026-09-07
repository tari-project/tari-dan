//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{future, net::SocketAddr, sync::Arc, time::Instant};

use axum::{
    Router,
    body::{Body, HttpBody},
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use log::*;
use prometheus_client::{
    encoding::text::encode,
    metrics::{
        counter::Counter,
        gauge::Gauge,
        histogram::{Histogram, exponential_buckets},
    },
    registry::Registry,
};
use tari_ootle_app_utilities::tcp::try_bind_with_fallback;
use tari_shutdown::ShutdownSignal;

use crate::{metrics::CollectorRegister, rest_api::rate_limit::SseConnectionMetrics};

const LOG_TARGET: &str = "tari::ootle::indexer::rest_api::metrics";

const METRICS_CONTENT_TYPE: &str = "application/openmetrics-text;charset=utf-8;version=1.0.0";
#[derive(Debug, Clone)]
pub struct MetricsHandler(Arc<Registry>);

impl MetricsHandler {
    pub fn new(registry: Registry) -> Self {
        Self(Arc::new(registry))
    }
}

impl<S> axum::handler::Handler<(), S> for MetricsHandler {
    type Future = future::Ready<Response>;

    fn call(self, req: Request<Body>, _state: S) -> Self::Future {
        if req.method() != axum::http::Method::GET {
            let mut resp = "Method not allowed. Only GET requests are supported for metrics.".into_response();
            *resp.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
            return future::ready(resp);
        }

        let mut text = String::with_capacity(1024);
        encode(&mut text, &self.0).unwrap();

        future::ready(
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, METRICS_CONTENT_TYPE)],
                text,
            )
                .into_response(),
        )
    }
}

#[derive(Clone)]
pub struct RequestMetrics {
    request_counter: Counter,
    response_time_histogram: Histogram,
    requests_pending: Gauge,
    response_body_size_histogram: Histogram,
}

/// Everything the REST API exports, all of it under the `api` prefix.
#[derive(Clone)]
pub struct ApiMetrics {
    /// State for the [`layer`] middleware wrapping the whole router.
    pub requests: RequestMetrics,
    /// Shared with the SSE limit middleware, which maintains the count.
    pub sse: SseConnectionMetrics,
}

pub fn register(registry: &mut Registry) -> ApiMetrics {
    let registry = registry.sub_registry_with_prefix("api");

    ApiMetrics {
        requests: RequestMetrics {
            request_counter: Counter::default().register_at(
                "http_requests_total",
                "Total number of HTTP requests received",
                registry,
            ),
            response_time_histogram: Histogram::new(
                exponential_buckets(0.001, 2.0, 15), // buckets from 1ms, doubling, 15 buckets
            )
            .register_at("http_response_time_seconds", "HTTP response times in seconds", registry),
            requests_pending: Gauge::default().register_at(
                "http_requests_pending",
                "Number of HTTP requests currently being processed",
                registry,
            ),
            response_body_size_histogram: Histogram::new(
                exponential_buckets(100.0, 2.0, 15), // buckets from 100B, doubling, 15 buckets
            )
            .register_at(
                "http_response_body_size_bytes",
                "HTTP response body sizes in bytes, for responses of known length",
                registry,
            ),
        },
        sse: SseConnectionMetrics::register(registry),
    }
}

/// Serves `GET /_metrics` on its own listener, separate from the REST API, so that metrics can be bound to a
/// private interface (e.g. localhost-only for a Prometheus sidecar) while the REST API remains public.
pub async fn spawn_metrics_server(
    preferred_addr: SocketAddr,
    registry: Registry,
    shutdown: ShutdownSignal,
) -> anyhow::Result<SocketAddr> {
    let router = Router::new().route("/_metrics", get(MetricsHandler::new(registry)));

    let listener = try_bind_with_fallback(preferred_addr).await?;
    let listen_addr = listener.local_addr()?;
    info!(target: LOG_TARGET, "📊 Indexer metrics server listening on {listen_addr}");
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router.into_make_service())
            .with_graceful_shutdown(shutdown)
            .await
        {
            error!(target: LOG_TARGET, "Metrics HTTP server error: {error}");
        }
    });

    Ok(listen_addr)
}

pub async fn layer(State(metrics): State<RequestMetrics>, req: Request<Body>, next: Next) -> Response {
    metrics.request_counter.inc();
    metrics.requests_pending.inc();

    let timer = Instant::now();
    let response = next.run(req).await;
    if let Some(size) = response.size_hint().exact() {
        metrics.response_body_size_histogram.observe(size as f64);
    }
    let elapsed = timer.elapsed().as_secs_f64();
    metrics.response_time_histogram.observe(elapsed);
    metrics.requests_pending.dec();

    response
}

#[cfg(test)]
mod tests {
    use tari_shutdown::Shutdown;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    async fn http_get(addr: SocketAddr, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[tokio::test]
    async fn it_serves_metrics_on_a_dedicated_listener() {
        let shutdown = Shutdown::new();
        let listen_addr = spawn_metrics_server(
            "127.0.0.1:0".parse().unwrap(),
            Registry::default(),
            shutdown.to_signal(),
        )
        .await
        .unwrap();

        let response = http_get(listen_addr, "/_metrics").await;
        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "unexpected response: {response}"
        );
        assert!(
            response.contains(METRICS_CONTENT_TYPE),
            "unexpected response: {response}"
        );
        // The OpenMetrics text exposition always ends with an EOF marker
        assert!(response.contains("# EOF"), "unexpected response: {response}");
    }

    #[tokio::test]
    async fn it_exports_every_registered_metric() {
        let shutdown = Shutdown::new();
        let mut registry = Registry::default();
        let metrics = register(&mut registry);
        metrics.sse.endpoint("/events");

        let listen_addr = spawn_metrics_server("127.0.0.1:0".parse().unwrap(), registry, shutdown.to_signal())
            .await
            .unwrap();
        let response = http_get(listen_addr, "/_metrics").await;

        for name in [
            "api_http_requests_total",
            "api_http_response_time_seconds",
            "api_http_requests_pending",
            "api_http_response_body_size_bytes",
        ] {
            assert!(response.contains(name), "{name} missing from: {response}");
        }
        // A `Family` gets its descriptor written whether or not it has children, so the
        // labelled series is what shows the endpoint handle reaches the exposition.
        assert!(
            response.contains(r#"api_sse_connections_active{endpoint="/events"}"#),
            "sse gauge series missing from: {response}"
        );
    }

    #[tokio::test]
    async fn it_serves_nothing_but_metrics() {
        let shutdown = Shutdown::new();
        let listen_addr = spawn_metrics_server(
            "127.0.0.1:0".parse().unwrap(),
            Registry::default(),
            shutdown.to_signal(),
        )
        .await
        .unwrap();

        let response = http_get(listen_addr, "/transactions").await;
        assert!(response.starts_with("HTTP/1.1 404"), "unexpected response: {response}");
    }
}
