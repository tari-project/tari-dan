//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    fmt::Debug,
    sync::Arc,
    time::{Duration, Instant},
};

use axum_extra::extract::cookie::{Cookie, SameSite};
use tari_ootle_walletd_client::{
    permissions::{Claims, Permissions},
    types::RefreshTokenHash,
};
use tokio::sync::RwLock;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone)]
pub struct RefreshTokenStore {
    tokens: Arc<RwLock<HashMap<RefreshTokenHash, RefreshTokenData>>>,
    expiry: Duration,
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct RefreshToken(Box<uuid::Bytes>);

impl RefreshToken {
    pub fn into_cookie(self, name: &str) -> Cookie<'_> {
        let mut cookie = Cookie::new(name, uuid::Uuid::from_bytes_ref(&self.0).to_string());
        cookie.set_http_only(true);
        cookie.set_same_site(SameSite::Strict);
        cookie
    }
}

impl AsRef<[u8]> for RefreshToken {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

#[derive(Debug)]
struct RefreshTokenData {
    claims: Claims,
    expires_at: Instant,
}

impl RefreshTokenStore {
    pub fn new(expiry: Duration) -> Self {
        Self {
            tokens: Default::default(),
            expiry,
        }
    }

    pub async fn new_token(&self, permissions: Permissions, exp: u64) -> RefreshToken {
        let mut write = self.tokens.write().await;
        clear_expired_tokens(&mut write);
        let data = RefreshTokenData {
            claims: Claims {
                permissions,
                exp,
                delegated: false,
                delegation_id: None,
            },
            expires_at: Instant::now() + self.expiry,
        };

        let token = RefreshToken(Box::new(uuid::Uuid::new_v4().into_bytes()));
        write.insert(hash_token(&token), data);
        token
    }

    pub async fn validate_token_str(&self, token: &str) -> Option<Claims> {
        let token = uuid::Uuid::parse_str(token)
            .ok()
            .map(|b| RefreshToken(Box::new(b.into_bytes())))?;
        let hashed_token = hash_token(&token);
        let read = self.tokens.read().await;
        if let Some(data) = read.get(&hashed_token) &&
            Instant::now() < data.expires_at
        {
            return Some(data.claims.clone());
        }
        None
    }

    pub async fn revoke_token(&self, token: &RefreshTokenHash) -> bool {
        let mut write = self.tokens.write().await;
        clear_expired_tokens(&mut write);
        write.remove(token).is_some()
    }

    pub async fn clear_expired(&self) {
        let mut write = self.tokens.write().await;
        clear_expired_tokens(&mut write);
    }

    pub async fn to_vec(&self) -> Vec<(RefreshTokenHash, Claims)> {
        let read = self.tokens.read().await;
        read.iter().map(|(k, v)| (*k, v.claims.clone())).collect()
    }
}

fn hash_token(token: &RefreshToken) -> RefreshTokenHash {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_ref());
    let result = hasher.finalize();
    RefreshTokenHash::new(result.into())
}

fn clear_expired_tokens(tokens: &mut HashMap<RefreshTokenHash, RefreshTokenData>) {
    let now = Instant::now();
    tokens.retain(|_, data| data.expires_at > now);
}

impl Debug for RefreshTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshTokenStore")
            .field("tokens", &"<redacted>")
            .finish()
    }
}
