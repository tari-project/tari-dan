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
        WebauthnListPasskeysRequest,
        WebauthnListPasskeysResponse,
        WebauthnPasskeyInfo,
        WebauthnRemovePasskeyRequest,
        WebauthnRemovePasskeyResponse,
        WebauthnRenamePasskeyRequest,
        WebauthnRenamePasskeyResponse,
        WebauthnStartAuthRequest,
        WebauthnStartAuthResponse,
        WebauthnStartRegisterRequest,
        WebauthnStartRegisterResponse,
    },
};
use uuid::Uuid;
use webauthn_rs::{Webauthn, prelude::Credential};

use crate::{
    handlers::{
        HandlerContext,
        auth::REFRESH_TOKEN_COOKIE,
        helpers::{invalid_params, invalid_request, not_found},
    },
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

/// Authorizes adding an additional passkey to an already-enrolled wallet. Requires an interactive
/// `Admin` user session (`authorize_user_only`), rejecting unauthenticated callers with the
/// enrolment-gate message and rejecting API keys or WebRTC-delegated tokens with `UserAuthOnly`.
fn authorize_enrolled_passkey_addition(context: &HandlerContext, token: Option<&Bearer>) -> Result<(), anyhow::Error> {
    if token.is_none() {
        return Err(invalid_request(
            "This wallet is already enrolled. A new credential must be added from an authenticated session",
        ));
    }
    context.authorize_user_only(token, &[Permission::Admin])?;
    Ok(())
}

pub async fn handle_start_registration(
    context: &HandlerContext,
    token: Option<&Bearer>,
    request: WebauthnStartRegisterRequest,
) -> Result<WebauthnStartRegisterResponse, anyhow::Error> {
    let webauthn = webauthn(context)?;
    let webauthn_service = webauthn_service(context)?;
    let existing_passkeys = webauthn_service.wallet_passkeys()?;
    let allow_existing = !existing_passkeys.is_empty();
    let exclude_credentials = if allow_existing {
        authorize_enrolled_passkey_addition(context, token)?;
        Some(existing_passkeys.into_iter().map(|pk| pk.cred_id().clone()).collect())
    } else {
        None
    };

    let (response, passkey_reg) = webauthn.start_passkey_registration(
        Uuid::new_v4(),
        request.username.as_str(),
        request.username.as_str(),
        exclude_credentials,
    )?;

    let session_id = webauthn_service
        .start_registration(request.username, request.label, passkey_reg, allow_existing)
        .await?;

    Ok(WebauthnStartRegisterResponse {
        session_id,
        public_key: response.public_key,
    })
}

pub async fn handle_finish_registration(
    context: &HandlerContext,
    token: Option<&Bearer>,
    request: (axum_jrpc::Id, WebauthnFinishRegisterRequest),
) -> Result<(CookieJar, JsonRpcResponse), anyhow::Error> {
    let (answer_id, request) = request;
    let webauthn = webauthn(context)?;
    let webauthn_service = webauthn_service(context)?;
    let session_data = webauthn_service.get_session(&request.session_id).await?;
    if session_data.allow_existing() {
        authorize_enrolled_passkey_addition(context, token)?;
    } else {
        assert_not_enrolled(context)?;
    }
    let passkey = webauthn.finish_passkey_registration(&request.credential, session_data.passkey_reg())?;
    webauthn_service
        .finish_registration(request.session_id, passkey)
        .await
        .map_err(|e| match e {
            WebauthnServiceError::AlreadyEnrolled | WebauthnServiceError::DuplicateCredential => invalid_request(e),
            e => e.into(),
        })?;

    if session_data.allow_existing() {
        return Ok((
            CookieJar::new(),
            JsonRpcResponse::success(answer_id, WebauthnFinishRegisterResponse { token: None }),
        ));
    }

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
        JsonRpcResponse::success(answer_id, WebauthnFinishRegisterResponse { token: Some(token) }),
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

pub async fn handle_list_passkeys(
    context: &HandlerContext,
    token: Option<&Bearer>,
    _request: WebauthnListPasskeysRequest,
) -> Result<WebauthnListPasskeysResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Admin])?;
    webauthn(context)?;
    let entries = webauthn_service(context)?.list_passkeys()?;
    let passkeys = entries
        .into_iter()
        .map(|entry| WebauthnPasskeyInfo {
            id: entry.id,
            label: entry.label,
            counter: Credential::from(entry.passkey).counter,
            created_at: entry.created_at.assume_utc().unix_timestamp(),
            last_used_at: entry.last_used_at.map(|ts| ts.assume_utc().unix_timestamp()),
        })
        .collect();
    Ok(WebauthnListPasskeysResponse { passkeys })
}

pub async fn handle_rename_passkey(
    context: &HandlerContext,
    token: Option<&Bearer>,
    request: WebauthnRenamePasskeyRequest,
) -> Result<WebauthnRenamePasskeyResponse, anyhow::Error> {
    context.authorize_user_only(token, &[Permission::Admin])?;
    webauthn(context)?;
    webauthn_service(context)?
        .rename_passkey(request.id, request.label)
        .map_err(|e| match e {
            WebauthnServiceError::EmptyLabel => invalid_params("label", Some(&e)),
            WebauthnServiceError::PasskeyNotFound { .. } => not_found(e),
            other => other.into(),
        })?;
    Ok(WebauthnRenamePasskeyResponse {})
}

pub async fn handle_remove_passkey(
    context: &HandlerContext,
    token: Option<&Bearer>,
    request: WebauthnRemovePasskeyRequest,
) -> Result<WebauthnRemovePasskeyResponse, anyhow::Error> {
    context.authorize_user_only(token, &[Permission::Admin])?;
    webauthn(context)?;
    webauthn_service(context)?
        .remove_passkey(request.id)
        .map_err(|e| match e {
            WebauthnServiceError::CannotRemoveLastPasskey => invalid_request(e),
            WebauthnServiceError::PasskeyNotFound { .. } => not_found(e),
            other => other.into(),
        })?;
    Ok(WebauthnRemovePasskeyResponse {})
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use axum_extra::headers::Authorization;
    use tari_ootle_address::Network;
    use tari_ootle_wallet_sdk::{IndexerTrust, WalletSdkConfig, cipher_seed::CipherSeedRestore, models::EpochBirthday};
    use tari_ootle_wallet_sdk_services::{
        account_monitor::AccountMonitor,
        indexer_rest_api::IndexerRestApiNetworkInterface,
        notify::Notify,
        transaction_service::TransactionService,
        utxo_scanner::StealthUtxoScannerWorker,
    };
    use tari_ootle_walletd_client::{permissions::Claims, types::AuthCreateApiKeyRequest};
    use tari_shutdown::Shutdown;
    use tari_utilities::SafePassword;
    use webauthn_rs::prelude::{Passkey, RegisterPublicKeyCredential};

    use super::*;
    use crate::{
        WalletSdk,
        config::{WalletDaemonAuth, WalletDaemonConfig},
        handlers::auth::{api_keys, create_authenticator, jwt::AuthError},
    };

    struct HandlerTest {
        context: HandlerContext,
        admin_bearer: Bearer,
        non_admin_bearer: Bearer,
        delegated_admin_bearer: Bearer,
        api_key_admin_bearer: Bearer,
        _temp: tempfile::TempDir,
    }

    fn dummy_passkey(cred_id_b64: &str) -> Passkey {
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

    fn dummy_register_credential() -> RegisterPublicKeyCredential {
        serde_json::from_value(serde_json::json!({
            "id": "AAECAwQFBgcICQoLDA0ODw",
            "rawId": "AAECAwQFBgcICQoLDA0ODw",
            "type": "public-key",
            "response": {
                "attestationObject": "o2NmbXRkbm9uZWdhdHRTdG10oGhhdXRoRGF0YViYSZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2NFAAAAAK3OAAI1vMYKZIsLJfHwVQMAEgABAgMEBQYHCAkKCwwNDg-lAQIDJiABIVggAAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8iWCAAAQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHw",
                "clientDataJSON": "eyJ0eXBlIjoid2ViYXV0aG4uY3JlYXRlIiwiY2hhbGxlbmdlIjoiQUFFQ0F3UUZCZ2NJQ1FvTERBME9EdyIsIm9yaWdpbiI6Imh0dHA6Ly9sb2NhbGhvc3Q6NTEwMCJ9"
            },
            "extensions": {}
        }))
        .unwrap()
    }

    async fn setup() -> HandlerTest {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();
        let mut sdk = WalletSdk::initialize_with_local_key_store(
            store.clone(),
            IndexerRestApiNetworkInterface::new("http://127.0.0.1:1"),
            WalletSdkConfig {
                network: Network::LocalNet,
                override_keyring_password: Some(SafePassword::from_str("test wallet password").unwrap()),
                indexer_trust: IndexerTrust::default(),
            },
            EpochBirthday::far_future(),
        )
        .unwrap();
        sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)
            .unwrap();

        let notify = Notify::new(10);
        let shutdown = Shutdown::new();
        let (transaction_service, transaction_service_handle) =
            TransactionService::new(notify.clone(), sdk.clone(), shutdown.to_signal());
        let (utxo_worker, utxo_scanner_handle) = StealthUtxoScannerWorker::new(sdk.clone(), notify.clone()).spawn();
        let (account_monitor, account_monitor_handle) =
            AccountMonitor::new(notify.clone(), sdk.clone(), utxo_scanner_handle, shutdown.to_signal());
        let mut config = WalletDaemonConfig::default();
        config.network = Network::LocalNet;
        config.authentication = WalletDaemonAuth::WebAuthn;
        let context = HandlerContext::new(
            sdk,
            notify,
            transaction_service_handle,
            account_monitor_handle,
            config.clone(),
            create_authenticator(&config, store).unwrap(),
            SafePassword::from_str("test jwt secret").unwrap(),
            shutdown.to_signal(),
        );

        shutdown.trigger();
        drop(account_monitor);
        drop(transaction_service);
        utxo_worker.abort();
        drop(utxo_worker.await);

        let jwt = context.jwt_api();
        let admin_claims = jwt
            .generate_auth_claims(Permissions::from(vec![Permission::Admin]))
            .unwrap();
        let admin_bearer = Authorization::<Bearer>::bearer(&jwt.grant(&admin_claims).unwrap())
            .unwrap()
            .0;

        let non_admin_claims = jwt
            .generate_auth_claims(Permissions::from(vec![Permission::AccountInfo]))
            .unwrap();
        let non_admin_bearer = Authorization::<Bearer>::bearer(&jwt.grant(&non_admin_claims).unwrap())
            .unwrap()
            .0;

        let delegated_claims = Claims {
            permissions: Permissions::from(vec![Permission::Admin]),
            exp: admin_claims.exp,
            delegated: true,
            delegation_id: Some(42),
        };
        let delegated_admin_bearer = Authorization::<Bearer>::bearer(&jwt.grant(&delegated_claims).unwrap())
            .unwrap()
            .0;

        let created_key = api_keys::handle_create_api_key(&context, Some(&admin_bearer), AuthCreateApiKeyRequest {
            name: "admin-agent".to_string(),
            permissions: vec![Permission::Admin.to_string()],
            confirm_admin: true,
            expires_at: None,
        })
        .await
        .unwrap();
        let api_key_admin_bearer = Authorization::<Bearer>::bearer(created_key.api_key.as_str()).unwrap().0;

        HandlerTest {
            context,
            admin_bearer,
            non_admin_bearer,
            delegated_admin_bearer,
            api_key_admin_bearer,
            _temp: temp,
        }
    }

    async fn enrol_initial_passkey(test: &HandlerTest) -> u32 {
        let start = handle_start_registration(&test.context, None, WebauthnStartRegisterRequest {
            username: "owner".to_string(),
            label: Some("Primary".to_string()),
        })
        .await
        .unwrap();
        test.context
            .webauthn_service()
            .unwrap()
            .finish_registration(start.session_id, dummy_passkey("AAECAwQFBgcICQoLDA0ODw"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn enrolled_wallet_refuses_unauthenticated_non_admin_api_key_and_delegated_tokens() {
        let test = setup().await;
        let first_id = enrol_initial_passkey(&test).await;

        // 1. Unauthenticated reg_start on an enrolled wallet is refused
        let err = handle_start_registration(&test.context, None, WebauthnStartRegisterRequest {
            username: "attacker".to_string(),
            label: None,
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("already enrolled"), "{err}");

        // 2. Non-Admin user token is refused on reg_start, list, rename, and remove
        let err = handle_start_registration(
            &test.context,
            Some(&test.non_admin_bearer),
            WebauthnStartRegisterRequest {
                username: "owner".to_string(),
                label: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<AuthError>(),
            Some(AuthError::InsufficientPermissions { .. })
        ));

        let err = handle_list_passkeys(
            &test.context,
            Some(&test.non_admin_bearer),
            WebauthnListPasskeysRequest {},
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<AuthError>(),
            Some(AuthError::InsufficientPermissions { .. })
        ));

        // 3. Admin API key and delegated Admin token are refused with UserAuthOnly on reg_start, reg_finish, rename,
        //    and remove
        for forbidden_bearer in [&test.api_key_admin_bearer, &test.delegated_admin_bearer] {
            let err = handle_start_registration(&test.context, Some(forbidden_bearer), WebauthnStartRegisterRequest {
                username: "owner".to_string(),
                label: Some("Backdoor".to_string()),
            })
            .await
            .unwrap_err();
            assert!(
                matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)),
                "reg_start should reject non-interactive bearer with UserAuthOnly: {err}"
            );

            let err = handle_rename_passkey(&test.context, Some(forbidden_bearer), WebauthnRenamePasskeyRequest {
                id: first_id,
                label: "Renamed".to_string(),
            })
            .await
            .unwrap_err();
            assert!(
                matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)),
                "rename_passkey should reject non-interactive bearer with UserAuthOnly: {err}"
            );

            let err = handle_remove_passkey(&test.context, Some(forbidden_bearer), WebauthnRemovePasskeyRequest {
                id: first_id,
            })
            .await
            .unwrap_err();
            assert!(
                matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)),
                "remove_passkey should reject non-interactive bearer with UserAuthOnly: {err}"
            );
        }

        // Start a valid authenticated session with admin_bearer, then verify reg_finish rejects None, non-Admin, API
        // key, and delegated token
        let start = handle_start_registration(&test.context, Some(&test.admin_bearer), WebauthnStartRegisterRequest {
            username: "owner".to_string(),
            label: Some("Second".to_string()),
        })
        .await
        .unwrap();

        let err = handle_finish_registration(
            &test.context,
            None,
            (axum_jrpc::Id::Num(1), WebauthnFinishRegisterRequest {
                session_id: start.session_id.clone(),
                credential: dummy_register_credential(),
            }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("already enrolled"), "{err}");

        for forbidden_bearer in [&test.api_key_admin_bearer, &test.delegated_admin_bearer] {
            let err = handle_finish_registration(
                &test.context,
                Some(forbidden_bearer),
                (axum_jrpc::Id::Num(1), WebauthnFinishRegisterRequest {
                    session_id: start.session_id.clone(),
                    credential: dummy_register_credential(),
                }),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)),
                "reg_finish should reject non-interactive bearer with UserAuthOnly: {err}"
            );
        }

        // Meanwhile list_passkeys works with an Admin API key as expected
        let listed = handle_list_passkeys(
            &test.context,
            Some(&test.api_key_admin_bearer),
            WebauthnListPasskeysRequest {},
        )
        .await
        .unwrap();
        assert_eq!(listed.passkeys.len(), 1);
    }

    #[tokio::test]
    async fn bootstrap_session_cannot_finish_after_another_enrolment_lands() {
        let test = setup().await;

        // Start an unauthenticated bootstrap session before any passkey is enrolled (`allow_existing == false`)
        let stale_bootstrap = handle_start_registration(&test.context, None, WebauthnStartRegisterRequest {
            username: "attacker".to_string(),
            label: None,
        })
        .await
        .unwrap();

        // Legitimate enrollment completes first
        enrol_initial_passkey(&test).await;

        // Attempting to finish the stale bootstrap session must be refused even if the caller presents an Admin bearer
        for token in [None, Some(&test.admin_bearer)] {
            let err = handle_finish_registration(
                &test.context,
                token,
                (axum_jrpc::Id::Num(1), WebauthnFinishRegisterRequest {
                    session_id: stale_bootstrap.session_id.clone(),
                    credential: dummy_register_credential(),
                }),
            )
            .await
            .unwrap_err();
            assert!(
                err.to_string().contains("already enrolled"),
                "bootstrap session must be refused once wallet is enrolled: {err}"
            );
        }
    }
}
