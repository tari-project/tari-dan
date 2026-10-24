// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tari_ootle_wallet_sdk::storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter};
use thiserror::Error;
use webauthn_rs::{
    Webauthn,
    prelude::{
        AuthenticationResult,
        Credential,
        Passkey,
        PasskeyAuthentication,
        PasskeyRegistration,
        RequestChallengeResponse,
        WebauthnError,
    },
};
use zeroize::Zeroizing;

use crate::services::{SessionData, SessionStore, SessionStoreError};

/// Upper bound on in-progress registration or authentication ceremonies of each kind. Ceremonies are started by
/// unauthenticated callers, and a single-user wallet only ever has a handful in flight. A full store evicts its
/// oldest session, so under sustained ceremony-start traffic an in-flight ceremony survives only this many later
/// starts.
const MAX_PENDING_SESSIONS: usize = 8;

/// Default time-to-live for the one-time setup token required to register the first passkey.
pub const SETUP_TOKEN_TTL: Duration = Duration::from_secs(60 * 60);

/// File name under `<base_path>` where the one-time setup token is written on startup when no passkey is enrolled.
pub const SETUP_TOKEN_FILE_NAME: &str = "setup_token";

#[derive(Debug, Error)]
pub enum WebauthnServiceError {
    #[error("Session store error: {0}")]
    SessionStore(#[from] SessionStoreError),
    #[error("Wallet store error: {0}")]
    WalletStorage(#[from] WalletStorageError),
    #[error("This wallet is already enrolled. A new credential must be added from an authenticated session")]
    AlreadyEnrolled,
    #[error("A valid WebAuthn setup token is required to register the first passkey")]
    InvalidSetupToken,
    #[error("The WebAuthn setup token has expired. Restart the wallet daemon to generate a new setup token")]
    SetupTokenExpired,
    #[error("Setup token I/O error: {0}")]
    SetupTokenIo(#[from] io::Error),
    #[error("The credential that authenticated is not enrolled in this wallet")]
    UnknownCredential,
    #[error(
        "The authenticator's signature counter ({presented}) is not above the last one this wallet saw ({stored}). \
         The credential may have been cloned"
    )]
    CounterNotAdvanced { presented: u32, stored: u32 },
    #[error("This wallet has no enrolled credential to log in with. Register one first")]
    NotEnrolled,
    #[error("Webauthn error: {0}")]
    Webauthn(#[from] WebauthnError),
}

#[derive(Clone)]
struct SetupTokenState {
    token_hash: [u8; 32],
    expires_at: Instant,
    file_path: Option<PathBuf>,
}

impl std::fmt::Debug for SetupTokenState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupTokenState")
            .field("token_hash", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("file_path", &self.file_path)
            .finish()
    }
}

fn generate_setup_token() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    rand::rng().fill_bytes(bytes.as_mut());
    Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_ref()))
}

fn hash_setup_token(token: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().into()
}

fn verify_setup_token_state(
    state: &mut Option<SetupTokenState>,
    presented: Option<&str>,
) -> Result<(), WebauthnServiceError> {
    let current = state.as_ref().ok_or(WebauthnServiceError::InvalidSetupToken)?;
    let presented = presented
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebauthnServiceError::InvalidSetupToken)?;

    let presented_hash = hash_setup_token(presented);
    if !bool::from(current.token_hash.ct_eq(&presented_hash)) {
        return Err(WebauthnServiceError::InvalidSetupToken);
    }

    if Instant::now() >= current.expires_at {
        if let Some(expired) = state.take() &&
            let Some(path) = expired.file_path &&
            let Err(err) = fs::remove_file(&path) &&
            err.kind() != io::ErrorKind::NotFound
        {
            log::warn!("Failed to remove expired setup token file {}: {}", path.display(), err);
        }
        return Err(WebauthnServiceError::SetupTokenExpired);
    }

    Ok(())
}

fn write_setup_token_file(base_path: &Path, token: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(base_path)?;
    let final_path = base_path.join(SETUP_TOKEN_FILE_NAME);
    let tmp_path = base_path.join(format!(".{SETUP_TOKEN_FILE_NAME}.tmp.{}", std::process::id()));
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)?;
    Ok(final_path)
}

/// Removes `<base_path>/setup_token` if it exists.
pub fn remove_setup_token_file(base_path: &Path) -> io::Result<()> {
    let path = base_path.join(SETUP_TOKEN_FILE_NAME);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Registration session data
#[derive(Debug, Clone)]
pub(crate) struct RegistrationSessionData {
    username: String,
    passkey_reg: PasskeyRegistration,
    created_at: Instant,
}

impl RegistrationSessionData {
    pub fn new(username: String, passkey_reg: PasskeyRegistration) -> Self {
        Self {
            username,
            passkey_reg,
            created_at: Instant::now(),
        }
    }

    pub fn passkey_reg(&self) -> &PasskeyRegistration {
        &self.passkey_reg
    }
}

impl SessionData for RegistrationSessionData {
    fn created_at(&self) -> Instant {
        self.created_at
    }
}

/// Authentication session data
#[derive(Debug, Clone)]
struct AuthSessionData {
    passkey_auth: PasskeyAuthentication,
    created_at: Instant,
}

impl AuthSessionData {
    pub fn new(passkey_auth: PasskeyAuthentication) -> Self {
        Self {
            passkey_auth,
            created_at: Instant::now(),
        }
    }
}

impl SessionData for AuthSessionData {
    fn created_at(&self) -> Instant {
        self.created_at
    }
}

/// A service to store temporary registration data (between `start` and `finish` RPC calls)
/// and save the result in DB when finished.
#[derive(Debug, Clone)]
pub struct WebauthnService<TStore> {
    wallet_store: TStore,
    registration_sessions: SessionStore<RegistrationSessionData>,
    auth_sessions: SessionStore<AuthSessionData>,
    setup_token: Arc<Mutex<Option<SetupTokenState>>>,
}

impl<TStore> WebauthnService<TStore>
where TStore: WalletStore
{
    pub fn new(wallet_store: TStore, session_ttl: Duration) -> Self {
        Self {
            wallet_store,
            registration_sessions: SessionStore::new(session_ttl, MAX_PENDING_SESSIONS),
            auth_sessions: SessionStore::new(session_ttl, MAX_PENDING_SESSIONS),
            setup_token: Arc::new(Mutex::new(None)),
        }
    }

    /// True once this wallet holds a credential, whatever username it was enrolled under. The wallet
    /// is single-user, so enrolment is a property of the wallet and the only question the
    /// unauthenticated bootstrap path is allowed to ask.
    pub fn is_enrolled(&self) -> Result<bool, WebauthnServiceError> {
        let mut tx = self.wallet_store.create_read_tx()?;
        Ok(tx.webauthn_has_any_registration()?)
    }

    fn store_setup_token(&self, token: &str, ttl: Duration, file_path: Option<PathBuf>) {
        let state = SetupTokenState {
            token_hash: hash_setup_token(token),
            expires_at: Instant::now() + ttl,
            file_path,
        };
        let mut guard = self.setup_token.lock().expect("setup_token mutex poisoned");
        *guard = Some(state);
    }

    /// Mints a fresh in-memory setup token with the default [`SETUP_TOKEN_TTL`].
    #[cfg(test)]
    pub fn issue_setup_token(&self) -> Zeroizing<String> {
        self.issue_setup_token_with_ttl(SETUP_TOKEN_TTL, None)
    }

    /// Mints a fresh setup token with a custom TTL and optional file path to delete on consumption.
    #[cfg(test)]
    pub fn issue_setup_token_with_ttl(&self, ttl: Duration, file_path: Option<PathBuf>) -> Zeroizing<String> {
        let token = generate_setup_token();
        self.store_setup_token(&token, ttl, file_path);
        token
    }

    /// Mints a fresh setup token, writes it to `<base_path>/setup_token` (mode `0600` on Unix), and stores its
    /// SHA-256 digest in memory for verification during first-passkey registration.
    pub fn issue_setup_token_to_file(
        &self,
        base_path: &Path,
    ) -> Result<(Zeroizing<String>, PathBuf), WebauthnServiceError> {
        let token = generate_setup_token();
        let file_path = write_setup_token_file(base_path, &token)?;
        self.store_setup_token(&token, SETUP_TOKEN_TTL, Some(file_path.clone()));
        Ok((token, file_path))
    }

    /// Verifies that `presented` matches the currently active, non-expired one-time setup token in constant time.
    pub fn verify_setup_token(&self, presented: Option<&str>) -> Result<(), WebauthnServiceError> {
        let mut guard = self.setup_token.lock().expect("setup_token mutex poisoned");
        verify_setup_token_state(&mut guard, presented)
    }

    /// Start registration by creating a new session and save the temporary [`PasskeyRegistration`].
    pub async fn start_registration(
        &self,
        username: String,
        passkey_reg: PasskeyRegistration,
    ) -> Result<String, WebauthnServiceError> {
        Ok(self
            .registration_sessions
            .add(RegistrationSessionData::new(username, passkey_reg))
            .await?)
    }

    /// Retrieve [`PasskeyRegistration`] by session ID.
    pub async fn get_session(&self, session_id: &str) -> Result<RegistrationSessionData, WebauthnServiceError> {
        let session = self.registration_sessions.get(session_id).await?;
        Ok(session)
    }

    /// Finalizing registration, remove session from store and save passkey (public key of credential) to DB.
    ///
    /// This is the enrolment boundary: the wallet takes exactly one credential, and the check that it
    /// has none and that the one-time setup token is still valid runs in the same write transaction as
    /// the insert. Any check made before this one is a courtesy rejection — two concurrent finishes both
    /// pass it and only this transaction separates them. Consumes the setup token and deletes
    /// `<base_path>/setup_token` when registration finishes.
    pub async fn finish_registration(
        &self,
        session_id: String,
        passkey: Passkey,
        setup_token: Option<&str>,
    ) -> Result<(), WebauthnServiceError> {
        let session = self.registration_sessions.get(session_id.as_str()).await?;
        let file_to_remove = self.wallet_store.with_write_tx(|tx| {
            if tx.webauthn_has_any_registration()? {
                return Err(WebauthnServiceError::AlreadyEnrolled);
            }
            let mut guard = self.setup_token.lock().expect("setup_token mutex poisoned");
            verify_setup_token_state(&mut guard, setup_token)?;
            tx.webauthn_reg_insert(session.username, passkey)?;
            let consumed = guard.take();
            Ok(consumed.and_then(|s| s.file_path))
        })?;
        let _ = self.registration_sessions.remove(session_id.as_str()).await;
        if let Some(path) = file_to_remove &&
            let Err(err) = fs::remove_file(&path) &&
            err.kind() != io::ErrorKind::NotFound
        {
            log::warn!("Failed to remove consumed setup token file {}: {}", path.display(), err);
        }
        Ok(())
    }

    /// Every passkey this wallet holds.
    pub fn wallet_passkeys(&self) -> Result<Vec<Passkey>, WebauthnServiceError> {
        let mut tx = self.wallet_store.create_read_tx()?;
        Ok(tx.webauthn_reg_fetch_all_passkeys()?)
    }

    /// Start a login ceremony that any of this wallet's passkeys can complete, returning the challenge
    /// and the session ID that finishes it. The wallet is single-user, so the ceremony is the wallet's
    /// and nothing the caller supplies selects the credentials it offers.
    pub async fn start_authentication(
        &self,
        webauthn: &Webauthn,
    ) -> Result<(RequestChallengeResponse, String), WebauthnServiceError> {
        let passkeys = self.wallet_passkeys()?;
        if passkeys.is_empty() {
            return Err(WebauthnServiceError::NotEnrolled);
        }
        let (challenge, passkey_auth) = webauthn.start_passkey_authentication(passkeys.as_slice())?;
        let session_id = self.auth_sessions.add(AuthSessionData::new(passkey_auth)).await?;
        Ok((challenge, session_id))
    }

    pub async fn auth_passkey(&self, session_id: &str) -> Result<PasskeyAuthentication, WebauthnServiceError> {
        Ok(self.auth_sessions.get(session_id).await?.passkey_auth.clone())
    }

    /// Checks a verified assertion's signature counter against the stored credential and stores the
    /// advanced counter. The login must be refused on error.
    ///
    /// A ceremony checks the counter against the credential as it stood when the ceremony started,
    /// and ceremonies stay open for the session TTL. A login completed in between advances the stored
    /// counter, so this check against the stored value is the one that catches an assertion from a
    /// cloned authenticator. As in the WebAuthn rule, a credential whose counter is zero on both
    /// sides has no counter, as is usual for synced passkeys, and is not checked.
    pub fn record_authentication(&self, result: &AuthenticationResult) -> Result<(), WebauthnServiceError> {
        self.wallet_store.with_write_tx(|tx| {
            let mut passkey = tx
                .webauthn_reg_fetch_all_passkeys()?
                .into_iter()
                .find(|passkey| passkey.cred_id() == result.cred_id())
                .ok_or(WebauthnServiceError::UnknownCredential)?;
            let stored = Credential::from(passkey.clone()).counter;
            let presented = result.counter();
            if (presented > 0 || stored > 0) && presented <= stored {
                return Err(WebauthnServiceError::CounterNotAdvanced { presented, stored });
            }
            if passkey.update_credential(result) == Some(true) {
                tx.webauthn_reg_update_passkey(&passkey)?;
            }
            Ok(())
        })
    }

    pub async fn finish_authentication(&self, session_id: &str) -> Result<(), WebauthnServiceError> {
        self.auth_sessions.remove(session_id).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
    use url::Url;
    use uuid::Uuid;
    use webauthn_rs::{WebauthnBuilder, prelude::CredentialID};

    use super::*;

    /// A credential in the shape the store round-trips. The enrolment gate turns on a registration
    /// being present, so no signature over this key is ever checked.
    fn passkey() -> Passkey {
        serde_json::from_value(serde_json::json!({
            "cred": {
                "cred_id": "AAECAwQFBgcICQoLDA0ODw",
                "cred": {
                    "type_": "ES256",
                    "key": {
                        "EC_EC2": {
                            "curve": "SECP256R1",
                            "x": "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
                            "y": "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
                        }
                    }
                },
                "counter": 0,
                "transports": null,
                "user_verified": true,
                "backup_eligible": false,
                "backup_state": false,
                "registration_policy": "required",
                "extensions": {},
                "attestation": { "data": "None", "metadata": "None" },
                "attestation_format": "none"
            }
        }))
        .unwrap()
    }

    fn webauthn() -> Webauthn {
        WebauthnBuilder::new("localhost", &Url::parse("http://localhost:5100").unwrap())
            .unwrap()
            .build()
            .unwrap()
    }

    /// The in-progress registration state a real `reg_start` produces. The service only carries it
    /// from session to session, it never inspects it.
    fn passkey_reg(username: &str) -> PasskeyRegistration {
        let (_, passkey_reg) = webauthn()
            .start_passkey_registration(Uuid::new_v4(), username, username, None)
            .unwrap();
        passkey_reg
    }

    fn service() -> (WebauthnService<SqliteWalletStore>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();
        (WebauthnService::new(store, Duration::from_secs(60)), temp)
    }

    async fn enrol(service: &WebauthnService<SqliteWalletStore>, username: &str) -> Result<(), WebauthnServiceError> {
        let token = service.issue_setup_token();
        let session_id = service
            .start_registration(username.to_string(), passkey_reg(username))
            .await?;
        service
            .finish_registration(session_id, passkey(), Some(token.as_str()))
            .await
    }

    #[tokio::test]
    async fn first_enrolment_succeeds() {
        let (service, _temp) = service();
        assert!(!service.is_enrolled().unwrap());
        enrol(&service, "owner").await.unwrap();
        assert!(service.is_enrolled().unwrap());
    }

    fn allowed_credential_ids(challenge: &RequestChallengeResponse) -> Vec<serde_json::Value> {
        challenge
            .public_key
            .allow_credentials
            .iter()
            .map(|allowed| serde_json::to_value(&allowed.id).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn login_challenge_offers_the_wallet_credential() {
        let (service, _temp) = service();
        enrol(&service, "owner").await.unwrap();

        let (challenge, _) = service.start_authentication(&webauthn()).await.unwrap();
        assert_eq!(allowed_credential_ids(&challenge), vec![
            serde_json::to_value(passkey().cred_id()).unwrap()
        ]);
    }

    #[tokio::test]
    async fn login_is_refused_before_enrolment() {
        let (service, _temp) = service();
        let err = service.start_authentication(&webauthn()).await.unwrap_err();
        assert!(matches!(err, WebauthnServiceError::NotEnrolled));
    }

    #[tokio::test]
    async fn enrolment_under_another_username_is_refused() {
        let (service, _temp) = service();
        enrol(&service, "owner").await.unwrap();

        let err = enrol(&service, "attacker").await.unwrap_err();
        assert!(matches!(err, WebauthnServiceError::AlreadyEnrolled));
        assert_eq!(service.wallet_passkeys().unwrap().len(), 1);
    }

    /// What `finish_passkey_authentication` reports for a ceremony by `cred_id` at `counter`.
    fn authentication_result(cred_id: &CredentialID, counter: u32) -> AuthenticationResult {
        serde_json::from_value(serde_json::json!({
            "cred_id": cred_id,
            "needs_update": true,
            "user_verified": true,
            "backup_state": false,
            "backup_eligible": false,
            "counter": counter,
            "extensions": {},
        }))
        .unwrap()
    }

    /// A ceremony by the enrolled credential at `counter`.
    fn enrolled_at(counter: u32) -> AuthenticationResult {
        authentication_result(passkey().cred_id(), counter)
    }

    fn stored_counter(service: &WebauthnService<SqliteWalletStore>) -> u32 {
        let passkeys = service.wallet_passkeys().unwrap();
        assert_eq!(passkeys.len(), 1);
        Credential::from(passkeys[0].clone()).counter
    }

    #[tokio::test]
    async fn an_authentication_advances_the_stored_counter() {
        let (service, _temp) = service();
        enrol(&service, "owner").await.unwrap();
        assert_eq!(stored_counter(&service), 0);

        service.record_authentication(&enrolled_at(5)).unwrap();
        assert_eq!(stored_counter(&service), 5);
        service.record_authentication(&enrolled_at(6)).unwrap();
        assert_eq!(stored_counter(&service), 6);
    }

    /// A ceremony started before a later login completed checked its counter against the credential
    /// as it stood at its start. Completing it with a counter at or below the one that login stored
    /// is what a cloned authenticator does, and must be refused.
    #[tokio::test]
    async fn a_counter_at_or_below_the_stored_one_is_refused() {
        let (service, _temp) = service();
        enrol(&service, "owner").await.unwrap();
        service.record_authentication(&enrolled_at(10)).unwrap();

        for counter in [3, 10] {
            let err = service.record_authentication(&enrolled_at(counter)).unwrap_err();
            assert!(
                matches!(err, WebauthnServiceError::CounterNotAdvanced { presented, stored: 10 } if presented == counter),
                "{counter}: {err}"
            );
        }
        assert_eq!(stored_counter(&service), 10);
    }

    #[tokio::test]
    async fn a_credential_without_a_counter_is_not_checked() {
        let (service, _temp) = service();
        enrol(&service, "owner").await.unwrap();

        service.record_authentication(&enrolled_at(0)).unwrap();
        service.record_authentication(&enrolled_at(0)).unwrap();
        assert_eq!(stored_counter(&service), 0);
    }

    #[tokio::test]
    async fn an_authentication_by_an_unknown_credential_is_refused() {
        let (service, _temp) = service();
        enrol(&service, "owner").await.unwrap();

        let unknown = CredentialID::from(vec![0xff; 16]);
        let err = service
            .record_authentication(&authentication_result(&unknown, 5))
            .unwrap_err();
        assert!(matches!(err, WebauthnServiceError::UnknownCredential));
        assert_eq!(stored_counter(&service), 0);
    }

    /// Two enrolments that both passed any earlier check still end with one credential, because the
    /// count and the insert share a write transaction. Atomicity here comes from [`SqliteWalletStore`]
    /// holding its single connection's mutex for the transaction's lifetime: were the store ever to
    /// move to a connection pool, the deferred `BEGIN` would let two readers both count zero.
    #[tokio::test]
    async fn second_enrolment_is_refused_within_the_write_tx() {
        let (service, _temp) = service();
        let token = service.issue_setup_token();

        let first = service
            .start_registration("owner".to_string(), passkey_reg("owner"))
            .await
            .unwrap();
        let second = service
            .start_registration("attacker".to_string(), passkey_reg("attacker"))
            .await
            .unwrap();

        let (a, b) = tokio::join!(
            service.finish_registration(first, passkey(), Some(token.as_str())),
            service.finish_registration(second, passkey(), Some(token.as_str()))
        );

        assert_eq!(
            [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
            1,
            "exactly one of the two enrolments must be accepted"
        );
        assert_eq!(service.wallet_passkeys().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn setup_token_is_required_and_never_echoed_in_errors() {
        let (service, _temp) = service();
        let issued = service.issue_setup_token();

        // Missing token on early check
        let err = service.verify_setup_token(None).unwrap_err();
        assert!(matches!(err, WebauthnServiceError::InvalidSetupToken));

        // Wrong token on early check
        let wrong = "wrong-secret-token-value-12345";
        let err = service.verify_setup_token(Some(wrong)).unwrap_err();
        assert!(matches!(err, WebauthnServiceError::InvalidSetupToken));
        assert!(!err.to_string().contains(wrong));
        assert!(!err.to_string().contains(issued.as_str()));

        // Valid token on early check, but missing/wrong on finish_registration
        service.verify_setup_token(Some(issued.as_str())).unwrap();
        let session_id = service
            .start_registration("owner".to_string(), passkey_reg("owner"))
            .await
            .unwrap();
        let err = service
            .finish_registration(session_id.clone(), passkey(), Some(wrong))
            .await
            .unwrap_err();
        assert!(matches!(err, WebauthnServiceError::InvalidSetupToken));
        assert!(!err.to_string().contains(wrong));
        assert!(!err.to_string().contains(issued.as_str()));
        assert!(!service.is_enrolled().unwrap());

        // Session was not consumed because token check failed before session removal
        service
            .finish_registration(session_id, passkey(), Some(issued.as_str()))
            .await
            .unwrap();
        assert!(service.is_enrolled().unwrap());
    }

    #[tokio::test]
    async fn setup_token_file_is_created_and_deleted_on_consumption() {
        let (service, temp) = service();
        let (token, path) = service.issue_setup_token_to_file(temp.path()).unwrap();
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), token.as_str());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let session_id = service
            .start_registration("owner".to_string(), passkey_reg("owner"))
            .await
            .unwrap();
        // Still exists until finish_registration succeeds
        assert!(path.exists());

        service
            .finish_registration(session_id, passkey(), Some(token.as_str()))
            .await
            .unwrap();
        assert!(!path.exists(), "setup_token file must be deleted when consumed");
        assert!(
            service.verify_setup_token(Some(token.as_str())).is_err(),
            "consumed setup token must be rejected"
        );
    }

    #[tokio::test]
    async fn expired_setup_token_is_rejected_and_removes_file() {
        let (service, temp) = service();
        let file_path = write_setup_token_file(temp.path(), "placeholder").unwrap();
        let token = service.issue_setup_token_with_ttl(Duration::ZERO, Some(file_path.clone()));
        fs::write(&file_path, token.as_bytes()).unwrap();

        let err = service.verify_setup_token(Some(token.as_str())).unwrap_err();
        assert!(matches!(err, WebauthnServiceError::SetupTokenExpired));
        assert!(!err.to_string().contains(token.as_str()));
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn reset_wallet_passkeys_clears_credentials_and_allows_fresh_setup_token_enrolment() {
        let temp = tempfile::tempdir().unwrap();
        let mut common = tari_common::configuration::CommonConfig::default();
        common.base_path = temp.path().to_path_buf();
        let config = crate::config::ApplicationConfig {
            common,
            ootle_wallet_daemon: crate::config::WalletDaemonConfig::default(),
        };
        let store = crate::init_wallet_store(&config).unwrap();
        let service = WebauthnService::new(store, Duration::from_secs(60));

        enrol(&service, "owner").await.unwrap();
        assert!(service.is_enrolled().unwrap());

        // Write a stale setup_token file to verify reset_wallet_passkeys removes it too
        let stale_path = write_setup_token_file(temp.path(), "stale").unwrap();
        assert!(stale_path.exists());

        let removed = crate::reset_wallet_passkeys(&config).unwrap();
        assert_eq!(removed, 1);
        assert!(!service.is_enrolled().unwrap());
        assert!(!stale_path.exists());

        // Without a new setup token, registration is refused
        let err = service.verify_setup_token(None).unwrap_err();
        assert!(matches!(err, WebauthnServiceError::InvalidSetupToken));

        // Issuing a new setup token on restart allows registering a new passkey
        let (new_token, token_path) = service.issue_setup_token_to_file(temp.path()).unwrap();
        let session_id = service
            .start_registration("new_owner".to_string(), passkey_reg("new_owner"))
            .await
            .unwrap();
        service
            .finish_registration(session_id, passkey(), Some(new_token.as_str()))
            .await
            .unwrap();
        assert!(service.is_enrolled().unwrap());
        assert!(!token_path.exists());
    }
}
