// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, Instant};

use tari_ootle_wallet_sdk::{
    models::WebauthnRegistrationPasskeyModel,
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};
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

use crate::services::{SessionData, SessionStore, SessionStoreError};

/// Upper bound on in-progress registration or authentication ceremonies of each kind. Ceremonies are started by
/// unauthenticated callers, and a single-user wallet only ever has a handful in flight. A full store evicts its
/// oldest session, so under sustained ceremony-start traffic an in-flight ceremony survives only this many later
/// starts.
const MAX_PENDING_SESSIONS: usize = 8;

const DEFAULT_PASSKEY_LABEL_PREFIX: &str = "Passkey";

#[derive(Debug, Error)]
pub enum WebauthnServiceError {
    #[error("Session store error: {0}")]
    SessionStore(#[from] SessionStoreError),
    #[error("Wallet store error: {0}")]
    WalletStorage(#[from] WalletStorageError),
    #[error("This wallet is already enrolled. A new credential must be added from an authenticated session")]
    AlreadyEnrolled,
    #[error("This credential is already registered in this wallet")]
    DuplicateCredential,
    #[error("Cannot remove the last passkey from the wallet")]
    CannotRemoveLastPasskey,
    #[error("Passkey {id} was not found")]
    PasskeyNotFound { id: u32 },
    #[error("Passkey label must not be empty")]
    EmptyLabel,
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

/// Registration session data
#[derive(Debug, Clone)]
pub(crate) struct RegistrationSessionData {
    username: String,
    label: Option<String>,
    passkey_reg: PasskeyRegistration,
    allow_existing: bool,
    created_at: Instant,
}

impl RegistrationSessionData {
    pub fn new(
        username: String,
        label: Option<String>,
        passkey_reg: PasskeyRegistration,
        allow_existing: bool,
    ) -> Self {
        Self {
            username,
            label,
            passkey_reg,
            allow_existing,
            created_at: Instant::now(),
        }
    }

    pub fn passkey_reg(&self) -> &PasskeyRegistration {
        &self.passkey_reg
    }

    pub fn allow_existing(&self) -> bool {
        self.allow_existing
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

    /// Start registration by creating a new session and saving the temporary [`PasskeyRegistration`],
    /// optional human-readable `label`, and whether existing credentials are allowed (`true` when
    /// initiated by an authenticated `Admin` user session).
    pub async fn start_registration(
        &self,
        username: String,
        label: Option<String>,
        passkey_reg: PasskeyRegistration,
        allow_existing: bool,
    ) -> Result<String, WebauthnServiceError> {
        Ok(self
            .registration_sessions
            .add(RegistrationSessionData::new(
                username,
                label,
                passkey_reg,
                allow_existing,
            ))
            .await?)
    }

    /// Retrieve [`RegistrationSessionData`] by session ID.
    pub async fn get_session(&self, session_id: &str) -> Result<RegistrationSessionData, WebauthnServiceError> {
        let session = self.registration_sessions.get(session_id).await?;
        Ok(session)
    }

    /// Finalizing registration, remove session from store, save passkey (public key of credential) to DB,
    /// and return the newly inserted passkey row ID.
    ///
    /// For an unauthenticated bootstrap ceremony (`allow_existing == false`), this is the enrolment boundary:
    /// the check that the wallet has no credential runs in the same write transaction as the insert. Any check
    /// made before this one is a courtesy rejection — two concurrent finishes both pass it and only this
    /// transaction separates them.
    pub async fn finish_registration(&self, session_id: String, passkey: Passkey) -> Result<u32, WebauthnServiceError> {
        let session = self.registration_sessions.remove(session_id.as_str()).await?;
        self.wallet_store.with_write_tx(|tx| {
            let existing = tx.webauthn_reg_list_passkeys()?;
            if !session.allow_existing && !existing.is_empty() {
                return Err(WebauthnServiceError::AlreadyEnrolled);
            }
            if existing
                .iter()
                .any(|stored| stored.passkey.cred_id() == passkey.cred_id())
            {
                return Err(WebauthnServiceError::DuplicateCredential);
            }
            let resolved_label = session
                .label
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    let mut next = existing.iter().map(|p| p.id).max().unwrap_or(0) + 1;
                    while existing
                        .iter()
                        .any(|p| p.label == format!("{DEFAULT_PASSKEY_LABEL_PREFIX} {next}"))
                    {
                        next += 1;
                    }
                    format!("{DEFAULT_PASSKEY_LABEL_PREFIX} {next}")
                });
            let id = tx.webauthn_reg_insert(session.username, resolved_label, passkey)?;
            Ok(id)
        })
    }

    /// Every passkey this wallet holds.
    pub fn wallet_passkeys(&self) -> Result<Vec<Passkey>, WebauthnServiceError> {
        let mut tx = self.wallet_store.create_read_tx()?;
        Ok(tx.webauthn_reg_fetch_all_passkeys()?)
    }

    /// Every passkey metadata entry this wallet holds, ordered by creation / id.
    pub fn list_passkeys(&self) -> Result<Vec<WebauthnRegistrationPasskeyModel>, WebauthnServiceError> {
        let mut tx = self.wallet_store.create_read_tx()?;
        Ok(tx.webauthn_reg_list_passkeys()?)
    }

    /// Rename a registered passkey by ID.
    pub fn rename_passkey(&self, id: u32, label: String) -> Result<(), WebauthnServiceError> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(WebauthnServiceError::EmptyLabel);
        }
        self.wallet_store.with_write_tx(|tx| {
            tx.webauthn_reg_rename_passkey(id, trimmed).map_err(|e| match e {
                WalletStorageError::NotFound { .. } => WebauthnServiceError::PasskeyNotFound { id },
                other => WebauthnServiceError::WalletStorage(other),
            })
        })
    }

    /// Remove a registered passkey by ID. Refuses to remove the last remaining passkey in the wallet.
    pub fn remove_passkey(&self, id: u32) -> Result<(), WebauthnServiceError> {
        self.wallet_store.with_write_tx(|tx| {
            let passkeys = tx.webauthn_reg_list_passkeys()?;
            if !passkeys.iter().any(|entry| entry.id == id) {
                return Err(WebauthnServiceError::PasskeyNotFound { id });
            }
            if passkeys.len() <= 1 {
                return Err(WebauthnServiceError::CannotRemoveLastPasskey);
            }
            tx.webauthn_reg_delete_passkey(id)?;
            Ok(())
        })
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
    /// advanced counter and `last_used_at` timestamp. The login must be refused on error.
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
            let _ = passkey.update_credential(result);
            tx.webauthn_reg_update_passkey(&passkey)?;
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

    async fn enrol(service: &WebauthnService<SqliteWalletStore>, username: &str) -> Result<u32, WebauthnServiceError> {
        let session_id = service
            .start_registration(username.to_string(), None, passkey_reg(username), false)
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

        let first = service
            .start_registration("owner".to_string(), None, passkey_reg("owner"), false)
            .await
            .unwrap();
        let second = service
            .start_registration("attacker".to_string(), None, passkey_reg("attacker"), false)
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
        assert_eq!(service.wallet_passkeys().unwrap().len(), 1);
    }

    fn passkey_with_cred_id(cred_id_b64: &str) -> Passkey {
        serde_json::from_value(serde_json::json!({
            "cred": {
                "cred_id": cred_id_b64,
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

    async fn enrol_additional(
        service: &WebauthnService<SqliteWalletStore>,
        username: &str,
        label: Option<&str>,
        pk: Passkey,
    ) -> Result<u32, WebauthnServiceError> {
        let session_id = service
            .start_registration(
                username.to_string(),
                label.map(str::to_string),
                passkey_reg(username),
                true,
            )
            .await
            .unwrap();
        service.finish_registration(session_id, pk).await
    }

    #[tokio::test]
    async fn authenticated_session_can_register_multiple_passkeys_and_login_offers_all() {
        let (service, _temp) = service();

        // First bootstrap registration generates "Passkey 1" when no label is given
        let first_session = service
            .start_registration("owner".to_string(), None, passkey_reg("owner"), false)
            .await
            .unwrap();
        let first_pk = passkey_with_cred_id("AAECAwQFBgcICQoLDA0ODw");
        let first_id = service
            .finish_registration(first_session, first_pk.clone())
            .await
            .unwrap();

        // Second registration under the same username with explicit label
        let second_pk = passkey_with_cred_id("EBESExQVFhcYGRobHB0eHw");
        let second_id = enrol_additional(&service, "owner", Some("YubiKey Backup"), second_pk.clone())
            .await
            .unwrap();
        assert_ne!(first_id, second_id);

        // Re-registering the same credential ID is rejected
        let dup_err = enrol_additional(&service, "owner", Some("Duplicate"), second_pk.clone())
            .await
            .unwrap_err();
        assert!(matches!(dup_err, WebauthnServiceError::DuplicateCredential));

        // Listing returns both passkeys in order with their labels
        let listed = service.list_passkeys().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, first_id);
        assert_eq!(listed[0].label, "Passkey 1");
        assert!(listed[0].last_used_at.is_none());
        assert_eq!(listed[1].id, second_id);
        assert_eq!(listed[1].label, "YubiKey Backup");
        assert!(listed[1].last_used_at.is_none());

        // Login challenge offers both credentials
        let (challenge, _) = service.start_authentication(&webauthn()).await.unwrap();
        assert_eq!(allowed_credential_ids(&challenge), vec![
            serde_json::to_value(first_pk.cred_id()).unwrap(),
            serde_json::to_value(second_pk.cred_id()).unwrap(),
        ]);

        // Counters and last_used_at are tracked independently per passkey
        service
            .record_authentication(&authentication_result(second_pk.cred_id(), 7))
            .unwrap();
        let listed = service.list_passkeys().unwrap();
        assert_eq!(Credential::from(listed[0].passkey.clone()).counter, 0);
        assert!(listed[0].last_used_at.is_none());
        assert_eq!(Credential::from(listed[1].passkey.clone()).counter, 7);
        assert!(listed[1].last_used_at.is_some());

        // Zero-counter synced passkey still records last_used_at on login
        service
            .record_authentication(&authentication_result(first_pk.cred_id(), 0))
            .unwrap();
        let listed = service.list_passkeys().unwrap();
        assert!(listed[0].last_used_at.is_some());
    }

    #[tokio::test]
    async fn rename_and_remove_passkeys_enforce_invariants_and_default_labels_do_not_repeat() {
        let (service, _temp) = service();
        let first_id = enrol(&service, "owner").await.unwrap();

        let second_pk = passkey_with_cred_id("EBESExQVFhcYGRobHB0eHw");
        let second_id = enrol_additional(&service, "owner", None, second_pk).await.unwrap();
        let listed = service.list_passkeys().unwrap();
        assert_eq!(listed[0].label, "Passkey 1");
        assert_eq!(listed[1].label, "Passkey 2");

        // Remove first passkey ("Passkey 1") while "Passkey 2" (id=2) remains, then add a third without a label.
        // The new default label must not repeat "Passkey 2".
        service.remove_passkey(first_id).unwrap();
        let third_pk = passkey_with_cred_id("ICQoLDA0ODxAREhMUFRYXA");
        let third_id = enrol_additional(&service, "owner", None, third_pk).await.unwrap();
        let listed = service.list_passkeys().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, second_id);
        assert_eq!(listed[0].label, "Passkey 2");
        assert_eq!(listed[1].id, third_id);
        assert_eq!(listed[1].label, "Passkey 3");

        // Rename second passkey
        service
            .rename_passkey(second_id, "  Hardware Security Key  ".to_string())
            .unwrap();
        let listed = service.list_passkeys().unwrap();
        assert_eq!(listed[0].label, "Hardware Security Key");

        // Empty label is rejected
        let empty_err = service.rename_passkey(second_id, "   ".to_string()).unwrap_err();
        assert!(matches!(empty_err, WebauthnServiceError::EmptyLabel));

        // Unknown ID is rejected on rename and remove
        let missing_rename = service.rename_passkey(9999, "Valid".to_string()).unwrap_err();
        assert!(matches!(missing_rename, WebauthnServiceError::PasskeyNotFound {
            id: 9999
        }));
        let missing_remove = service.remove_passkey(9999).unwrap_err();
        assert!(matches!(missing_remove, WebauthnServiceError::PasskeyNotFound {
            id: 9999
        }));

        // Removing one of two passkeys succeeds
        service.remove_passkey(third_id).unwrap();
        let remaining = service.list_passkeys().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, second_id);

        // Removing the last remaining passkey is refused
        let last_err = service.remove_passkey(second_id).unwrap_err();
        assert!(matches!(last_err, WebauthnServiceError::CannotRemoveLastPasskey));
        assert_eq!(service.list_passkeys().unwrap().len(), 1);
    }
}
