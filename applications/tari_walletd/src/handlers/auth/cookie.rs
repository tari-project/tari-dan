// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Ephemeral cookie-file authentication for local clients.
//!
//! On startup, when `ootle_wallet_daemon.cookie_auth` is enabled, the daemon mints a
//! fresh random token (`tc_<urlsafe_b64(32 random bytes)>`), writes it
//! atomically with mode `0600` to `<base_path>/.cookie`, and removes the file
//! on clean shutdown via [`CookieFileGuard`]. Restarting the daemon rotates the
//! cookie token automatically.
//!
//! Requests carrying `Authorization: Bearer tc_...` are verified in constant
//! time against the in-memory token and granted the configured `cookie_scopes`
//! (`["admin"]` by default), while still being rejected by
//! `HandlerContext::check_auth_user_only` so a cookie bearer cannot export
//! keys, mint/revoke long-lived API keys, or approve transaction/signing
//! requests.

use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use anyhow::Context;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use log::*;
use rand::Rng;
use sha2::{Digest, Sha256};
pub use tari_ootle_walletd_client::{COOKIE_TOKEN_PREFIX, cookie_file_path};
use tari_utilities::SafePassword;
use tempfile::Builder as TempFileBuilder;
use zeroize::{Zeroize, Zeroizing};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::auth::cookie";

/// Number of random bytes encoded in a cookie token.
const COOKIE_RAW_BYTES: usize = 32;

/// Generates a fresh `tc_<urlsafe_b64(32 random bytes)>` cookie token.
pub fn mint_cookie_token() -> SafePassword {
    let mut bytes = Zeroizing::new([0u8; COOKIE_RAW_BYTES]);
    rand::rng().fill_bytes(bytes.as_mut_slice());
    let mut encoded = URL_SAFE_NO_PAD.encode(bytes.as_slice());
    let mut raw = String::with_capacity(COOKIE_TOKEN_PREFIX.len() + encoded.len());
    raw.push_str(COOKIE_TOKEN_PREFIX);
    raw.push_str(&encoded);
    encoded.zeroize();
    let token = SafePassword::from(raw.as_str());
    raw.zeroize();
    token
}

/// Compares `presented` against `expected` in constant time.
///
/// Both inputs are hashed with SHA-256 (32-byte fixed-size digests) and all 32
/// bytes are XOR-accumulated alongside a constant-time length check so neither
/// token contents nor prefix match length is leaked via timing side channels.
pub fn verify_cookie_token(expected: &SafePassword, presented: &str) -> bool {
    if !presented.starts_with(COOKIE_TOKEN_PREFIX) {
        return false;
    }

    let expected_bytes = expected.reveal().as_slice();
    let presented_bytes = presented.as_bytes();

    let expected_digest = Sha256::digest(expected_bytes);
    let presented_digest = Sha256::digest(presented_bytes);

    let mut diff: u8 = 0;
    for (a, b) in expected_digest.iter().zip(presented_digest.iter()) {
        diff |= std::hint::black_box(*a ^ *b);
    }

    let len_eq = std::hint::black_box(expected_bytes.len() ^ presented_bytes.len()) == 0;
    diff == 0 && len_eq
}

/// RAII guard that creates `<base_path>/.cookie` atomically on startup and
/// deletes it when dropped on clean daemon shutdown.
#[derive(Debug)]
pub struct CookieFileGuard {
    path: PathBuf,
}

impl CookieFileGuard {
    /// Mints a fresh cookie token, writes it atomically to `<base_path>/.cookie`
    /// (with mode `0600` on Unix), and returns the in-memory token together with
    /// the guard that removes the file on drop.
    pub fn create(base_path: &Path) -> anyhow::Result<(SafePassword, Self)> {
        fs::create_dir_all(base_path)
            .with_context(|| format!("failed to create wallet daemon base directory {}", base_path.display()))?;

        let token = mint_cookie_token();
        let target_path = cookie_file_path(base_path);

        let mut temp_file = TempFileBuilder::new()
            .prefix(".cookie.tmp.")
            .tempfile_in(base_path)
            .with_context(|| format!("failed to create temporary cookie file in {}", base_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            temp_file
                .as_file()
                .set_permissions(perms)
                .context("failed to set 0600 permissions on temporary cookie file")?;
        }

        temp_file
            .as_file_mut()
            .write_all(token.reveal().as_slice())
            .context("failed to write cookie token to temporary file")?;
        temp_file
            .as_file()
            .sync_all()
            .context("failed to sync temporary cookie file to disk")?;

        temp_file
            .persist(&target_path)
            .map_err(|err| err.error)
            .with_context(|| format!("failed to atomically persist cookie file to {}", target_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&target_path, perms)
                .with_context(|| format!("failed to enforce 0600 permissions on {}", target_path.display()))?;
        }

        let guard = Self { path: target_path };
        info!(
            target: LOG_TARGET,
            "Wrote local authentication cookie file to {}",
            guard.path().display()
        );

        Ok((token, guard))
    }

    /// Removes any leftover `<base_path>/.cookie` file when `cookie_auth` is disabled.
    pub fn remove_stale(base_path: &Path) {
        let path = cookie_file_path(base_path);
        match fs::remove_file(&path) {
            Ok(()) => {
                info!(
                    target: LOG_TARGET,
                    "Removed stale authentication cookie file at {}",
                    path.display()
                );
            },
            Err(err) if err.kind() == ErrorKind::NotFound => {},
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to remove stale authentication cookie file at {}: {}",
                    path.display(),
                    err
                );
            },
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CookieFileGuard {
    fn drop(&mut self) {
        match fs::remove_file(&self.path) {
            Ok(()) => {
                info!(
                    target: LOG_TARGET,
                    "Removed authentication cookie file at {}",
                    self.path.display()
                );
            },
            Err(err) if err.kind() == ErrorKind::NotFound => {},
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to remove authentication cookie file at {} on shutdown: {}",
                    self.path.display(),
                    err
                );
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str;

    use tempfile::tempdir;

    use super::*;

    fn reveal_str(token: &SafePassword) -> &str {
        str::from_utf8(token.reveal()).expect("ASCII cookie token")
    }

    #[test]
    fn mint_cookie_token_format_and_uniqueness() {
        let a = mint_cookie_token();
        let b = mint_cookie_token();
        assert_ne!(reveal_str(&a), reveal_str(&b));
        assert!(reveal_str(&a).starts_with(COOKIE_TOKEN_PREFIX));
        // "tc_" (3) + base64_url_nopad(32 bytes) (43) == 46 chars
        assert_eq!(reveal_str(&a).len(), 46);
        let decoded = URL_SAFE_NO_PAD
            .decode(&reveal_str(&a)[COOKIE_TOKEN_PREFIX.len()..])
            .expect("valid base64 body");
        assert_eq!(decoded.len(), COOKIE_RAW_BYTES);
    }

    #[test]
    fn verify_cookie_token_checks_constant_time_equality() {
        let token = mint_cookie_token();
        assert!(verify_cookie_token(&token, reveal_str(&token)));

        // Mutate final character
        let mut wrong = reveal_str(&token).to_string();
        let last = wrong.pop().unwrap();
        wrong.push(if last == 'A' { 'B' } else { 'A' });
        assert!(!verify_cookie_token(&token, &wrong));

        // Truncated token
        assert!(!verify_cookie_token(&token, &reveal_str(&token)[..45]));

        // Wrong prefix or empty
        assert!(!verify_cookie_token(&token, ""));
        assert!(!verify_cookie_token(&token, "tw_not_a_cookie_token"));
    }

    #[test]
    fn cookie_file_guard_creates_rotates_and_deletes_on_drop() {
        let dir = tempdir().unwrap();
        let cookie_path = cookie_file_path(dir.path());

        let (first_token, guard1) = CookieFileGuard::create(dir.path()).unwrap();
        assert_eq!(guard1.path(), cookie_path.as_path());
        assert!(cookie_path.exists());

        let on_disk = fs::read_to_string(&cookie_path).unwrap();
        assert_eq!(on_disk, reveal_str(&first_token));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&cookie_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Simulating a daemon restart rotates the cookie atomically.
        let (second_token, guard2) = CookieFileGuard::create(dir.path()).unwrap();
        assert_ne!(reveal_str(&first_token), reveal_str(&second_token));
        let on_disk_rotated = fs::read_to_string(&cookie_path).unwrap();
        assert_eq!(on_disk_rotated, reveal_str(&second_token));

        // Dropping the active guard removes the cookie file on clean shutdown.
        std::mem::forget(guard1);
        drop(guard2);
        assert!(!cookie_path.exists());
    }

    #[test]
    fn remove_stale_deletes_existing_cookie_file() {
        let dir = tempdir().unwrap();
        let cookie_path = cookie_file_path(dir.path());
        fs::write(&cookie_path, "tc_stale").unwrap();
        assert!(cookie_path.exists());

        CookieFileGuard::remove_stale(dir.path());
        assert!(!cookie_path.exists());
    }
}
