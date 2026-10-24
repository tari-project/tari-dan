//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Create / approve / reject for signing requests.
//!
//! A requester asks this wallet to co-sign a transaction that another party
//! seals, for example a council member's signature on a governance
//! transaction. A person approves the request in an interactive wallet
//! session, walletd signs, and the requester fetches the
//! [`TransactionSignature`](tari_ootle_transaction::TransactionSignature) to
//! assemble and submit itself.
//!
//! Approval and rejection require an interactive session: an API key is
//! refused even when it holds `signing_requests:approve`. A credential handed
//! to a tool can request signatures; only a person releases one.
//!
//! That guarantee rests on the daemon's authentication: with
//! `authentication = None`, any caller can open an admin session.

use std::time::Duration;

use axum_extra::headers::authorization::Bearer;
use log::*;
use ootle_byte_type::ToByteType;
use tari_ootle_common_types::optional::IsNotFoundError;
use tari_ootle_transaction::TransactionSignature;
use tari_ootle_wallet_sdk::{
    models::{
        KeyBranch,
        KeyId,
        NewSigningRequest,
        SigningRequestEffectiveStatus,
        SigningRequestId,
        SigningRequestModel,
        SigningRequester,
    },
    storage::{ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_walletd_client::{
    permissions::{Permission, TxRequestAction},
    types::{
        SigningRequestCreateRequest,
        SigningRequestCreateResponse,
        SigningRequestDecisionRequest,
        SigningRequestDecisionResponse,
        SigningRequestGetRequest,
        SigningRequestGetResponse,
        SigningRequestInfo,
        SigningRequestListRequest,
        SigningRequestListResponse,
    },
};

use super::context::{AuthIdentity, HandlerContext};
use crate::handlers::helpers::{invalid_params, invalid_request};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::handlers::signing_requests";

/// Upper bound on a caller-supplied approval window. Co-signers of a council
/// transaction may take days to respond, and a transaction is valid for at
/// most ~30 days of epochs, so a longer window could only outlive the
/// transaction it signs.
const MAX_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Upper bound on the requester's memo, in characters.
const MAX_MEMO_CHARS: usize = 280;

/// Upper bound on requests awaiting a decision. Expired requests are deleted
/// when a new one is created, so this bounds the table along with the decided
/// requests, which only a person creates.
const MAX_PENDING: u64 = 256;

pub async fn handle_create(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: SigningRequestCreateRequest,
) -> Result<SigningRequestCreateResponse, anyhow::Error> {
    let auth = context.authorize_with_identity(token, &[Permission::SigningRequests(TxRequestAction::Create)])?;
    let sdk = context.wallet_sdk();

    let ttl = validate_create_request(context, &req)?;
    let transaction = req.transaction;

    let signer_public_key = match sdk.key_manager_api().get_public_key(req.key_id) {
        Ok(key) => key.public_key.to_byte_type(),
        Err(e) if e.is_not_found_error() => {
            return Err(invalid_params(
                "key_id",
                Some(format!("this wallet has no key {}", req.key_id)),
            ));
        },
        Err(e) => return Err(e.into()),
    };

    let current_epoch = context.current_epoch().await?;
    if transaction.max_epoch() < current_epoch {
        return Err(invalid_params(
            "transaction.max_epoch",
            Some(format!(
                "the transaction expired at epoch {}; the current epoch is {current_epoch}",
                transaction.max_epoch()
            )),
        ));
    }

    let message_hash = TransactionSignature::create_message(&req.seal_public_key, &transaction);
    let requester = requester_of(auth);

    let model = sdk.store().with_write_tx(|tx| {
        tx.signing_requests_delete_expired()?;
        if tx.signing_requests_count_pending()? >= MAX_PENDING {
            return Err(invalid_request(format!(
                "{MAX_PENDING} signing requests are already awaiting a decision; approve or reject some first"
            )));
        }
        let model = tx.signing_request_insert(NewSigningRequest {
            unsigned_transaction: &transaction,
            seal_public_key: &req.seal_public_key,
            key_id: req.key_id,
            signer_public_key: &signer_public_key,
            message_hash: &message_hash,
            memo: &req.memo,
            requester: &requester,
            ttl,
        })?;
        Ok::<_, anyhow::Error>(model)
    })?;

    info!(
        target: LOG_TARGET,
        "Signing request {} created by {} for key {} (message {}, expires {})",
        model.id,
        model.requester,
        model.key_id,
        fingerprint(&model.message_hash),
        model.expires_at,
    );

    Ok(SigningRequestCreateResponse {
        request_id: model.id,
        message_hash: model.message_hash,
        expires_at: model.expires_at.assume_utc().unix_timestamp(),
    })
}

/// Checks a create request needs no I/O for, returning its approval window.
fn validate_create_request(
    context: &HandlerContext,
    req: &SigningRequestCreateRequest,
) -> Result<Duration, anyhow::Error> {
    let transaction = &req.transaction;
    transaction
        .validate_blob_references()
        .map_err(|e| invalid_params("transaction.blobs", Some(e.to_string())))?;

    let network = context.config().network;
    if transaction.network() != network.as_byte() {
        return Err(invalid_params(
            "transaction.network",
            Some(format!(
                "the transaction is for network byte {:#04x}, but this wallet is on {network} ({:#04x})",
                transaction.network(),
                network.as_byte()
            )),
        ));
    }

    if transaction.is_dry_run() {
        return Err(invalid_params(
            "transaction.dry_run",
            Some("a dry-run transaction cannot be committed, so it is never co-signed"),
        ));
    }

    if req.memo.chars().count() > MAX_MEMO_CHARS {
        return Err(invalid_params(
            "memo",
            Some(format!("must be at most {MAX_MEMO_CHARS} characters")),
        ));
    }

    let ttl = req
        .ttl_secs
        .map(Duration::from_secs)
        .unwrap_or(context.config().signing_request_ttl);
    if ttl > MAX_TTL {
        return Err(invalid_params(
            "ttl_secs",
            Some(format!("must be at most {} seconds", MAX_TTL.as_secs())),
        ));
    }

    if !is_signing_key(context, &req.key_id)? {
        return Err(invalid_params(
            "key_id",
            Some(format!(
                "{} is not a signing key: only account, transaction and imported keys sign transactions",
                req.key_id
            )),
        ));
    }

    Ok(ttl)
}

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: SigningRequestGetRequest,
) -> Result<SigningRequestGetResponse, anyhow::Error> {
    context.authorize(token, &[Permission::SigningRequests(TxRequestAction::Read)])?;

    let model = context
        .wallet_sdk()
        .store()
        .with_read_tx(|tx| tx.signing_request_get(req.request_id))?;

    Ok(SigningRequestGetResponse {
        request: to_info(model),
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: SigningRequestListRequest,
) -> Result<SigningRequestListResponse, anyhow::Error> {
    context.authorize(token, &[Permission::SigningRequests(TxRequestAction::Read)])?;

    let models = context
        .wallet_sdk()
        .store()
        .with_read_tx(|tx| tx.signing_requests_list())?;

    let requests = models
        .into_iter()
        .map(to_info)
        // Expiry is derived, so it cannot be filtered in SQL.
        .filter(|info| req.status.is_none_or(|s| s == info.status))
        .collect();

    Ok(SigningRequestListResponse { requests })
}

pub async fn handle_approve(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: SigningRequestDecisionRequest,
) -> Result<SigningRequestDecisionResponse, anyhow::Error> {
    context.authorize_user_only(token, &[Permission::SigningRequests(TxRequestAction::Approve)])?;
    let sdk = context.wallet_sdk();

    let request = get_pending(context, req.request_id)?;

    // The approver was shown `message_hash`; sign only if the stored request
    // still produces exactly that message.
    let message = TransactionSignature::create_message(&request.seal_public_key, &request.unsigned_transaction);
    if message != request.message_hash {
        return Err(invalid_request(format!(
            "Signing request {} no longer produces the message it was created with; reject it and ask for a new one",
            req.request_id
        )));
    }

    let signature = sdk
        .signer_api()
        .with_context(&request.seal_public_key)
        .generate_signature(request.key_id, &request.unsigned_transaction)?;
    if *signature.public_key() != request.signer_public_key {
        return Err(invalid_request(format!(
            "Key {} now resolves to a different public key than signing request {} was created for",
            request.key_id, req.request_id
        )));
    }

    let model = sdk
        .store()
        .with_write_tx(|tx| tx.signing_request_mark_signed(req.request_id, &signature))?;

    info!(
        target: LOG_TARGET,
        "Signing request {} approved: signed message {} with key {}",
        req.request_id,
        fingerprint(&model.message_hash),
        model.key_id,
    );

    Ok(SigningRequestDecisionResponse {
        request_id: model.id,
        status: model.effective_status_now(),
        signature: model.signature,
    })
}

pub async fn handle_reject(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: SigningRequestDecisionRequest,
) -> Result<SigningRequestDecisionResponse, anyhow::Error> {
    context.authorize_user_only(token, &[Permission::SigningRequests(TxRequestAction::Approve)])?;

    get_pending(context, req.request_id)?;
    let model = context
        .wallet_sdk()
        .store()
        .with_write_tx(|tx| tx.signing_request_reject(req.request_id))?;

    info!(target: LOG_TARGET, "Signing request {} rejected", req.request_id);

    Ok(SigningRequestDecisionResponse {
        request_id: model.id,
        status: model.effective_status_now(),
        signature: None,
    })
}

/// Loads a request that is still awaiting a decision, or explains why it is
/// not. The storage write re-checks the same conditions atomically.
fn get_pending(context: &HandlerContext, request_id: SigningRequestId) -> Result<SigningRequestModel, anyhow::Error> {
    let model = context
        .wallet_sdk()
        .store()
        .with_read_tx(|tx| tx.signing_request_get(request_id))?;

    match model.effective_status_now() {
        SigningRequestEffectiveStatus::Pending => Ok(model),
        SigningRequestEffectiveStatus::Expired => Err(invalid_request(format!(
            "Signing request {request_id} expired at {} without a decision",
            model.expires_at
        ))),
        status => Err(invalid_request(format!(
            "Signing request {request_id} was already decided: {status:?}"
        ))),
    }
}

fn requester_of(auth: AuthIdentity) -> SigningRequester {
    auth.signing_requester()
}

/// Keys a signing request may name. A nonce key signs only as the throw-away
/// authorizer of a stealth statement's revealed funds, in a transaction walletd
/// seals itself, so a co-signature for another party never needs one.
fn is_signing_key(context: &HandlerContext, key_id: &KeyId) -> Result<bool, anyhow::Error> {
    let is_signing_key = context.wallet_sdk().key_manager_api().is_signing_key(*key_id)?;
    Ok(is_signing_key && key_id.derived_branch() != Some(KeyBranch::Nonce))
}

/// The first 8 bytes of the authorization message, as shown to the approver.
fn fingerprint(message_hash: &[u8; 64]) -> String {
    hex::encode(&message_hash[..8])
}

fn to_info(model: SigningRequestModel) -> SigningRequestInfo {
    SigningRequestInfo {
        request_id: model.id,
        status: model.effective_status_now(),
        transaction: model.unsigned_transaction,
        seal_public_key: model.seal_public_key,
        key_id: model.key_id,
        signer_public_key: model.signer_public_key,
        message_hash: model.message_hash,
        memo: model.memo,
        requester: model.requester,
        signature: model.signature,
        expires_at: model.expires_at.assume_utc().unix_timestamp(),
        decided_at: model.decided_at.map(|t| t.assume_utc().unix_timestamp()),
        created_at: model.created_at.assume_utc().unix_timestamp(),
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_address::Network;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_transaction::{Transaction, UnsignedTransaction};
    use tari_ootle_wallet_sdk::models::{KeyBranch, KeyId, SigningRequestStatus};
    use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

    use super::*;
    use crate::handlers::test_support::TestDaemon;

    struct SigningTest {
        daemon: TestDaemon,
        context: HandlerContext,
        /// An interactive session holding `admin`.
        session: Bearer,
        /// An API key holding every signing-request permission.
        api_key: Bearer,
    }

    fn signer_key() -> KeyId {
        KeyId::derived(KeyBranch::Account, 0)
    }

    fn seal_public_key() -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(&[0x5e; 32]).unwrap()
    }

    fn transaction() -> UnsignedTransaction {
        UnsignedTransaction::new(Network::LocalNet.as_byte(), Epoch(100))
    }

    async fn setup() -> SigningTest {
        let daemon = TestDaemon::start().await;
        let api_key = daemon.api_key("governance-signer", "signing_requests:create,signing_requests:approve");
        SigningTest {
            context: daemon.context.clone(),
            session: daemon.session.clone(),
            api_key,
            daemon,
        }
    }

    /// Stores a pending request the way `handle_create` would, skipping its
    /// network-dependent epoch check.
    fn insert_request(test: &SigningTest) -> SigningRequestId {
        let sdk = test.context.wallet_sdk();
        let transaction = transaction();
        let signer_public_key = sdk
            .key_manager_api()
            .get_public_key(signer_key())
            .unwrap()
            .public_key
            .to_byte_type();
        let message_hash = TransactionSignature::create_message(&seal_public_key(), &transaction);
        sdk.store()
            .with_write_tx(|tx| {
                tx.signing_request_insert(NewSigningRequest {
                    unsigned_transaction: &transaction,
                    seal_public_key: &seal_public_key(),
                    key_id: signer_key(),
                    signer_public_key: &signer_public_key,
                    message_hash: &message_hash,
                    memo: "",
                    requester: &SigningRequester::ApiKey {
                        name: "governance-signer".to_string(),
                    },
                    ttl: Duration::from_secs(600),
                })
            })
            .unwrap()
            .id
    }

    fn create_request(transaction: UnsignedTransaction, key_id: KeyId) -> SigningRequestCreateRequest {
        SigningRequestCreateRequest {
            transaction,
            seal_public_key: seal_public_key(),
            key_id,
            memo: String::new(),
            ttl_secs: None,
        }
    }

    fn status_of(test: &SigningTest, request_id: SigningRequestId) -> SigningRequestStatus {
        test.context
            .wallet_sdk()
            .store()
            .with_read_tx(|tx| tx.signing_request_get(request_id))
            .unwrap()
            .status
    }

    #[tokio::test]
    async fn an_approved_request_yields_a_signature_over_the_stored_transaction() {
        let test = setup().await;
        let request_id = insert_request(&test);

        let response = handle_approve(&test.context, Some(&test.session), SigningRequestDecisionRequest {
            request_id,
        })
        .await
        .unwrap();
        assert_eq!(response.status, SigningRequestEffectiveStatus::Signed);

        let info = handle_get(&test.context, Some(&test.api_key), SigningRequestGetRequest {
            request_id,
        })
        .await
        .unwrap()
        .request;
        let signature = info.signature.expect("a signed request carries its signature");
        assert_eq!(Some(&signature), response.signature.as_ref());
        assert_eq!(*signature.public_key(), info.signer_public_key);
        assert_eq!(
            info.message_hash,
            TransactionSignature::create_message(&seal_public_key(), &transaction())
        );
        assert!(
            signature.verify_message(info.message_hash),
            "the signature must verify against the message the approver was shown"
        );
    }

    #[tokio::test]
    async fn an_api_key_cannot_approve_or_reject() {
        let test = setup().await;
        let request_id = insert_request(&test);

        let err = handle_approve(&test.context, Some(&test.api_key), SigningRequestDecisionRequest {
            request_id,
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("user"), "unexpected error: {err}");

        handle_reject(&test.context, Some(&test.api_key), SigningRequestDecisionRequest {
            request_id,
        })
        .await
        .unwrap_err();

        assert_eq!(status_of(&test, request_id), SigningRequestStatus::Pending);
    }

    #[tokio::test]
    async fn a_delegated_token_cannot_approve() {
        // `webrtc.start` mints a delegated token carrying its caller's grants,
        // which an API key holding `webrtc` can call.
        let test = setup().await;
        let request_id = insert_request(&test);

        let delegated = test.daemon.delegated_session("admin");

        let err = handle_approve(&test.context, Some(&delegated), SigningRequestDecisionRequest {
            request_id,
        })
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("interactive user session"),
            "unexpected error: {err}"
        );
        assert_eq!(status_of(&test, request_id), SigningRequestStatus::Pending);
    }

    #[tokio::test]
    async fn a_connected_app_is_recorded_as_one() {
        let test = setup().await;
        let required = [Permission::SigningRequests(TxRequestAction::Create)];
        let requester =
            |bearer: &Bearer| requester_of(test.context.authorize_with_identity(Some(bearer), &required).unwrap());

        let delegated = test.daemon.delegated_session("admin");

        assert_eq!(requester(&delegated), SigningRequester::ConnectedApp);
        assert_eq!(requester(&test.session), SigningRequester::WalletSession);
        assert_eq!(requester(&test.daemon.cookie), SigningRequester::LocalClient);
        assert_eq!(requester(&test.api_key), SigningRequester::ApiKey {
            name: "governance-signer".to_string()
        });
    }

    #[tokio::test]
    async fn a_decided_request_cannot_be_decided_again() {
        let test = setup().await;
        let signed = insert_request(&test);
        let rejected = insert_request(&test);
        let decide = |request_id| SigningRequestDecisionRequest { request_id };

        handle_approve(&test.context, Some(&test.session), decide(signed))
            .await
            .unwrap();
        handle_approve(&test.context, Some(&test.session), decide(signed))
            .await
            .unwrap_err();
        handle_reject(&test.context, Some(&test.session), decide(signed))
            .await
            .unwrap_err();

        handle_reject(&test.context, Some(&test.session), decide(rejected))
            .await
            .unwrap();
        handle_approve(&test.context, Some(&test.session), decide(rejected))
            .await
            .unwrap_err();

        assert_eq!(status_of(&test, signed), SigningRequestStatus::Signed);
        assert_eq!(status_of(&test, rejected), SigningRequestStatus::Rejected);
    }

    #[tokio::test]
    async fn create_refuses_a_transaction_this_wallet_must_not_sign() {
        let test = setup().await;
        let create = |req| handle_create(&test.context, Some(&test.api_key), req);

        let mut other_network = transaction();
        other_network.set_network(Network::Esmeralda.as_byte());
        let err = create(create_request(other_network, signer_key())).await.unwrap_err();
        assert!(err.to_string().contains("transaction.network"), "{err}");

        let dry_run = Transaction::builder(Network::LocalNet.as_byte(), Epoch(100))
            .with_dry_run(true)
            .build_unsigned();
        let err = create(create_request(dry_run, signer_key())).await.unwrap_err();
        assert!(err.to_string().contains("transaction.dry_run"), "{err}");

        let unknown_key = KeyId::Imported { local_key_id: 999 };
        let err = create(create_request(transaction(), unknown_key)).await.unwrap_err();
        assert!(err.to_string().contains("key_id"), "{err}");

        for key_branch in [
            KeyBranch::ConfidentialMask,
            KeyBranch::StealthMask,
            KeyBranch::ElgamalEncryptionViewKey,
            KeyBranch::Nonce,
            KeyBranch::ViewOnlyKey,
        ] {
            let err = create(create_request(transaction(), KeyId::derived(key_branch, 0)))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("not a signing key"), "{key_branch:?}: {err}");
        }

        let mut long_memo = create_request(transaction(), signer_key());
        long_memo.memo = "x".repeat(MAX_MEMO_CHARS + 1);
        let err = create(long_memo).await.unwrap_err();
        assert!(err.to_string().contains("memo"), "{err}");

        let none_created = test
            .context
            .wallet_sdk()
            .store()
            .with_read_tx(|tx| tx.signing_requests_list())
            .unwrap();
        assert!(none_created.is_empty());
    }
}
