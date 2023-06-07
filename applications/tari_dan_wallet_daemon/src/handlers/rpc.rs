//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_sdk::apis::jwt::JrpcPermission;
use tari_wallet_daemon_client::types::{
    AuthGetAllJwtRequest, AuthGetAllJwtResponse, AuthLoginAcceptRequest, AuthLoginAcceptResponse, AuthLoginDenyRequest,
    AuthLoginDenyResponse, AuthLoginRequest, AuthLoginResponse, AuthRevokeTokenRequest, AuthRevokeTokenResponse,
};

use crate::{handlers::HandlerContext, services::AuthLoginRequestEvent};

pub async fn handle_discover(
    _context: &HandlerContext,
    _token: Option<String>,
    _value: serde_json::Value,
) -> Result<serde_json::Value, anyhow::Error> {
    Ok(serde_json::from_str(include_str!("../../openrpc.json"))?)
}

pub async fn handle_login_request(
    context: &HandlerContext,
    _: Option<String>,
    auth_request: AuthLoginRequest,
) -> Result<AuthLoginResponse, anyhow::Error> {
    let jwt = context.wallet_sdk().jwt_api();

    let (auth_token, valid_till) = jwt.generate_auth_token(auth_request.permissions, auth_request.duration)?;
    context.notifier().notify(AuthLoginRequestEvent {
        auth_token: auth_token.clone(),
        valid_till,
    });
    Ok(AuthLoginResponse { auth_token })
}

pub async fn handle_login_accept(
    context: &HandlerContext,
    _: Option<String>,
    auth_accept_request: AuthLoginAcceptRequest,
) -> Result<AuthLoginAcceptResponse, anyhow::Error> {
    let jwt = context.wallet_sdk().jwt_api();
    let permissions_token = jwt.grant(auth_accept_request.name, auth_accept_request.auth_token)?;
    Ok(AuthLoginAcceptResponse { permissions_token })
}

pub async fn handle_login_deny(
    context: &HandlerContext,
    _: Option<String>,
    auth_deny_request: AuthLoginDenyRequest,
) -> Result<AuthLoginDenyResponse, anyhow::Error> {
    let jwt = context.wallet_sdk().jwt_api();
    jwt.deny(auth_deny_request.auth_token)?;
    Ok(AuthLoginDenyResponse {})
}

pub async fn handle_revoke(
    context: &HandlerContext,
    token: Option<String>,
    revoke_request: AuthRevokeTokenRequest,
) -> Result<AuthRevokeTokenResponse, anyhow::Error> {
    let jwt = context.wallet_sdk().jwt_api();
    jwt.check_auth(token, &[JrpcPermission::Admin])?;
    jwt.revoke(revoke_request.permission_token.as_str())?;
    Ok(AuthRevokeTokenResponse {})
}

pub async fn handle_get_all_jwt(
    context: &HandlerContext,
    token: Option<String>,
    _request: AuthGetAllJwtRequest,
) -> Result<AuthGetAllJwtResponse, anyhow::Error> {
    let jwt = context.wallet_sdk().jwt_api();
    jwt.check_auth(token, &[JrpcPermission::Admin])?;
    let tokens = jwt.get_tokens()?;
    Ok(AuthGetAllJwtResponse { jwt: tokens })
}
