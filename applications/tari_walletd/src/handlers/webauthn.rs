// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use axum_extra::{extract::CookieJar, headers::authorization::Bearer};
use axum_jrpc::JsonRpcResponse;
use tari_ootle_wallet_sdk::models::AuthLoginRequestEvent;
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_ootle_walletd_client::{
    permissions::{Permission, Permissions},
    types::{
        WebauthnAlreadyRegisteredRequest,
        WebauthnAlreadyRegisteredResponse,
        WebauthnFinishRegisterRequest,
        WebauthnFinishRegisterResponse,
        WebauthnStartAuthRequest,
        WebauthnStartAuthResponse,
        WebauthnStartRegisterRequest,
        WebauthnStartRegisterResponse,
    },
};
use uuid::Uuid;
use webauthn_rs::Webauthn;

use crate::{
    handlers::{HandlerContext, auth::REFRESH_TOKEN_COOKIE, helpers::invalid_request},
    services::{WebauthnService, WebauthnServiceError},
};

/// The permissions granted to the credential this wallet is enrolled with. Registration is
/// unauthenticated — it is the path that brings the wallet's first credential into existence — so the
/// set it mints is the daemon's to decide. A single-user wallet has exactly one owner, and the owner
/// is an administrator of their own wallet.
const BOOTSTRAP_PERMISSIONS: [Permission; 1] = [Permission::Admin];

fn is_enrolled(context: &HandlerContext) -> Result<bool, anyhow::Error> {
    Ok(webauthn_service(context)?.is_enrolled()?)
}

/// Reject enrolment once the wallet holds a credential. The wallet is single-user, so enrolment is a
/// property of the wallet itself and must not turn on the username, which the caller chooses freely.
///
/// This is the early rejection. Two concurrent enrolments both pass it, so the boundary that decides
/// the race is [`WebauthnService::finish_registration`], where the same check shares a transaction
/// with the insert.
fn assert_not_enrolled(context: &HandlerContext) -> Result<(), anyhow::Error> {
    if is_enrolled(context)? {
        return Err(invalid_request(
            "This wallet is already enrolled. A new credential must be added from an authenticated session",
        ));
    }

    Ok(())
}

fn webauthn(context: &HandlerContext) -> Result<&Webauthn, anyhow::Error> {
    context
        .webauthn()
        .ok_or_else(|| invalid_request("Webauthn is disabled for this wallet"))
}

fn webauthn_service(context: &HandlerContext) -> Result<&WebauthnService<SqliteWalletStore>, anyhow::Error> {
    context
        .webauthn_service()
        .ok_or_else(|| invalid_request("Webauthn is disabled for this wallet"))
}

/// Answer whether this wallet has been set up yet. The wallet is single-user, so the answer is
/// wallet-level and the request's `username` is not consulted — an unauthenticated caller learns
/// nothing about which usernames exist.
pub async fn handle_already_registered(
    context: &HandlerContext,
    _token: Option<&Bearer>,
    _request: WebauthnAlreadyRegisteredRequest,
) -> Result<WebauthnAlreadyRegisteredResponse, anyhow::Error> {
    webauthn(context)?;
    let registered = is_enrolled(context)?;
    Ok(WebauthnAlreadyRegisteredResponse { registered })
}

pub async fn handle_start_registration(
    context: &HandlerContext,
    _token: Option<&Bearer>,
    request: WebauthnStartRegisterRequest,
) -> Result<WebauthnStartRegisterResponse, anyhow::Error> {
    let webauthn = webauthn(context)?;
    assert_not_enrolled(context)?;
    let (response, passkey_reg) = webauthn.start_passkey_registration(
        Uuid::new_v4(),
        request.username.as_str(),
        request.username.as_str(),
        None,
    )?;

    let session_id = webauthn_service(context)?
        .start_registration(request.username, passkey_reg)
        .await?;

    Ok(WebauthnStartRegisterResponse {
        session_id,
        public_key: response.public_key,
    })
}

pub async fn handle_finish_registration(
    context: &HandlerContext,
    _token: Option<&Bearer>,
    request: (axum_jrpc::Id, WebauthnFinishRegisterRequest),
) -> Result<(CookieJar, JsonRpcResponse), anyhow::Error> {
    let (answer_id, request) = request;
    let webauthn = webauthn(context)?;
    let webauthn_service = webauthn_service(context)?;
    let session_data = webauthn_service.get_session(&request.session_id).await?;
    assert_not_enrolled(context)?;
    let passkey = webauthn.finish_passkey_registration(&request.credential, session_data.passkey_reg())?;
    webauthn_service
        .finish_registration(request.session_id, passkey)
        .await
        .map_err(|e| match e {
            WebauthnServiceError::AlreadyEnrolled => invalid_request(e),
            e => e.into(),
        })?;

    let jwt = context.jwt_api();
    let claims = jwt.generate_auth_claims(Permissions::from(BOOTSTRAP_PERMISSIONS.to_vec()))?;
    let token = jwt.grant(&claims)?;
    let refresh_token = context
        .refresh_token_store()
        .new_token(claims.permissions, claims.exp)
        .await;
    let refresh_cookie = refresh_token.into_cookie(REFRESH_TOKEN_COOKIE);
    let cookie = CookieJar::new().add(refresh_cookie);
    context.notifier().notify(AuthLoginRequestEvent);
    Ok((
        cookie,
        JsonRpcResponse::success(answer_id, WebauthnFinishRegisterResponse { token }),
    ))
}

/// Start a login ceremony for this wallet. The wallet is single-user, so every caller is offered the
/// wallet's own credentials.
pub async fn handle_start_auth(
    context: &HandlerContext,
    _token: Option<&Bearer>,
    _request: WebauthnStartAuthRequest,
) -> Result<WebauthnStartAuthResponse, anyhow::Error> {
    let webauthn = webauthn(context)?;
    let (challenge, session_id) = webauthn_service(context)?
        .start_authentication(webauthn)
        .await
        .map_err(|e| match e {
            WebauthnServiceError::NotEnrolled => invalid_request(e),
            e => e.into(),
        })?;
    Ok(WebauthnStartAuthResponse { session_id, challenge })
}
