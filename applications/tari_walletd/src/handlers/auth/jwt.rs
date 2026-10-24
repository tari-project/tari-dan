//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum_extra::headers::authorization::Bearer;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, TokenData, Validation};
use tari_crypto::tari_utilities::SafePassword;
use tari_ootle_wallet_sdk::storage::WalletStorageError;
use tari_ootle_walletd_client::{
    permissions::{Claims, Permission, Permissions},
    types::EncodedJwtString,
};

pub struct JwtApi<'a> {
    default_expiry: Duration,
    jwt_secret_key: &'a SafePassword,
}

impl<'a> JwtApi<'a> {
    pub(crate) fn new(default_expiry: Duration, jwt_secret_key: &'a SafePassword) -> Self {
        Self {
            default_expiry,
            jwt_secret_key,
        }
    }

    pub fn generate_auth_claims(&self, permissions: Permissions) -> Result<Claims, AuthError> {
        let valid_till = SystemTime::now() + self.default_expiry;
        let exp = valid_till
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AuthError::InvalidExpiry)?;
        Ok(Claims {
            permissions,
            exp: exp.as_secs(),
            delegated: false,
            delegation_id: None,
        })
    }

    fn decode_jwt(&self, token: &str) -> Result<TokenData<Claims>, AuthError> {
        let token_data = jsonwebtoken::decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret_key.reveal()),
            &Validation::default(),
        )?;
        Ok(token_data)
    }

    pub fn grant(&self, claims: &Claims) -> Result<EncodedJwtString, AuthError> {
        let permissions_token = jsonwebtoken::encode(
            &Header::default(),
            claims,
            &EncodingKey::from_secret(self.jwt_secret_key.reveal()),
        )?;

        Ok(permissions_token.into())
    }

    /// Validate the bearer JWT and return its granted permissions. Does
    /// not check any specific requirement — use [`enforce_scopes`] (or
    /// `HandlerContext::authorize` for the combined check).
    pub fn check_auth(&self, token: Option<&Bearer>) -> Result<Permissions, AuthError> {
        let token = token.ok_or(AuthError::AccessDeniedNoBearerToken)?;
        let token_data = self.decode_jwt(token.token())?;
        Ok(token_data.claims.permissions)
    }

    /// Validate the bearer JWT and return all of its claims, including
    /// whether it is a delegated token.
    pub fn check_auth_claims(&self, token: Option<&Bearer>) -> Result<Claims, AuthError> {
        let token = token.ok_or(AuthError::AccessDeniedNoBearerToken)?;
        Ok(self.decode_jwt(token.token())?.claims)
    }

    /// Like [`Self::check_auth`], but refuses a delegated token: only a token
    /// issued to the wallet's own user session passes.
    pub fn check_user_auth(&self, token: Option<&Bearer>) -> Result<Permissions, AuthError> {
        let token = token.ok_or(AuthError::AccessDeniedNoBearerToken)?;
        let token_data = self.decode_jwt(token.token())?;
        if token_data.claims.delegated {
            return Err(AuthError::UserAuthOnly);
        }
        Ok(token_data.claims.permissions)
    }
}

/// Surface [`Permissions::check`] as an `AuthError` so handlers can use a
/// single `?` operator. Shared by the JWT and API-key paths so both enforce
/// the exact same policy.
pub fn enforce_scopes(granted: &Permissions, required: &[Permission]) -> Result<(), AuthError> {
    granted
        .check(required)
        .map_err(|required| AuthError::InsufficientPermissions { required })
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("JWT error: {0}")]
    JwtError(jsonwebtoken::errors::Error),
    #[error("Access denied. No bearer token provided")]
    AccessDeniedNoBearerToken,
    #[error("Access denied. Invalid bearer token")]
    AccessDeniedInvalidBearerToken,
    #[error("Access denied. Expired token")]
    AccessDeniedExpiredToken,
    // Same wording for "no such key" and "key revoked" so an attacker can't
    // distinguish via the error string. Mapped to the same 401 Unauthorized
    // status as the other bearer-token failures.
    #[error("Access denied. API key is invalid or revoked")]
    ApiKeyInvalidOrRevoked,
    // The endpoint requires an interactive user session (WebAuthn) — API
    // keys, delegated tokens, and the local cookie are deliberately excluded so
    // a leaked Admin key or local `.cookie` cannot mint or revoke further keys,
    // export secret keys, or approve requests.
    #[error(
        "Access denied. This endpoint requires an interactive user session, not an API key, delegated token, or local \
         cookie"
    )]
    UserAuthOnly,
    #[error("Insufficient permissions. Required '{required:?}'")]
    InsufficientPermissions { required: Permission },
    #[error("Invalid expiry")]
    InvalidExpiry,
}

impl From<jsonwebtoken::errors::Error> for AuthError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        match err.kind() {
            jsonwebtoken::errors::ErrorKind::InvalidToken => Self::AccessDeniedInvalidBearerToken,
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => Self::AccessDeniedExpiredToken,
            _ => Self::JwtError(err),
        }
    }
}
