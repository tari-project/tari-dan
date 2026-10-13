// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, Instant};

use tari_ootle_wallet_sdk::storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter};
use thiserror::Error;
use webauthn_rs::prelude::{Passkey, PasskeyAuthentication, PasskeyRegistration};

use crate::services::{SessionData, SessionStore, SessionStoreError};

/// Upper bound on in-progress registration or authentication ceremonies of each kind. Ceremonies are started by
/// unauthenticated callers, and a single-user wallet only ever has a handful in flight. A full store evicts its
/// oldest session, so under sustained ceremony-start traffic an in-flight ceremony survives only this many later
/// starts.
const MAX_PENDING_SESSIONS: usize = 8;

#[derive(Debug, Error)]
pub enum WebauthnServiceError {
    #[error("Session store error: {0}")]
    SessionStore(#[from] SessionStoreError),
    #[error("Wallet store error: {0}")]
    WalletStorage(#[from] WalletStorageError),
    #[error("This wallet is already enrolled. A new credential must be added from an authenticated session")]
    AlreadyEnrolled,
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
}

impl<TStore> WebauthnService<TStore>
where TStore: WalletStore
{
    pub fn new(wallet_store: TStore, session_ttl: Duration) -> Self {
        Self {
            wallet_store,
            registration_sessions: SessionStore::new(session_ttl, MAX_PENDING_SESSIONS),
            auth_sessions: SessionStore::new(session_ttl, MAX_PENDING_SESSIONS),
        }
    }

    /// True once this wallet holds a credential, whatever username it was enrolled under. The wallet
    /// is single-user, so enrolment is a property of the wallet and the only question the
    /// unauthenticated bootstrap path is allowed to ask.
    pub fn is_enrolled(&self) -> Result<bool, WebauthnServiceError> {
        let mut tx = self.wallet_store.create_read_tx()?;
        Ok(tx.webauthn_has_any_registration()?)
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
    /// has none runs in the same write transaction as the insert. Any check made before this one is a
    /// courtesy rejection — two concurrent finishes both pass it and only this transaction separates
    /// them.
    pub async fn finish_registration(&self, session_id: String, passkey: Passkey) -> Result<(), WebauthnServiceError> {
        let session = self.registration_sessions.remove(session_id.as_str()).await?;
        self.wallet_store.with_write_tx(|tx| {
            if tx.webauthn_has_any_registration()? {
                return Err(WebauthnServiceError::AlreadyEnrolled);
            }
            tx.webauthn_reg_insert(session.username, passkey)?;
            Ok(())
        })
    }

    /// Fetch passkeys for a username.
    pub fn passkeys(&self, username: String) -> Result<Vec<Passkey>, WebauthnServiceError> {
        let mut tx = self.wallet_store.create_read_tx()?;
        Ok(tx.webauthn_reg_fetch_passkeys(username)?)
    }

    pub async fn start_authentication(
        &self,
        passkey_auth: PasskeyAuthentication,
    ) -> Result<String, WebauthnServiceError> {
        Ok(self.auth_sessions.add(AuthSessionData::new(passkey_auth)).await?)
    }

    pub async fn auth_passkey(&self, session_id: &str) -> Result<PasskeyAuthentication, WebauthnServiceError> {
        Ok(self.auth_sessions.get(session_id).await?.passkey_auth.clone())
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
    use webauthn_rs::WebauthnBuilder;

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

    /// The in-progress registration state a real `reg_start` produces. The service only carries it
    /// from session to session, it never inspects it.
    fn passkey_reg(username: &str) -> PasskeyRegistration {
        let webauthn = WebauthnBuilder::new("localhost", &Url::parse("http://localhost:5100").unwrap())
            .unwrap()
            .build()
            .unwrap();
        let (_, passkey_reg) = webauthn
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
        let session_id = service
            .start_registration(username.to_string(), passkey_reg(username))
            .await
            .unwrap();
        service.finish_registration(session_id, passkey()).await
    }

    #[tokio::test]
    async fn first_enrolment_succeeds() {
        let (service, _temp) = service();
        assert!(!service.is_enrolled().unwrap());
        enrol(&service, "owner").await.unwrap();
        assert!(service.is_enrolled().unwrap());
    }

    #[tokio::test]
    async fn enrolment_under_another_username_is_refused() {
        let (service, _temp) = service();
        enrol(&service, "owner").await.unwrap();

        let err = enrol(&service, "attacker").await.unwrap_err();
        assert!(matches!(err, WebauthnServiceError::AlreadyEnrolled));
        assert!(service.passkeys("attacker".to_string()).unwrap().is_empty());
    }

    /// Two enrolments that both passed any earlier check still end with one credential, because the
    /// count and the insert share a write transaction. Atomicity here comes from [`SqliteWalletStore`]
    /// holding its single connection's mutex for the transaction's lifetime: were the store ever to
    /// move to a connection pool, the deferred `BEGIN` would let two readers both count zero.
    #[tokio::test]
    async fn second_enrolment_is_refused_within_the_write_tx() {
        let (service, _temp) = service();

        let first = service
            .start_registration("owner".to_string(), passkey_reg("owner"))
            .await
            .unwrap();
        let second = service
            .start_registration("attacker".to_string(), passkey_reg("attacker"))
            .await
            .unwrap();

        let (a, b) = tokio::join!(
            service.finish_registration(first, passkey()),
            service.finish_registration(second, passkey())
        );

        assert_eq!(
            [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
            1,
            "exactly one of the two enrolments must be accepted"
        );
        assert!(
            service.passkeys("owner".to_string()).unwrap().is_empty() !=
                service.passkeys("attacker".to_string()).unwrap().is_empty()
        );
    }
}
