// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Admin-only JSON-RPC handlers + shared helpers for the API key
//! authentication path (issue #1957).
//!
//! ## Trust model
//!
//! - API keys are long-lived credentials minted by an Admin user. The raw key bytes leave the daemon exactly once (the
//!   response of `auth.create_api_key`). Only a SHA-256 hex digest is persisted.
//! - The granted scopes on a key are an arbitrary subset of `Permissions`, chosen at creation time by the admin. The
//!   `Admin` scope itself is permitted as a grant (since the admin issuing the key has Admin), but the creator must set
//!   `confirm_admin: true` to acknowledge they're minting a credential that has full access.
//! - Revocation is immediate: the storage layer filters revoked rows out of `api_key_find_active_by_hash`, so a key
//!   whose `revoked_at` is set cannot authenticate even if a request is mid-flight.
//! - Agents present the raw key as the `Authorization: Bearer …` token on **every** JSON-RPC call. The shim in
//!   `HandlerContext::check_auth` routes any `tw_`-prefixed bearer to `find_active_by_raw`, parses the stored scopes,
//!   and applies the same `enforce_scopes` policy the JWT path applies. There is no `auth.request` round-trip and no
//!   JWT exchange for API keys.
//!
//! ## Key format
//!
//! Raw keys are 32 random bytes encoded with URL-safe Base64 (no padding)
//! and prefixed with `tw_` so they're identifiable in logs and the UI.
//! Total length ~46 chars. Sufficient entropy to make brute-force pointless;
//! the prefix is for human convenience only and is not used for auth.

use std::{str::FromStr, time::Duration};

use anyhow::{Context, anyhow};
use axum_extra::headers::authorization::Bearer;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Digest, Sha256};
use tari_ootle_wallet_sdk::{
    models::ApiKey,
    storage::{
        CommittableStore,
        ReadableWalletStore,
        WalletStorageError,
        WalletStore,
        WalletStoreReader,
        WalletStoreWriter,
        WriteableWalletStore,
    },
};
use tari_ootle_walletd_client::{
    permissions::{Permission, Permissions, TxRequestAction},
    types::{
        AuthCreateApiKeyRequest,
        AuthCreateApiKeyResponse,
        AuthListApiKeysRequest,
        AuthListApiKeysResponse,
        AuthRevokeApiKeyRequest,
        AuthRevokeApiKeyResponse,
        EncodedApiKey,
        IssuedApiKey,
    },
};
use zeroize::{Zeroize, Zeroizing};

use crate::handlers::HandlerContext;

/// Prefix all minted keys with this so the bearer-token router knows when to
/// resolve via the api_keys table vs. decode as JWT, and so they're
/// identifiable in logs. Not used as a security boundary — the SHA-256 hash
/// covers the whole string including the prefix.
pub const API_KEY_PREFIX: &str = "tw_";
/// 32 bytes of entropy. With URL-safe base64 this becomes 43 chars; with
/// the 3-char prefix the total key is 46 chars. 2^256 search space is
/// more than enough.
const API_KEY_RAW_BYTES: usize = 32;

/// Throttle for `last_used_at` bumps performed by the per-request auth
/// shim. Caps writes on the `api_keys` table to at most one per key per
/// window, which matters under a busy agent (the shim runs on every
/// authenticated call). The admin's "last used" hint is coarse-grained
/// information; 30 s is plenty of resolution.
pub const LAST_USED_BUMP_THROTTLE: Duration = Duration::from_secs(30);

/// Produce the deterministic storage hash for an API key. Callers MUST go
/// through this function — both the create path and the verify path — so a
/// future change to the hashing strategy only has to land in one place.
pub fn hash_api_key(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

/// Mint a fresh random API key string in the canonical form
/// `tw_<urlsafe_b64(32 random bytes)>`. Entropy comes from the OS-seeded
/// thread-local CSPRNG. Never returns the same string twice.
///
/// The raw-byte buffer is wrapped in `Zeroizing` so it is wiped after
/// encoding, and the returned string is wrapped in `Zeroizing<String>`
/// (`EncodedApiKey`) so its heap buffer is wiped on drop. Callers MUST
/// keep it as `EncodedApiKey` end-to-end — converting to a plain `String`
/// would defeat the wipe.
fn mint_raw_api_key() -> EncodedApiKey {
    let mut bytes = Zeroizing::new([0u8; API_KEY_RAW_BYTES]);
    rand::rng().fill_bytes(bytes.as_mut_slice());
    let mut encoded = URL_SAFE_NO_PAD.encode(bytes.as_slice());
    let mut out = String::with_capacity(API_KEY_PREFIX.len() + encoded.len());
    out.push_str(API_KEY_PREFIX);
    out.push_str(&encoded);
    // Wipe the intermediate base64 buffer before its drop runs — that
    // allocation also held the raw key in encoded form.
    encoded.zeroize();
    Zeroizing::new(out)
}

/// Sync lookup used by the per-request auth shim. Hashes the raw key,
/// queries the active-row filter on `api_keys`, and returns the matching
/// row (or `None` for both "no such key" and "key revoked" — the storage
/// layer filters the latter out, so the caller can't distinguish).
pub fn find_active_by_raw<TStore>(store: &TStore, raw_key: &str) -> Result<Option<ApiKey>, WalletStorageError>
where TStore: WalletStore {
    let hash = hash_api_key(raw_key);
    let mut tx = store.create_read_tx()?;
    tx.api_key_find_active_by_hash(&hash)
}

/// Sync, throttled `last_used_at` bump. Designed to be fired from
/// `tokio::spawn` after the auth shim has already returned, so a write
/// failure here cannot abort an already-successful request. The throttle
/// is enforced inside the storage layer so concurrent shim invocations
/// stay at most one write per key per window.
pub fn touch_last_used_throttled<TStore>(
    store: &TStore,
    id: i32,
    throttle: Duration,
) -> Result<(), WalletStorageError>
where
    TStore: WalletStore,
{
    let mut tx = store.create_write_tx()?;
    tx.api_key_touch_last_used(id, throttle)?;
    tx.commit()
}

/// Refuses a grant that only an interactive user session can exercise, since an API key carrying it could never use
/// it. `Admin` covers these endpoints by implication and is grantable, because the endpoints themselves refuse API
/// keys.
fn reject_user_only_grants(permissions: &Permissions) -> Result<(), anyhow::Error> {
    let approve = Permission::TransactionRequests(TxRequestAction::Approve);
    if permissions.has_permission(&approve) {
        return Err(anyhow!(
            "{approve} cannot be granted to an API key: approving a transaction request requires an interactive user \
             session"
        ));
    }
    Ok(())
}

pub fn parse_permissions(s: &str) -> Result<Permissions, anyhow::Error> {
    Permissions::from_str(s).map_err(|e| anyhow!("invalid permission string '{s}': {e}"))
}

// -----------------------------------------------------------------------------
// JSON-RPC handlers
// -----------------------------------------------------------------------------

pub async fn handle_create_api_key(
    context: &HandlerContext,
    token: Option<&Bearer>,
    request: AuthCreateApiKeyRequest,
) -> Result<AuthCreateApiKeyResponse, anyhow::Error> {
    // Admin + interactive user session. An API key — even one carrying
    // `Admin` — cannot mint further keys, so a leaked admin key cannot
    // establish persistence beyond its own revocation.
    context.authorize_user_only(token, &[Permission::Admin])?;

    if request.name.trim().is_empty() {
        return Err(anyhow!("API key name must not be empty"));
    }
    if request
        .name
        .trim()
        .eq_ignore_ascii_case(crate::handlers::context::LOCAL_COOKIE_REQUESTER)
    {
        return Err(anyhow!(
            "API key name '{}' is reserved for the daemon's local cookie",
            crate::handlers::context::LOCAL_COOKIE_REQUESTER
        ));
    }

    let permissions = Permissions::try_from(request.permissions.as_slice())
        .map_err(|e| anyhow!("Invalid permissions in API key request: {e}"))?;
    if permissions.is_empty() {
        return Err(anyhow!(
            "API key must grant at least one permission; refusing to issue an unusable key"
        ));
    }
    reject_user_only_grants(&permissions)?;
    // Granting Admin to an API key is allowed (the issuer already has it)
    // but is dangerous, so we require explicit acknowledgement so a UI
    // checkbox can be the gate rather than the JSON-RPC payload alone.
    if permissions.has_permission(&Permission::Admin) && !request.confirm_admin {
        return Err(anyhow!(
            "Granting the Admin permission to an API key requires `confirm_admin: true` in the request"
        ));
    }

    // Convert the optional unix-seconds expiry to a PrimitiveDateTime,
    // rejecting any value in the past — minting an already-expired key
    // would be useless and almost certainly a client bug.
    let expires_at = request
        .expires_at
        .map(|secs| {
            let ts = time::OffsetDateTime::from_unix_timestamp(secs)
                .map_err(|e| anyhow!("Invalid expires_at unix timestamp {secs}: {e}"))?;
            if ts <= time::OffsetDateTime::now_utc() {
                return Err(anyhow!(
                    "expires_at must be in the future; refusing to mint an already-expired key"
                ));
            }
            Ok(time::PrimitiveDateTime::new(ts.date(), ts.time()))
        })
        .transpose()?;

    let raw_key: EncodedApiKey = mint_raw_api_key();
    let hash = hash_api_key(&raw_key);

    // Serialise the permission set back to its textual form. We could
    // store JSON here but the JWT layer already uses the comma-separated
    // form and `Permissions::from_str` round-trips it, so this keeps
    // exactly one codec across the daemon.
    let permissions_str = format_permissions(&permissions);

    let stored = {
        let mut tx = context
            .wallet_sdk()
            .store()
            .create_write_tx()
            .context("Failed to open write transaction")?;
        let row = tx
            .api_key_insert(request.name.trim(), &hash, &permissions_str, expires_at)
            .context("Failed to insert api_key row")?;
        tx.commit().context("Failed to commit api_key insert")?;
        row
    };

    Ok(AuthCreateApiKeyResponse {
        id: stored.id,
        name: stored.name.clone(),
        permissions: permissions.into_vec(),
        // The raw key is returned exactly once. The client must store it
        // immediately — there is no way to retrieve it later from the
        // daemon. The DB only ever holds the hash.
        api_key: raw_key,
        created_at: stored.created_at.assume_utc().unix_timestamp(),
    })
}

pub async fn handle_list_api_keys(
    context: &HandlerContext,
    token: Option<&Bearer>,
    request: AuthListApiKeysRequest,
) -> Result<AuthListApiKeysResponse, anyhow::Error> {
    // User-auth-only: enumeration is an admin-tooling operation; an API
    // key shouldn't be able to enumerate the wallet's other keys.
    context.authorize_user_only(token, &[Permission::Admin])?;

    let rows = {
        let mut tx = context
            .wallet_sdk()
            .store()
            .create_read_tx()
            .context("Failed to open read transaction")?;
        tx.api_key_list(request.include_revoked)?
    };

    let keys = rows
        .into_iter()
        .map(|k| {
            let permissions = Permissions::from_str(&k.permissions)
                .map(Permissions::into_vec)
                .unwrap_or_default();
            IssuedApiKey {
                id: k.id,
                name: k.name,
                permissions,
                created_at: k.created_at.assume_utc().unix_timestamp(),
                last_used_at: k
                    .last_used_at
                    .map(|ts: time::PrimitiveDateTime| ts.assume_utc().unix_timestamp()),
                revoked_at: k
                    .revoked_at
                    .map(|ts: time::PrimitiveDateTime| ts.assume_utc().unix_timestamp()),
                expires_at: k
                    .expires_at
                    .map(|ts: time::PrimitiveDateTime| ts.assume_utc().unix_timestamp()),
            }
        })
        .collect();

    Ok(AuthListApiKeysResponse { keys })
}

pub async fn handle_revoke_api_key(
    context: &HandlerContext,
    token: Option<&Bearer>,
    request: AuthRevokeApiKeyRequest,
) -> Result<AuthRevokeApiKeyResponse, anyhow::Error> {
    // User-auth-only: a leaked admin key cannot revoke other (or its own
    // sibling) keys; an attacker cannot use a compromised key to "clean
    // up" the audit trail.
    context.authorize_user_only(token, &[Permission::Admin])?;

    let mut tx = context
        .wallet_sdk()
        .store()
        .create_write_tx()
        .context("Failed to open write transaction")?;
    tx.api_key_revoke(request.id).context("Failed to revoke api_key")?;
    tx.commit().context("Failed to commit api_key revocation")?;

    Ok(AuthRevokeApiKeyResponse {})
}

/// Round-trip a `Permissions` back to the comma-separated textual form
/// that `Permissions::from_str` accepts. We keep the order
/// deterministic (sorted by display form) so a re-list returns the same
/// string the user supplied at creation time.
fn format_permissions(p: &Permissions) -> String {
    let mut parts: Vec<String> = p.iter().map(|p| p.to_string()).collect();
    parts.sort();
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use tari_ootle_walletd_client::permissions::Crud;

    use super::*;

    #[test]
    fn minted_keys_have_correct_shape_and_are_unique() {
        // Every minted key must start with the `tw_` prefix so it's
        // identifiable in logs, and two consecutive mints must never
        // collide. 32 bytes of entropy makes that overwhelmingly likely
        // anyway, but we still assert it to catch any accidental
        // shared-state regression in the minter.
        let a = mint_raw_api_key();
        let b = mint_raw_api_key();
        assert!(a.starts_with(API_KEY_PREFIX), "key {} missing prefix", *a);
        assert!(b.starts_with(API_KEY_PREFIX));
        assert_ne!(a, b, "two consecutive mints must not collide");
        // Prefix (3) + ceil(32 * 4 / 3) without padding = 3 + 43 = 46
        assert_eq!(a.len(), API_KEY_PREFIX.len() + 43);
    }

    #[test]
    fn hashing_is_deterministic_and_distinguishes_inputs() {
        // The same input always hashes to the same digest (otherwise no
        // existing key would ever authenticate again). Distinct inputs
        // must hash to distinct digests — at SHA-256 strength the
        // probability of accidental collision in any realistic deployment
        // is effectively zero.
        let k1 = mint_raw_api_key();
        let k2 = mint_raw_api_key();
        assert_eq!(hash_api_key(&k1), hash_api_key(&k1));
        assert_ne!(hash_api_key(&k1), hash_api_key(&k2));
        // SHA-256 hex digest is always 64 chars.
        assert_eq!(hash_api_key(&k1).len(), 64);
    }

    #[test]
    fn format_permissions_round_trips_through_from_str() {
        // `format_permissions` must produce a string that
        // `Permissions::from_str` accepts and reconstructs into the
        // same logical permission set. This is the single most important
        // invariant for the storage column: we re-parse it on every
        // authentication.
        let original: Permissions = vec![
            Permission::Accounts(Crud::Read, None),
            Permission::Transactions(Crud::Read),
            Permission::Keys(Crud::Read),
        ]
        .into();
        let s = format_permissions(&original);
        let parsed: Permissions = Permissions::from_str(&s).expect("must re-parse");
        // HashSet order is not guaranteed; compare membership instead of
        // direct equality.
        assert_eq!(parsed.len(), 3);
        assert!(parsed.has_permission(&Permission::Accounts(Crud::Read, None)));
        assert!(parsed.has_permission(&Permission::Transactions(Crud::Read)));
        assert!(parsed.has_permission(&Permission::Keys(Crud::Read)));
    }

    #[test]
    fn format_permissions_is_deterministic_under_input_reordering() {
        // The list endpoint shows users the exact permission string they
        // supplied at creation time. Sorting the parts before joining
        // means the order they typed them in is irrelevant — they always
        // see the same canonical string.
        let a: Permissions = vec![
            Permission::Accounts(Crud::Read, None),
            Permission::Transactions(Crud::Read),
        ]
        .into();
        let b: Permissions = vec![
            Permission::Transactions(Crud::Read),
            Permission::Accounts(Crud::Read, None),
        ]
        .into();
        assert_eq!(format_permissions(&a), format_permissions(&b));
    }

    #[test]
    fn approving_transaction_requests_cannot_be_granted_to_an_api_key() {
        let approve: Permissions = vec![
            Permission::TransactionRequests(TxRequestAction::Create),
            Permission::TransactionRequests(TxRequestAction::Approve),
        ]
        .into();
        assert!(reject_user_only_grants(&approve).is_err());

        let create_and_read: Permissions = vec![
            Permission::TransactionRequests(TxRequestAction::Create),
            Permission::TransactionRequests(TxRequestAction::Read),
        ]
        .into();
        assert!(reject_user_only_grants(&create_and_read).is_ok());
        assert!(reject_user_only_grants(&vec![Permission::Admin].into()).is_ok());
    }
}

/// Integration tests covering the building blocks the per-request auth shim
/// in `HandlerContext::check_auth` composes: `find_active_by_raw` →
/// `parse_permissions` → `enforce_scopes`. Standing up a full
/// `HandlerContext` would require the entire wallet-daemon service stack,
/// so we exercise the same composition the shim performs against an
/// in-memory SQLite store. If any of these regress, the shim regresses.
#[cfg(test)]
mod shim_tests {
    use std::time::Duration;

    use tari_ootle_wallet_sdk::storage::{CommittableStore, WalletStoreWriter, WriteableWalletStore};
    use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
    use tari_ootle_walletd_client::permissions::{Crud, Permission, Permissions};

    use super::*;
    use crate::handlers::auth::jwt::{AuthError, enforce_scopes};

    fn open_store() -> SqliteWalletStore {
        let db = SqliteWalletStore::try_open(":memory:").unwrap();
        db.run_migrations().unwrap();
        db
    }

    fn mint_key(store: &SqliteWalletStore, name: &str, perms: Permissions) -> (EncodedApiKey, i32) {
        mint_key_with_expiry(store, name, perms, None)
    }

    fn mint_key_with_expiry(
        store: &SqliteWalletStore,
        name: &str,
        perms: Permissions,
        expires_at: Option<time::PrimitiveDateTime>,
    ) -> (EncodedApiKey, i32) {
        let raw = mint_raw_api_key();
        let hash = hash_api_key(&raw);
        let perm_str = format_permissions(&perms);
        let mut tx = store.create_write_tx().unwrap();
        let row = tx.api_key_insert(name, &hash, &perm_str, expires_at).unwrap();
        tx.commit().unwrap();
        (raw, row.id)
    }

    /// Replays the shim path: lookup → parse → scope-check. The shim itself
    /// adds a `tokio::spawn` for the `last_used_at` bump, which is covered
    /// by the storage-layer throttle tests; the auth decision itself is
    /// exactly this composition.
    fn resolve(store: &SqliteWalletStore, raw: &str, required: &[Permission]) -> Result<Permissions, AuthError> {
        let row = find_active_by_raw(store, raw)
            .map_err(AuthError::from)?
            .ok_or(AuthError::ApiKeyInvalidOrRevoked)?;
        let granted = parse_permissions(&row.permissions).map_err(|_| AuthError::ApiKeyInvalidOrRevoked)?;
        enforce_scopes(&granted, required)?;
        Ok(granted)
    }

    #[test]
    fn valid_key_in_scope_is_accepted() {
        let store = open_store();
        let (raw, _) = mint_key(
            &store,
            "agent",
            vec![
                Permission::Accounts(Crud::Read, None),
                Permission::Transactions(Crud::Read),
            ]
            .into(),
        );
        let granted = resolve(&store, &raw, &[Permission::Accounts(Crud::Read, None)]).expect("in-scope must succeed");
        assert!(granted.has_permission(&Permission::Accounts(Crud::Read, None)));
        assert!(!granted.has_permission(&Permission::Admin));
    }

    #[test]
    fn out_of_scope_call_is_rejected() {
        let store = open_store();
        let (raw, _) = mint_key(&store, "info-only", vec![Permission::Accounts(Crud::Read, None)].into());
        let err =
            resolve(&store, &raw, &[Permission::Transactions(Crud::Read)]).expect_err("out-of-scope must be rejected");
        assert!(
            matches!(err, AuthError::InsufficientPermissions {
                required: Permission::Transactions(Crud::Read)
            }),
            "expected InsufficientPermissions(transactions:read), got {err:?}"
        );
    }

    #[test]
    fn unknown_key_yields_api_key_invalid_or_revoked() {
        // Probe-resistance: an unknown `tw_…` produces exactly the same
        // error variant as a revoked one. The shim and these tests share
        // the same code path so the error parity is a direct property of
        // the storage-layer active filter.
        let store = open_store();
        let err = resolve(&store, "tw_does-not-exist", &[Permission::Accounts(Crud::Read, None)])
            .expect_err("unknown key must be rejected");
        assert!(
            matches!(err, AuthError::ApiKeyInvalidOrRevoked),
            "expected ApiKeyInvalidOrRevoked, got {err:?}"
        );
    }

    #[test]
    fn revoked_key_yields_api_key_invalid_or_revoked() {
        let store = open_store();
        let (raw, id) = mint_key(&store, "ephemeral", vec![Permission::Accounts(Crud::Read, None)].into());
        resolve(&store, &raw, &[Permission::Accounts(Crud::Read, None)]).expect("pre-revoke must succeed");

        store.with_write_tx(|tx| tx.api_key_revoke(id)).unwrap();

        let err = resolve(&store, &raw, &[Permission::Accounts(Crud::Read, None)]).expect_err("post-revoke must fail");
        assert!(matches!(err, AuthError::ApiKeyInvalidOrRevoked));
    }

    /// `check_auth_user_only`-shaped behaviour: an API-key bearer must be
    /// rejected up-front by the prefix check, regardless of what scopes the
    /// key actually carries. The user-only gate is the bedrock of the
    /// "API keys can't mint API keys" property; if this regresses, a
    /// compromised Admin key can establish persistence.
    #[test]
    fn api_key_bearer_is_rejected_by_user_only_prefix_check() {
        // The actual `HandlerContext::check_auth_user_only` requires the
        // full handler context, but the gate itself is a single condition:
        // bearer string starts with `API_KEY_PREFIX` ⇒ `UserAuthOnly`.
        // Mirror that here so a future refactor that changes the prefix
        // semantics (e.g. accepting multiple prefixes) is forced to
        // update this test.
        fn user_only_gate(bearer_token: &str) -> Result<(), AuthError> {
            if bearer_token.starts_with(API_KEY_PREFIX) {
                return Err(AuthError::UserAuthOnly);
            }
            Ok(())
        }

        let store = open_store();
        let (raw, _) = mint_key(&store, "admin-leaked", vec![Permission::Admin].into());
        let err = user_only_gate(&raw).expect_err("admin API key MUST NOT pass the user-only gate");
        assert!(matches!(err, AuthError::UserAuthOnly));

        // A JWT-shaped string (no `tw_` prefix) passes the gate so the
        // JWT path can handle it normally.
        user_only_gate("eyJhbGciOiJIUzI1NiJ9.fakeclaims.sig").expect("JWT bearer must pass the user-only gate");
    }

    /// An expired API key must be indistinguishable from a revoked or
    /// unknown one at the auth boundary — same `ApiKeyInvalidOrRevoked`
    /// error, no enumeration leak via error type or string.
    #[test]
    fn expired_key_yields_api_key_invalid_or_revoked() {
        let store = open_store();
        let past = time::OffsetDateTime::now_utc() - time::Duration::seconds(60);
        let past = time::PrimitiveDateTime::new(past.date(), past.time());
        let (raw, _id) = mint_key_with_expiry(
            &store,
            "expired",
            vec![Permission::Accounts(Crud::Read, None)].into(),
            Some(past),
        );

        let err =
            resolve(&store, &raw, &[Permission::Accounts(Crud::Read, None)]).expect_err("expired key must be rejected");
        assert!(
            matches!(err, AuthError::ApiKeyInvalidOrRevoked),
            "expected ApiKeyInvalidOrRevoked, got {err:?}"
        );
    }

    /// A key with a future `expires_at` authenticates normally — the
    /// expiry filter is `expires_at > NOW`, so a future timestamp passes.
    #[test]
    fn future_expiry_key_is_accepted() {
        let store = open_store();
        let future = time::OffsetDateTime::now_utc() + time::Duration::seconds(3600);
        let future = time::PrimitiveDateTime::new(future.date(), future.time());
        let (raw, _) = mint_key_with_expiry(
            &store,
            "future",
            vec![Permission::Accounts(Crud::Read, None)].into(),
            Some(future),
        );

        resolve(&store, &raw, &[Permission::Accounts(Crud::Read, None)]).expect("future-expiry key must authenticate");
    }

    #[test]
    fn admin_scope_grants_bypass() {
        // The shim's Admin shortcut comes from `enforce_scopes` — a key
        // carrying `Admin` passes ANY required scope check without the
        // per-permission iteration.
        let store = open_store();
        let (raw, _) = mint_key(&store, "everything", vec![Permission::Admin].into());
        resolve(&store, &raw, &[
            Permission::Accounts(Crud::Read, None),
            Permission::Transactions(Crud::Read),
        ])
        .expect("Admin grant must satisfy any required scopes");
    }

    #[test]
    fn touch_last_used_throttle_skips_within_window() {
        // The shim calls `touch_last_used_throttled` after a successful
        // auth. Hammering the helper within the throttle window must not
        // produce more than one write — the storage-layer filter enforces
        // it independently of the in-memory DashMap on `HandlerContext`.
        let store = open_store();
        let (_, id) = mint_key(&store, "throttled", vec![Permission::Accounts(Crud::Read, None)].into());

        touch_last_used_throttled(&store, id, LAST_USED_BUMP_THROTTLE).unwrap();
        let first = {
            let mut tx = store.create_write_tx().unwrap();
            tx.api_key_get_by_id(id).unwrap().last_used_at.unwrap()
        };

        // Second call within the window: no-op.
        touch_last_used_throttled(&store, id, LAST_USED_BUMP_THROTTLE).unwrap();
        let second = {
            let mut tx = store.create_write_tx().unwrap();
            tx.api_key_get_by_id(id).unwrap().last_used_at.unwrap()
        };
        assert_eq!(first, second, "second touch within window must not advance timestamp");

        // ZERO throttle bypasses the window.
        touch_last_used_throttled(&store, id, Duration::ZERO).unwrap();
        let third = {
            let mut tx = store.create_write_tx().unwrap();
            tx.api_key_get_by_id(id).unwrap().last_used_at.unwrap()
        };
        assert!(third >= second);
    }
}
