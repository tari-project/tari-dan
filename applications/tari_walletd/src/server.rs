//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Extension},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::{TypedHeader, extract::CookieJar, headers, headers::authorization::Bearer};
use axum_jrpc::{
    JsonRpcAnswer,
    JsonRpcExtractor,
    JsonRpcResponse,
    error::{JsonRpcError, JsonRpcErrorReason},
};
use log::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use tari_ootle_app_utilities::{
    allowed_hosts::{AllowedHosts, check_host},
    tcp::try_bind_with_fallback,
};
use tokio::task;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use super::handlers::{HandlerContext, HandlerWithCookie, auth, stealth_utxos, substates, templates, wallet, webauthn};
use crate::{
    config::WalletDaemonConfig,
    handlers::{
        Handler,
        accounts,
        address_book,
        auth::jwt::AuthError,
        burn_proofs,
        confidential,
        error::HandlerError,
        keys,
        nfts,
        resources,
        settings,
        signing_requests,
        swap_pools,
        transaction,
        transaction_requests,
        validator,
        validator_set,
        webrtc,
    },
};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::json_rpc";

pub async fn spawn_listener(
    preferred_address: SocketAddr,
    signaling_server_address: SocketAddr,
    context: HandlerContext,
) -> anyhow::Result<(SocketAddr, task::JoinHandle<anyhow::Result<()>>)> {
    let shutdown_signal = context.shutdown_signal().clone();
    let router = router(preferred_address, signaling_server_address, context);

    let listener = try_bind_with_fallback(preferred_address).await?;
    let server = axum::serve(listener, router);
    let listen_addr = server.local_addr()?;
    info!(target: LOG_TARGET, "🌐 JSON-RPC listening on {listen_addr}");
    #[cfg(feature = "web_ui")]
    info!(target: LOG_TARGET, "🕸️ Web interface available at http://{listen_addr}");
    let server = server.with_graceful_shutdown(shutdown_signal);
    let task = tokio::spawn(async move {
        server.await?;
        info!(target: LOG_TARGET, "💤 Stopping RPC");
        Ok(())
    });

    Ok((listen_addr, task))
}

fn router(preferred_address: SocketAddr, signaling_server_address: SocketAddr, context: HandlerContext) -> Router {
    let enable_permissive_cors = context.config().enable_permissive_cors;
    let allowed_hosts = allowed_hosts(context.config());
    let router = Router::new()
        .route("/json_rpc", post(handler))
        .route("/json-rpc", post(handler))
        .route("/health", get(health))
        // TODO: Get these traces to work
        .layer(TraceLayer::new_for_http())
        .layer(Extension(Arc::new(context)))
        .layer(Extension((preferred_address,signaling_server_address)))
        .layer(if enable_permissive_cors { CorsLayer::permissive() } else { CorsLayer::new() })
        // Limit the body size to 5MB to allow for wasm uploads
        .layer(DefaultBodyLimit::max(5*1024*1024));

    #[cfg(feature = "web_ui")]
    let router = router.fallback(crate::handlers::web_ui::handler);
    #[cfg(not(feature = "web_ui"))]
    let router = router.fallback(crate::handlers::web_ui::feature_disabled_handler);

    // Layered last so that it guards the fallback as well as the routes.
    router.layer(middleware::from_fn_with_state(allowed_hosts, check_host))
}

/// Liveness probe for container health checks. Needs no authentication. The listener only starts once the wallet has
/// opened its database and loaded its cipher seed, so a response means the daemon is up.
async fn health() -> StatusCode {
    StatusCode::OK
}

/// The names listed in `allowed_hosts` and the host of the WebAuthn relying-party origin.
fn allowed_hosts(config: &WalletDaemonConfig) -> AllowedHosts {
    let rp_host = config.webauthn.rp_origin.as_ref().and_then(|origin| origin.host_str());
    AllowedHosts::new(
        "ootle_wallet_daemon.allowed_hosts",
        config.allowed_hosts.iter().map(String::as_str).chain(rp_host),
    )
}

/// Renders the part of a JSON-RPC request that may be written to a log: the request id, the method,
/// and the number of parameters.
///
/// Parameter *values* must never reach a log record. `keys.import` carries a raw spending key, and
/// the shipped log config routes this target at `debug` level to a rolling file, to `ootle.log` and
/// to stdout, so anything formatted here outlives the process and is collected by log shippers and
/// attached to bug reports. `JsonRpcExtractor`'s derived `Debug` prints its `parsed` field, which is
/// the whole params value, so the extractor itself must never be formatted into a record.
fn describe_request(request: &JsonRpcExtractor) -> String {
    let params = match &request.parsed {
        serde_json::Value::Null => "none".to_string(),
        serde_json::Value::Object(fields) => format!("{} named", fields.len()),
        serde_json::Value::Array(args) => format!("{} positional", args.len()),
        _ => "1 scalar".to_string(),
    };
    format!("id={:?} method={} params={params}", request.id, request.method)
}

#[allow(clippy::too_many_lines)]
async fn handler(
    Extension(context): Extension<Arc<HandlerContext>>,
    Extension(addresses): Extension<(SocketAddr, SocketAddr)>,
    authorization_header: Option<TypedHeader<headers::Authorization<Bearer>>>,
    cookie_jar: CookieJar,
    value: JsonRpcExtractor,
) -> Response {
    let token = authorization_header.map(|auth| auth.0.0);
    info!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);
    debug!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", describe_request(&value));
    match value.method.as_str().split_once('.') {
        Some(("auth", method)) => match method {
            "request" => call_handler_any_response(context, value, token, auth::handle_login_request).await,
            "refresh" => {
                call_handler_with_cookies(
                    context,
                    value,
                    token,
                    Some(cookie_jar),
                    HandlerWithCookie::new(auth::handle_token_refresh),
                )
                .await
            },
            "revoke" => call_handler(context, value, token, auth::handle_revoke).await,
            "list_sessions" => call_handler(context, value, token, auth::handle_list_sessions).await,
            "method" => call_handler(context, value, token, auth::handle_get_auth_method).await,
            // API key management (issue #1957). All three require Admin and
            // are gated by `check_auth` inside their handlers.
            "create_api_key" => call_handler(context, value, token, auth::handle_create_api_key).await,
            "list_api_keys" => call_handler(context, value, token, auth::handle_list_api_keys).await,
            "revoke_api_key" => call_handler(context, value, token, auth::handle_revoke_api_key).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("webauthn", method)) => match method {
            "already_registered" => call_handler(context, value, token, webauthn::handle_already_registered).await,
            "reg_start" => call_handler(context, value, token, webauthn::handle_start_registration).await,
            "reg_finish" => {
                call_handler_any_response(context, value, token, webauthn::handle_finish_registration).await
            },
            "auth_start" => call_handler(context, value, token, webauthn::handle_start_auth).await,
            "list_passkeys" => call_handler(context, value, token, webauthn::handle_list_passkeys).await,
            "rename_passkey" => call_handler(context, value, token, webauthn::handle_rename_passkey).await,
            "remove_passkey" => call_handler(context, value, token, webauthn::handle_remove_passkey).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("settings", method)) => match method {
            "get" => call_handler(context, value, token, settings::handle_get).await,
            "set" => call_handler(context, value, token, settings::handle_set).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("webrtc", "start")) => webrtc::handle_start(context, value, token.as_ref(), addresses).into_response(),
        Some(("keys", method)) => match method {
            "create" => call_handler(context, value, token, keys::handle_create).await,
            "import" => call_handler(context, value, token, keys::handle_import).await,
            "list" => call_handler(context, value, token, keys::handle_list).await,
            "set_active" => call_handler(context, value, token, keys::handle_set_active).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("transactions", method)) => match method {
            "submit_instruction" => call_handler(context, value, token, transaction::handle_submit_instruction).await,
            "submit" => call_handler(context, value, token, transaction::handle_submit).await,
            "submit_dry_run" => call_handler(context, value, token, transaction::handle_submit_dry_run).await,
            "detect_inputs" => call_handler(context, value, token, transaction::handle_detect_inputs).await,
            "submit_manifest" => call_handler(context, value, token, transaction::handle_submit_manifest).await,
            "publish_template" => call_handler(context, value, token, transaction::handle_publish_template).await,
            "get" => call_handler(context, value, token, transaction::handle_get).await,
            "get_result" => call_handler(context, value, token, transaction::handle_get_result).await,
            "wait_result" => call_handler(context, value, token, transaction::handle_wait_result).await,
            "list" => call_handler(context, value, token, transaction::handle_list).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("transaction_requests", method)) => match method {
            "create" => call_handler(context, value, token, transaction_requests::handle_create).await,
            "get" => call_handler(context, value, token, transaction_requests::handle_get).await,
            "list" => call_handler(context, value, token, transaction_requests::handle_list).await,
            "approve" => call_handler(context, value, token, transaction_requests::handle_approve).await,
            "reject" => call_handler(context, value, token, transaction_requests::handle_reject).await,
            "submit" => call_handler(context, value, token, transaction_requests::handle_submit).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("signing_requests", method)) => match method {
            "create" => call_handler(context, value, token, signing_requests::handle_create).await,
            "get" => call_handler(context, value, token, signing_requests::handle_get).await,
            "list" => call_handler(context, value, token, signing_requests::handle_list).await,
            "approve" => call_handler(context, value, token, signing_requests::handle_approve).await,
            "reject" => call_handler(context, value, token, signing_requests::handle_reject).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("accounts", method)) => match method {
            "claim_burn" => call_handler(context, value, token, accounts::handle_claim_burn).await,
            "create" => call_handler(context, value, token, accounts::handle_create).await,
            "create_or_get" => call_handler(context, value, token, accounts::handle_create_or_get).await,
            "import_view_only" => call_handler(context, value, token, accounts::handle_import_view_only).await,
            "export_view_key" => call_handler(context, value, token, accounts::handle_export_view_key).await,
            "list" => call_handler(context, value, token, accounts::handle_list).await,
            "get_balance_changes" => call_handler(context, value, token, accounts::handle_get_balance_changes).await,
            "get_balances" => call_handler(context, value, token, accounts::handle_get_balances).await,
            "get" => call_handler(context, value, token, accounts::handle_get).await,
            "get_by_key_index" => call_handler(context, value, token, accounts::handle_get_by_key_index).await,
            "get_default" => call_handler(context, value, token, accounts::handle_get_default).await,
            "transfer" => call_handler(context, value, token, accounts::handle_transfer).await,
            "confidential_transfer" => {
                call_handler(context, value, token, accounts::handle_confidential_transfer).await
            },
            "associate_stealth_resource" => {
                call_handler(context, value, token, accounts::handle_associate_stealth_resource).await
            },
            "stealth_transfer" => call_handler(context, value, token, accounts::handle_stealth_transfer).await,
            "create_stealth_transfer_statement" => {
                call_handler(
                    context,
                    value,
                    token,
                    accounts::handle_create_stealth_transfer_statement,
                )
                .await
            },
            "set_default" => call_handler(context, value, token, accounts::handle_set_default).await,
            "rename" => call_handler(context, value, token, accounts::handle_rename).await,
            "create_free_test_coins" => {
                call_handler(context, value, token, accounts::handle_create_free_test_coins).await
            },
            "get_faucet_balance" => call_handler(context, value, token, accounts::handle_get_faucet_balance).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("confidential", method)) => match method {
            "create_transfer_proof" => {
                call_handler(context, value, token, confidential::handle_create_transfer_proof).await
            },
            "finalize" => call_handler(context, value, token, confidential::handle_finalize_transfer).await,
            "cancel" => call_handler(context, value, token, confidential::handle_cancel_transfer).await,
            "create_output_proof" => {
                call_handler(context, value, token, confidential::handle_create_output_proof).await
            },
            "view_vault_balance" => call_handler(context, value, token, confidential::handle_view_vault_balance).await,
            "list_outputs" => call_handler(context, value, token, confidential::handle_list_outputs).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("substates", method)) => match method {
            "get" => call_handler(context, value, token, substates::handle_get).await,
            "list" => call_handler(context, value, token, substates::handle_list).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("templates", method)) => match method {
            "get" => call_handler(context, value, token, templates::handle_get).await,
            "list_authored" => call_handler(context, value, token, templates::handle_list_owned).await,
            "sign_metadata" => call_handler(context, value, token, templates::handle_sign_metadata).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("nfts", method)) => match method {
            "mint_faucet_nft" => call_handler(context, value, token, nfts::handle_mint_faucet).await,
            "get" => call_handler(context, value, token, nfts::handle_get).await,
            "list" => call_handler(context, value, token, nfts::handle_list).await,
            "transfer" => call_handler(context, value, token, nfts::handle_transfer).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("validators", method)) => match method {
            "get_fees" => call_handler(context, value, token, validator::handle_get_validator_fees).await,
            "claim_fees" => call_handler(context, value, token, validator::handle_claim_validator_fees).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("stealth_utxos", method)) => match method {
            "list" => call_handler(context, value, token, stealth_utxos::handle_list).await,
            "decrypt_value" => call_handler(context, value, token, stealth_utxos::handle_decrypt_value).await,
            "get_value_lookup_info" => {
                call_handler(context, value, token, stealth_utxos::handle_get_value_lookup_info).await
            },
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("burn_proofs", method)) => match method {
            "list" => call_handler(context, value, token, burn_proofs::handle_list).await,
            "get" => call_handler(context, value, token, burn_proofs::handle_get).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("swap_pools", method)) => match method {
            "get_exchange_rate" => call_handler(context, value, token, swap_pools::handle_get_exchange_rate).await,
            "list" => call_handler(context, value, token, swap_pools::handle_list).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("resources", method)) => match method {
            "get" => call_handler(context, value, token, resources::handle_get).await,
            "get_many" => call_handler(context, value, token, resources::handle_get_many).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("address_book", method)) => match method {
            "add" => call_handler(context, value, token, address_book::handle_add).await,
            "list" => call_handler(context, value, token, address_book::handle_list).await,
            "get" => call_handler(context, value, token, address_book::handle_get).await,
            "update" => call_handler(context, value, token, address_book::handle_update).await,
            "delete" => call_handler(context, value, token, address_book::handle_delete).await,
            _ => value.method_not_found(&value.method).into_response(),
        },
        Some(("wallet", "get_info")) => call_handler(context, value, token, wallet::handle_get_info).await,
        Some(("validator_set", "get_status")) => {
            call_handler(context, value, token, validator_set::handle_get_status).await
        },
        _ => value.method_not_found(&value.method).into_response(),
    }
}

async fn call_handler_with_cookies<H, TReq, TResp>(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<Bearer>,
    cookies: Option<CookieJar>,
    mut handler: H,
) -> Response
where
    TReq: DeserializeOwned,
    TResp: Serialize,
    H: for<'a> Handler<'a, TReq, Response = TResp>,
{
    let answer_id = value.get_answer_id();
    match value.parse_params().inspect_err(|e| {
        if let JsonRpcAnswer::Error(e) = &e.result {
            warn!(target: LOG_TARGET, "🌐 JSON-RPC params error: {}", e);
        }
    }) {
        Ok(req) => handler
            .handle(&context, token.as_ref(), cookies, req)
            .await
            .map_err(|e| resolve_handler_error(answer_id.clone(), &e))
            .map(|resp| JsonRpcResponse::success(answer_id.clone(), resp))
            .into_response(),
        Err(e) => Err::<(), _>(e).into_response(),
    }
}

async fn call_handler_any_response<H, TReq, TResp>(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<Bearer>,
    mut handler: H,
) -> Response
where
    TReq: DeserializeOwned,
    TResp: IntoResponse,
    H: for<'a> Handler<'a, (axum_jrpc::Id, TReq), Response = TResp>,
{
    let answer_id = value.get_answer_id();
    match value.parse_params().inspect_err(|e| {
        if let JsonRpcAnswer::Error(e) = &e.result {
            warn!(target: LOG_TARGET, "🌐 JSON-RPC params error: {}", e);
        }
    }) {
        Ok(req) => handler
            .handle(&context, token.as_ref(), None, (answer_id.clone(), req))
            .await
            .map_err(|e| resolve_handler_error(answer_id.clone(), &e))
            .into_response(),
        Err(e) => Err::<(), _>(e).into_response(),
    }
}

async fn call_handler<H, TReq, TResp>(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<Bearer>,
    handler: H,
) -> Response
where
    TReq: DeserializeOwned,
    TResp: Serialize,
    H: for<'a> Handler<'a, TReq, Response = TResp>,
{
    call_handler_with_cookies(context, value, token, None, handler).await
}

pub fn resolve_handler_error(answer_id: axum_jrpc::Id, e: &HandlerError) -> JsonRpcResponse {
    match e {
        HandlerError::Anyhow(e) => resolve_any_error(answer_id, e),
    }
}

/// True for an error already carrying the not-found code, whatever built it.
fn is_not_found(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<JsonRpcError>().map(|e| e.error_reason()),
        Some(JsonRpcErrorReason::ApplicationError(code)) if code == ApplicationErrorCode::NotFound as i32
    )
}

fn resolve_any_error(answer_id: axum_jrpc::Id, e: &anyhow::Error) -> JsonRpcResponse {
    // Asking about something that is not there is a normal thing for a caller to do, and one that
    // polls - `transactions.wait_result` against an id the wallet does not know - would otherwise
    // warn once per poll. Everything else stays a warning.
    if is_not_found(e) {
        debug!(target: LOG_TARGET, "🌐 JSON-RPC not found: {}", e);
    } else {
        warn!(target: LOG_TARGET, "🌐 JSON-RPC error: {}", e);
    }
    if let Some(handler_err) = e.downcast_ref::<HandlerError>() {
        return resolve_handler_error(answer_id, handler_err);
    }

    if let Some(error) = e.downcast_ref::<JsonRpcError>() {
        return JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(error.error_reason(), error.to_string(), serde_json::Value::Null),
        );
    }

    if let Some(error) = e.downcast_ref::<AuthError>() {
        JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(ApplicationErrorCode::Unauthorized as i32),
                error.to_string(),
                serde_json::Value::Null,
            ),
        )
    } else {
        JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(ApplicationErrorCode::GeneralError as i32),
                e.to_string(),
                json!({}),
            ),
        )
    }
}

#[repr(i32)]
pub enum ApplicationErrorCode {
    NotFound = 404,
    InvalidRequest = 400,
    Unauthorized = 401,
    TransactionRejected = 1000,
    FaucetAlreadyClaimed = 1001,
    FaucetEmpty = 1002,
    TransactionPending = 1003,
    GeneralError = 500,
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use axum_jrpc::Id;
    use tari_ootle_walletd_client::{
        permissions::Permission,
        types::{AuthCredentials, AuthLoginRequest},
    };
    use tower::ServiceExt;

    use super::*;
    use crate::handlers::test_support::TestDaemon;

    #[tokio::test]
    async fn health_answers_without_authentication() {
        let daemon = TestDaemon::start().await;
        let address = SocketAddr::from(([127, 0, 0, 1], 5100));
        let router = router(address, SocketAddr::from(([127, 0, 0, 1], 9100)), daemon.context);
        let request = Request::get("/health")
            .header(header::HOST, "127.0.0.1:5100")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        // The web UI fallback also answers 200, with a page; only the health route answers with an empty body.
        assert!(to_bytes(response.into_body(), usize::MAX).await.unwrap().is_empty());
    }

    fn keys_import(secret_key: &str) -> JsonRpcExtractor {
        JsonRpcExtractor {
            parsed: json!({ "key_type": "Ristretto", "secret_key": secret_key }),
            method: "keys.import".to_string(),
            id: Id::Num(7),
        }
    }

    /// The log record must be a function of the request envelope alone. Two requests differing only
    /// in a parameter value therefore describe identically -- which is the property that keeps an
    /// imported spending key out of `json_rpc.log`.
    #[test]
    fn parameter_values_are_not_described() {
        let secret = "a".repeat(64);
        let described = describe_request(&keys_import(&secret));

        assert_eq!(described, describe_request(&keys_import(&"b".repeat(64))));
        assert!(!described.contains(&secret));
    }

    #[test]
    fn the_envelope_is_described() {
        let described = describe_request(&keys_import(&"a".repeat(64)));
        assert!(described.contains("keys.import"));
        assert!(described.contains('7'));
        assert!(described.contains("2 named"));
    }

    /// Sends an anonymous admin `auth.request` to a no-auth daemon's router, addressed to `host`.
    async fn login_as(host: &str) -> Response {
        let daemon = TestDaemon::start().await;
        let address = SocketAddr::from(([127, 0, 0, 1], 5100));
        let router = router(address, SocketAddr::from(([127, 0, 0, 1], 9100)), daemon.context);
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "auth.request",
            "params": AuthLoginRequest {
                permissions: vec![Permission::Admin],
                credentials: AuthCredentials::None,
            },
        });
        let request = Request::post("/json_rpc")
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        router.oneshot(request).await.unwrap()
    }

    async fn issues_token(response: Response) -> bool {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        body["result"]["token"].is_string()
    }

    #[test]
    fn configured_names_and_the_rp_origin_host_are_allowed() {
        let mut config = WalletDaemonConfig::default();
        config.allowed_hosts = vec!["wallet.internal".to_string()];
        config.webauthn.rp_origin = Some("https://wallet.example.com".parse().unwrap());
        let allowed = allowed_hosts(&config);
        assert!(allowed.permits("wallet.internal"));
        assert!(allowed.permits("wallet.example.com"));
        assert!(!allowed.permits("other.example.com"));
    }

    #[tokio::test]
    async fn a_request_addressed_to_a_foreign_host_name_is_refused() {
        let response = login_as("wallet.example.com:5100").await;
        assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST);
        assert!(!issues_token(response).await);
    }

    #[tokio::test]
    async fn a_request_addressed_to_a_local_name_is_served() {
        for host in ["127.0.0.1:5100", "localhost:5100", "[::1]:5100", "localhost"] {
            let response = login_as(host).await;
            assert_eq!(response.status(), StatusCode::OK, "{host}");
            assert!(issues_token(response).await, "{host}");
        }
    }
}
