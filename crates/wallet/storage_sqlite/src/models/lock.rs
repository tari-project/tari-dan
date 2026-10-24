//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::{Identifiable, Queryable};
use tari_ootle_wallet_sdk::{
    models::{LockOwner, WalletLock},
    storage::WalletStorageError,
};
use time::PrimitiveDateTime;

use crate::{schema::locks, serialization::deserialize_hex_try_from};

/// One row of the `locks` table.
#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = locks)]
pub struct Lock {
    pub id: i32,
    pub transaction_id: Option<String>,
    pub timeout_at: Option<PrimitiveDateTime>,
    pub created_at: PrimitiveDateTime,
    pub owner: Option<String>,
}

impl Lock {
    pub fn try_into_wallet_lock(self) -> Result<WalletLock, WalletStorageError> {
        Ok(WalletLock {
            id: self.id,
            owner: decode_lock_owner(self.owner.as_deref())?,
            transaction_id: self.transaction_id.map(deserialize_hex_try_from).transpose()?,
            timeout_at: self.timeout_at,
        })
    }
}

const LOCAL_COOKIE_OWNER: &str = "local_cookie";
const DELEGATED_PREFIX: &str = "delegated:";
const API_KEY_PREFIX: &str = "api_key:";

pub fn encode_lock_owner(owner: LockOwner) -> Option<String> {
    match owner {
        LockOwner::Wallet => None,
        LockOwner::LocalCookie => Some(LOCAL_COOKIE_OWNER.to_string()),
        LockOwner::Delegated(id) => Some(format!("{DELEGATED_PREFIX}{id}")),
        LockOwner::ApiKey(id) => Some(format!("{API_KEY_PREFIX}{id}")),
    }
}

fn decode_lock_owner(owner: Option<&str>) -> Result<LockOwner, WalletStorageError> {
    let Some(owner) = owner else {
        return Ok(LockOwner::Wallet);
    };
    let decoded = if owner == LOCAL_COOKIE_OWNER {
        Some(LockOwner::LocalCookie)
    } else if let Some(id) = owner.strip_prefix(DELEGATED_PREFIX) {
        id.parse().ok().map(LockOwner::Delegated)
    } else {
        owner
            .strip_prefix(API_KEY_PREFIX)
            .and_then(|id| id.parse().ok())
            .map(LockOwner::ApiKey)
    };
    decoded.ok_or_else(|| WalletStorageError::DecodingError {
        operation: "decode_lock_owner",
        item: "locks.owner",
        details: format!("unrecognised lock owner '{owner}'"),
    })
}
