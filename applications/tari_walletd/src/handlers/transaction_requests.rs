//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Create / approve / submit for transaction requests (issue #2343).
//!
//! A limited-permission tool creates a request; an interactive user session
//! holding `transaction_requests:approve` authorises it; submit seals it. The
//! three are separately permissioned, and approval refuses API keys outright,
//! so a tool cannot approve the requests it creates.
//!
//! The transaction is **frozen at creation**: stored verbatim (the caller
//! supplies complete inputs and any out-of-band signatures). The approver views
//! exactly what will be sealed, and submit seals it unchanged.

use std::{collections::HashSet, time::Duration};

use axum_extra::headers::authorization::Bearer;
use log::*;
use ootle_byte_type::ToByteType;
use tari_ootle_common_types::optional::Optional;
use tari_ootle_transaction::{MainSigner, UnsignedTransaction};
use tari_ootle_wallet_sdk::{
    models::{
        EffectiveStatus,
        KeyId,
        LockOwner,
        OutputStatus,
        TransactionRequestCreatedEvent,
        TransactionRequestId,
        TransactionRequestModel,
        TransactionRequestStatus,
        WalletEvent,
        WalletLockId,
    },
    storage::{CommittableStore, ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_walletd_client::{
    permissions::{Permission, TxRequestAction},
    types::{
        TransactionRequestCreateRequest,
        TransactionRequestCreateResponse,
        TransactionRequestDecisionRequest,
        TransactionRequestDecisionResponse,
        TransactionRequestGetRequest,
        TransactionRequestGetResponse,
        TransactionRequestInfo,
        TransactionRequestListRequest,
        TransactionRequestListResponse,
        TransactionRequestMainSigner,
        TransactionRequestSubmitRequest,
        TransactionRequestSubmitResponse,
        TransactionRequestValueSummary,
        TransactionSubmitRequest,
    },
};

use super::context::HandlerContext;
use crate::handlers::{
    helpers::{ensure_transaction_signers, invalid_params, invalid_request},
    transaction::submit_inner_for_request,
};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::handlers::transaction_requests";

/// Extra margin on the locks beyond the approval window.
///
/// The lock must outlive the request, never the reverse: a lock released under
/// an Approved request would let submit sign a spend of UTXOs that are
/// spendable again, whereas a lock outliving a dead request merely idles funds
/// until the stale sweep takes it.
const LOCK_GRACE: Duration = Duration::from_secs(5 * 60);

/// Upper bound on a caller-supplied approval window. A request idling input
/// locks for longer than this is not an approval flow, and the bound keeps the
/// `ttl + LOCK_GRACE` and expiry-timestamp arithmetic trivially in range.
const MAX_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub async fn handle_create(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionRequestCreateRequest,
) -> Result<TransactionRequestCreateResponse, anyhow::Error> {
    let auth = context.authorize_with_identity(token, &[Permission::TransactionRequests(TxRequestAction::Create)])?;
    let sdk = context.wallet_sdk();

    req.transaction
        .validate_blob_references()
        .map_err(|e| invalid_params("transaction.blobs", Some(e.to_string())))?;
    ensure_transaction_signers(sdk, &req.seal_signer, &req.other_signers)?;
    if let TransactionRequestMainSigner::External { public_key } =
        resolve_main_signer(context, &req.transaction, &req.other_signers)? &&
        !req.signatures.iter().any(|sig| *sig.public_key() == public_key)
    {
        return Err(invalid_params(
            "transaction.main_signer",
            Some(format!(
                "names {public_key} as the main signer, but neither other_signers nor signatures sign with \
                 {public_key}"
            )),
        ));
    }

    // The transaction is stored verbatim: the request path neither detects
    // inputs nor touches any field, so a caller who has already collected
    // signatures over these exact bytes is not invalidated. Callers that need
    // inputs resolved run `transactions.detect_inputs` first.
    let unsigned = req.transaction;

    let ttl = req
        .ttl_secs
        .map(Duration::from_secs)
        .unwrap_or(context.config().transaction_request_ttl);
    if ttl > MAX_TTL {
        return Err(invalid_params(
            "ttl_secs",
            Some(format!("must be at most {} seconds", MAX_TTL.as_secs())),
        ));
    }
    let model = {
        let mut tx = sdk.store().create_write_tx()?;

        ensure_free_locks_of(&mut *tx, auth.lock_owner(), &req.lock_ids, "lock_ids")?;

        // Hold the inputs across the approval window. The transfer-selection
        // handlers lock for five minutes, which is a selection deadline, not a
        // deadline a person can meet.
        for lock_id in &req.lock_ids {
            tx.locks_extend_timeout(*lock_id, ttl + LOCK_GRACE)?;
        }

        let requested_by = auth.requested_by();
        let model = tx.transaction_request_insert(
            &unsigned,
            req.seal_signer,
            &req.other_signers,
            &req.signatures,
            &req.lock_ids,
            requested_by.as_deref(),
            ttl,
        )?;
        tx.commit()?;
        model
    };

    info!(
        target: LOG_TARGET,
        "Transaction request {} created by {} ({} lock(s), expires {})",
        model.id,
        auth.signing_requester(),
        req.lock_ids.len(),
        model.expires_at,
    );

    context
        .notifier()
        .notify(WalletEvent::TransactionRequestCreated(TransactionRequestCreatedEvent {
            request_id: model.id,
            requested_by: auth.requested_by(),
        }));

    Ok(TransactionRequestCreateResponse {
        request_id: model.id,
        expires_at: model.expires_at.assume_utc().unix_timestamp(),
    })
}

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionRequestGetRequest,
) -> Result<TransactionRequestGetResponse, anyhow::Error> {
    context.authorize(token, &[Permission::TransactionRequests(TxRequestAction::Read)])?;

    let model = context
        .wallet_sdk()
        .store()
        .with_read_tx(|tx| tx.transaction_request_get(req.request_id))?;

    Ok(TransactionRequestGetResponse {
        request: to_info(context, model)?,
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionRequestListRequest,
) -> Result<TransactionRequestListResponse, anyhow::Error> {
    context.authorize(token, &[Permission::TransactionRequests(TxRequestAction::Read)])?;

    let models = context
        .wallet_sdk()
        .store()
        .with_read_tx(|tx| tx.transaction_requests_list())?;

    let requests = models
        .into_iter()
        .map(|m| to_info(context, m))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        // Expiry is derived, so it cannot be filtered in SQL.
        .filter(|info| req.status.is_none_or(|s| s == info.status))
        .collect();

    Ok(TransactionRequestListResponse { requests })
}

pub async fn handle_approve(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionRequestDecisionRequest,
) -> Result<TransactionRequestDecisionResponse, anyhow::Error> {
    // Approving authorises the value transfer the request describes, so it is
    // a human decision: it takes an interactive user session, and no API key
    // can approve whatever it was granted.
    context.authorize_user_only(token, &[Permission::TransactionRequests(TxRequestAction::Approve)])?;

    let model = transition(
        context,
        req.request_id,
        &[TransactionRequestStatus::Pending],
        TransactionRequestStatus::Approved,
    )?;

    info!(target: LOG_TARGET, "Transaction request {} approved", req.request_id);

    Ok(TransactionRequestDecisionResponse {
        request_id: req.request_id,
        status: model.effective_status_now(),
    })
}

pub async fn handle_reject(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionRequestDecisionRequest,
) -> Result<TransactionRequestDecisionResponse, anyhow::Error> {
    context.authorize(token, &[Permission::TransactionRequests(TxRequestAction::Approve)])?;

    // An approved-but-unsubmitted request is still cancellable: approval is a
    // decision, not a commitment, and without this the only way out is letting
    // the request idle its input locks until expiry. Only an in-flight submit
    // (Submitting) is past the point of refusal.
    let model = transition(
        context,
        req.request_id,
        &[TransactionRequestStatus::Pending, TransactionRequestStatus::Approved],
        TransactionRequestStatus::Rejected,
    )?;

    // Release the inputs immediately rather than idling them until the sweep:
    // a refused request is never going to spend them.
    for lock_id in &model.lock_ids {
        if let Err(e) = context.wallet_sdk().locks_api().release_lock(*lock_id) {
            warn!(
                target: LOG_TARGET,
                "Failed to release lock {lock_id} for rejected request {}: {e}", req.request_id,
            );
        }
    }

    Ok(TransactionRequestDecisionResponse {
        request_id: req.request_id,
        status: model.effective_status_now(),
    })
}

pub async fn handle_submit(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionRequestSubmitRequest,
) -> Result<TransactionRequestSubmitResponse, anyhow::Error> {
    // Submitting an approved request is not the same authority as
    // `transactions:create` (unrestricted submit): the transaction is frozen at
    // creation, so this confers no ability to alter what gets signed.
    context.authorize(token, &[Permission::TransactionRequests(TxRequestAction::Create)])?;
    let sdk = context.wallet_sdk();

    // Claim the request before any sealing work: sealing uses a random nonce,
    // so two concurrent submitters that both passed a status *check* would
    // produce two distinct transactions spending the same inputs, and the
    // locks would end up linked to whichever sealed last. The conditional
    // Approved -> Submitting UPDATE lets exactly one caller proceed; the rest
    // fail here with the request untouched.
    let model = transition(
        context,
        req.request_id,
        &[TransactionRequestStatus::Approved],
        TransactionRequestStatus::Submitting,
    )?;

    let submit = TransactionSubmitRequest {
        transaction: model.unsigned_transaction,
        seal_signer: model.seal_signer,
        other_signers: model.other_signers,
        signatures: model.signatures,
        // Everything was resolved at creation. Detecting again here would
        // change the transaction the approver saw.
        detect_inputs: false,
        detect_inputs_use_unversioned: true,
        lock_ids: model.lock_ids,
    };

    let response = match submit_inner_for_request(context, submit).await {
        Ok(response) => response,
        Err(e) => {
            // Release the claim so the approval survives a failed submit and
            // another holder can retry. Best-effort: if this also fails the
            // claim expires with the request and the stale sweep reclaims the
            // (still unlinked) locks.
            if let Err(release_err) = sdk.store().with_write_tx(|tx| {
                tx.transaction_request_transition(
                    req.request_id,
                    &[TransactionRequestStatus::Submitting],
                    TransactionRequestStatus::Approved,
                )
            }) {
                warn!(
                    target: LOG_TARGET,
                    "Failed to release submit claim on transaction request {}: {release_err}", req.request_id,
                );
            }
            return Err(e);
        },
    };

    // Record what it became, so the approval is linked to the transaction it
    // authorised rather than dead-ending at "Submitted".
    sdk.store()
        .with_write_tx(|tx| tx.transaction_request_mark_submitted(req.request_id, response.transaction_id))?;

    info!(
        target: LOG_TARGET,
        "Transaction request {} submitted as {}", req.request_id, response.transaction_id,
    );

    Ok(TransactionRequestSubmitResponse {
        transaction_id: response.transaction_id,
    })
}

/// Refuses `lock_ids` unless `caller` created each lock and neither a
/// transaction nor a live request holds it. Rejecting a request releases its
/// locks, so a lock held by two requests would be released under the other.
///
/// Every refusal is the same error on `field`, so the call does not reveal
/// which lock ids exist.
pub(crate) fn ensure_free_locks_of<R: WalletStoreReader>(
    reader: &mut R,
    caller: LockOwner,
    lock_ids: &[WalletLockId],
    field: &'static str,
) -> Result<(), anyhow::Error> {
    if lock_ids.is_empty() {
        return Ok(());
    }
    let mut held = reader
        .transaction_requests_list()?
        .into_iter()
        .filter(|r| {
            matches!(
                r.effective_status_now(),
                EffectiveStatus::Pending | EffectiveStatus::Approved | EffectiveStatus::Submitting
            )
        })
        .flat_map(|r| r.lock_ids)
        .collect::<HashSet<_>>();

    for lock_id in lock_ids {
        let lock = reader.locks_get(*lock_id).optional()?;
        let free = lock.is_some_and(|l| l.owner == caller && l.transaction_id.is_none());
        if !free || !held.insert(*lock_id) {
            return Err(invalid_params(
                field,
                Some(format!(
                    "lock {lock_id} does not exist, was created by another caller, or is already in use"
                )),
            ));
        }
    }
    Ok(())
}

/// Guarded transition to `to`, refusing an already-expired request.
fn transition(
    context: &HandlerContext,
    request_id: TransactionRequestId,
    from: &[TransactionRequestStatus],
    to: TransactionRequestStatus,
) -> Result<TransactionRequestModel, anyhow::Error> {
    let sdk = context.wallet_sdk();

    let current = sdk.store().with_read_tx(|tx| tx.transaction_request_get(request_id))?;

    // A decision on a request that has already timed out is not a decision.
    if matches!(current.effective_status_now(), EffectiveStatus::Expired) {
        return Err(invalid_request(format!(
            "Transaction request {request_id} expired at {}",
            current.expires_at
        )));
    }

    let model = sdk
        .store()
        .with_write_tx(|tx| tx.transaction_request_transition(request_id, from, to))?;

    Ok(model)
}

/// The wallet-computed view of a request, including what it moves.
fn to_info(context: &HandlerContext, model: TransactionRequestModel) -> Result<TransactionRequestInfo, anyhow::Error> {
    let status = model.effective_status_now();

    let main_signer = resolve_main_signer(context, &model.unsigned_transaction, &model.other_signers)?;
    let external_signers = model.signatures.iter().map(|sig| *sig.public_key()).collect();

    Ok(TransactionRequestInfo {
        request_id: model.id,
        transaction: model.unsigned_transaction,
        seal_signer: model.seal_signer,
        other_signers: model.other_signers,
        requested_by: model.requested_by,
        status,
        transaction_id: model.transaction_id,
        value_summary: value_summary(context, &model.lock_ids)?,
        main_signer,
        external_signers,
        expires_at: model.expires_at.assume_utc().unix_timestamp(),
        approved_at: model.approved_at.map(|t| t.assume_utc().unix_timestamp()),
        created_at: model.created_at.assume_utc().unix_timestamp(),
    })
}

/// The transaction's declared main signer, placed among the keys of this wallet.
fn resolve_main_signer(
    context: &HandlerContext,
    transaction: &UnsignedTransaction,
    other_signers: &[KeyId],
) -> Result<TransactionRequestMainSigner, anyhow::Error> {
    let public_key = match transaction.main_signer() {
        MainSigner::SealSigner => return Ok(TransactionRequestMainSigner::SealSigner),
        MainSigner::None => return Ok(TransactionRequestMainSigner::None),
        MainSigner::Authorizer(public_key) => *public_key,
    };
    let key_manager = context.wallet_sdk().key_manager_api();
    for key_id in other_signers {
        if key_manager.get_public_key(*key_id)?.public_key.to_byte_type() == public_key {
            return Ok(TransactionRequestMainSigner::WalletKey { public_key });
        }
    }
    Ok(TransactionRequestMainSigner::External { public_key })
}

/// What actually leaves the wallet, derived from the request's locks.
///
/// A stealth transfer's instructions are commitments and range proofs; the
/// wallet owns the masks and is the only party that can say "10,000 µT leaves
/// this account". Without this the approver is authorising an opaque blob.
///
/// `None` for an ordinary transaction, whose instructions are readable as-is.
fn value_summary(
    context: &HandlerContext,
    lock_ids: &[WalletLockId],
) -> Result<Option<TransactionRequestValueSummary>, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let mut inputs_total = 0u64;
    let mut change_total = 0u64;
    let mut resource_address = None;

    for lock_id in lock_ids {
        let Some(outputs) = sdk.stealth_outputs_api().get_locked_by_lock_id(*lock_id).optional()? else {
            continue;
        };
        for output in outputs {
            resource_address.get_or_insert(output.resource_address);
            match output.status {
                // What this request spends.
                OutputStatus::LockedForSpend => inputs_total = inputs_total.saturating_add(output.value),
                // Outputs the statement created. Only the ones we hold a spend
                // key for are change coming back; the rest belong to the
                // recipient and are genuinely leaving.
                OutputStatus::LockedUnconfirmed if output.owner_key_id.is_some() => {
                    change_total = change_total.saturating_add(output.value)
                },
                _ => {},
            }
        }
    }

    let Some(resource_address) = resource_address else {
        return Ok(None);
    };

    Ok(Some(TransactionRequestValueSummary {
        resource_address,
        inputs_total,
        change_total,
        amount_leaving: inputs_total.saturating_sub(change_total),
    }))
}

#[cfg(test)]
mod tests {
    use tari_ootle_address::Network;
    use tari_ootle_common_types::{Epoch, crypto::create_key_pair, optional::IsNotFoundError};
    use tari_ootle_transaction::{TransactionSignature, UnsignedTransaction};
    use tari_ootle_wallet_sdk::models::{KeyBranch, KeyId, KeyType, LockOwner, WalletLock};

    use super::*;
    use crate::handlers::test_support::TestDaemon;

    fn insert_request(daemon: &TestDaemon) -> TransactionRequestId {
        daemon
            .context
            .wallet_sdk()
            .store()
            .with_write_tx(|tx| {
                tx.transaction_request_insert(
                    &UnsignedTransaction::new(Network::LocalNet.as_byte(), Epoch(100)),
                    KeyId::derived(KeyBranch::Account, 0),
                    &[],
                    &[],
                    &[],
                    Some("htlc-swap-tool"),
                    Duration::from_secs(600),
                )
            })
            .unwrap()
            .id
    }

    fn status_of(daemon: &TestDaemon, request_id: TransactionRequestId) -> TransactionRequestStatus {
        daemon
            .context
            .wallet_sdk()
            .store()
            .with_read_tx(|tx| tx.transaction_request_get(request_id))
            .unwrap()
            .status
    }

    #[tokio::test]
    async fn only_the_wallets_own_session_can_approve() {
        let daemon = TestDaemon::start().await;
        let request_id = insert_request(&daemon);
        let approve = |bearer| {
            handle_approve(&daemon.context, Some(bearer), TransactionRequestDecisionRequest {
                request_id,
            })
        };

        // A key minted before `transaction_requests:approve` stopped being grantable.
        let api_key = daemon.api_key("htlc-swap-tool", "transaction_requests:approve");
        let err = approve(&api_key).await.unwrap_err();
        assert!(err.to_string().contains("interactive user session"), "{err}");

        let admin_key = daemon.api_key("admin-tool", "admin");
        approve(&admin_key).await.unwrap_err();

        // What an API key holding `webrtc` gets back from `webrtc.start`.
        let delegated = daemon.delegated_session("admin");
        let err = approve(&delegated).await.unwrap_err();
        assert!(err.to_string().contains("interactive user session"), "{err}");

        assert_eq!(status_of(&daemon, request_id), TransactionRequestStatus::Pending);

        approve(&daemon.session).await.unwrap();
        assert_eq!(status_of(&daemon, request_id), TransactionRequestStatus::Approved);
    }

    fn create_request(lock_ids: Vec<WalletLockId>, ttl_secs: u64) -> TransactionRequestCreateRequest {
        TransactionRequestCreateRequest {
            transaction: UnsignedTransaction::new(Network::LocalNet.as_byte(), Epoch(100)),
            seal_signer: KeyId::derived(KeyBranch::Account, 0),
            other_signers: vec![],
            signatures: vec![],
            lock_ids,
            ttl_secs: Some(ttl_secs),
        }
    }

    async fn request_info(daemon: &TestDaemon, request_id: TransactionRequestId) -> TransactionRequestInfo {
        handle_get(&daemon.context, Some(&daemon.session), TransactionRequestGetRequest {
            request_id,
        })
        .await
        .unwrap()
        .request
    }

    #[tokio::test]
    async fn the_approver_sees_who_the_transaction_acts_as() {
        let daemon = TestDaemon::start().await;
        let tool = daemon.api_key("htlc-swap-tool", "transaction_requests:create");
        let wallet_key = KeyId::derived(KeyBranch::Account, 0);
        let wallet_public_key = daemon
            .context
            .wallet_sdk()
            .key_manager_api()
            .get_public_key(wallet_key)
            .unwrap()
            .public_key
            .to_byte_type();
        let (external_secret, external_public_key) = create_key_pair();
        let external_public_key = external_public_key.to_byte_type();

        let mut req = create_request(vec![], 600);
        req.transaction
            .set_main_signer(MainSigner::Authorizer(external_public_key));
        let err = handle_create(&daemon.context, Some(&tool), req.clone())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("transaction.main_signer"), "{err}");

        req.signatures = vec![TransactionSignature::sign(
            &external_secret,
            &wallet_public_key,
            &req.transaction,
        )];
        let request_id = handle_create(&daemon.context, Some(&tool), req)
            .await
            .unwrap()
            .request_id;
        let info = request_info(&daemon, request_id).await;
        assert_eq!(info.main_signer, TransactionRequestMainSigner::External {
            public_key: external_public_key
        });
        assert_eq!(info.external_signers, vec![external_public_key]);

        let mut req = create_request(vec![], 600);
        req.other_signers = vec![wallet_key];
        req.transaction
            .set_main_signer(MainSigner::Authorizer(wallet_public_key));
        let request_id = handle_create(&daemon.context, Some(&tool), req)
            .await
            .unwrap()
            .request_id;
        let info = request_info(&daemon, request_id).await;
        assert_eq!(info.main_signer, TransactionRequestMainSigner::WalletKey {
            public_key: wallet_public_key
        });
        assert!(info.external_signers.is_empty());
    }

    fn lock_for(daemon: &TestDaemon, owner: LockOwner) -> WalletLockId {
        daemon
            .context
            .wallet_sdk()
            .locks_api()
            .create_owned_lock_with_timeout(Duration::from_secs(5 * 60), owner)
            .unwrap()
            .keep_locked()
    }

    fn get_lock(daemon: &TestDaemon, lock_id: WalletLockId) -> WalletLock {
        daemon
            .context
            .wallet_sdk()
            .store()
            .with_read_tx(|tx| tx.locks_get(lock_id))
            .unwrap()
    }

    fn owner_of(daemon: &TestDaemon, bearer: &Bearer) -> LockOwner {
        daemon
            .context
            .check_auth_with_identity(Some(bearer))
            .unwrap()
            .lock_owner()
    }

    #[tokio::test]
    async fn a_request_cannot_hold_a_lock_another_caller_created() {
        let daemon = TestDaemon::start().await;
        let tool = daemon.api_key("htlc-swap-tool", "transaction_requests:create");
        let other_tool = daemon.api_key("other-tool", "transaction_requests:create");
        let app = daemon.delegated_session("transaction_requests:create");
        let foreign = [
            lock_for(&daemon, LockOwner::Wallet),
            lock_for(&daemon, owner_of(&daemon, &daemon.cookie)),
            lock_for(&daemon, owner_of(&daemon, &other_tool)),
            lock_for(&daemon, owner_of(&daemon, &app)),
        ];

        // A cookie caller's locks are distinct from the interactive wallet session's locks.
        let wallet_lock = lock_for(&daemon, LockOwner::Wallet);
        let err = handle_create(
            &daemon.context,
            Some(&daemon.cookie),
            create_request(vec![wallet_lock], 60 * 60),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("lock_ids"), "{err}");

        let cookie_lock = lock_for(&daemon, owner_of(&daemon, &daemon.cookie));
        let err = handle_create(
            &daemon.context,
            Some(&daemon.session),
            create_request(vec![cookie_lock], 60 * 60),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("lock_ids"), "{err}");

        let cookie_req_id = handle_create(
            &daemon.context,
            Some(&daemon.cookie),
            create_request(vec![cookie_lock], 60 * 60),
        )
        .await
        .unwrap()
        .request_id;
        assert_eq!(
            request_info(&daemon, cookie_req_id).await.requested_by.as_deref(),
            Some("local-cookie")
        );

        for lock_id in foreign {
            let deadline = get_lock(&daemon, lock_id).timeout_at;
            let err = handle_create(&daemon.context, Some(&tool), create_request(vec![lock_id], 60 * 60))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("lock_ids"), "{err}");
            assert_eq!(
                get_lock(&daemon, lock_id).timeout_at,
                deadline,
                "lock {lock_id} was moved"
            );
        }

        // A missing id is refused the same way as a foreign one.
        let missing = handle_create(&daemon.context, Some(&tool), create_request(vec![9999], 60 * 60))
            .await
            .unwrap_err();
        let foreign_err = handle_create(&daemon.context, Some(&tool), create_request(vec![foreign[0]], 60 * 60))
            .await
            .unwrap_err();
        assert_eq!(
            missing.to_string().replace("9999", "N"),
            foreign_err.to_string().replace(&foreign[0].to_string(), "N")
        );
    }

    #[tokio::test]
    async fn connected_apps_cannot_hold_each_others_locks() {
        let daemon = TestDaemon::start().await;
        let app = daemon.delegated_session("transaction_requests:create");
        let other_app = daemon.delegated_session("transaction_requests:create");
        let lock_id = lock_for(&daemon, owner_of(&daemon, &other_app));

        let err = handle_create(&daemon.context, Some(&app), create_request(vec![lock_id], 60 * 60))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("lock_ids"), "{err}");

        handle_create(
            &daemon.context,
            Some(&other_app),
            create_request(vec![lock_id], 60 * 60),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_request_holds_its_own_callers_lock_across_the_approval_window() {
        let daemon = TestDaemon::start().await;
        let tool = daemon.api_key("htlc-swap-tool", "transaction_requests:create");
        let lock_id = lock_for(&daemon, owner_of(&daemon, &tool));
        let deadline = get_lock(&daemon, lock_id).timeout_at.unwrap();

        handle_create(&daemon.context, Some(&tool), create_request(vec![lock_id], 60 * 60))
            .await
            .unwrap();

        let extended = get_lock(&daemon, lock_id).timeout_at.unwrap();
        assert!(
            extended - deadline >= time::Duration::minutes(55),
            "{deadline} -> {extended}"
        );
    }

    #[tokio::test]
    async fn a_lock_is_held_by_at_most_one_live_request() {
        let daemon = TestDaemon::start().await;
        let tool = daemon.api_key("htlc-swap-tool", "transaction_requests:create");
        let lock_id = lock_for(&daemon, owner_of(&daemon, &tool));

        let first = handle_create(&daemon.context, Some(&tool), create_request(vec![lock_id], 60 * 60))
            .await
            .unwrap();
        handle_create(&daemon.context, Some(&tool), create_request(vec![lock_id], 60 * 60))
            .await
            .unwrap_err();
        handle_create(
            &daemon.context,
            Some(&tool),
            create_request(vec![lock_for(&daemon, owner_of(&daemon, &tool)); 2], 60 * 60),
        )
        .await
        .unwrap_err();

        // The first request's rejection releases the lock, so nothing holds it.
        handle_reject(
            &daemon.context,
            Some(&daemon.session),
            TransactionRequestDecisionRequest {
                request_id: first.request_id,
            },
        )
        .await
        .unwrap();
        let released = daemon
            .context
            .wallet_sdk()
            .store()
            .with_read_tx(|tx| tx.locks_get(lock_id))
            .unwrap_err();
        assert!(released.is_not_found_error(), "{released}");
    }

    #[tokio::test]
    async fn a_request_never_brings_a_lock_deadline_forward() {
        let daemon = TestDaemon::start().await;
        let lock_id = daemon
            .context
            .wallet_sdk()
            .locks_api()
            .create_lock()
            .unwrap()
            .keep_locked();

        handle_create(&daemon.context, Some(&daemon.session), create_request(vec![lock_id], 0))
            .await
            .unwrap();

        assert_eq!(get_lock(&daemon, lock_id).timeout_at, None);
    }

    #[tokio::test]
    async fn a_request_cannot_name_a_mask_or_view_key_as_a_signer() {
        let daemon = TestDaemon::start().await;
        let tool = daemon.api_key("htlc-swap-tool", "transaction_requests:create");

        for branch in [
            KeyBranch::ConfidentialMask,
            KeyBranch::StealthMask,
            KeyBranch::ViewOnlyKey,
            KeyBranch::ElgamalEncryptionViewKey,
        ] {
            let mut req = create_request(vec![], 60 * 60);
            req.other_signers = vec![KeyId::derived(branch, 0)];
            let err = handle_create(&daemon.context, Some(&tool), req).await.unwrap_err();
            assert!(err.to_string().contains("other_signers"), "{err}");

            let mut req = create_request(vec![], 60 * 60);
            req.seal_signer = KeyId::derived(branch, 0);
            let err = handle_create(&daemon.context, Some(&tool), req).await.unwrap_err();
            assert!(err.to_string().contains("seal_signer"), "{err}");
        }

        let stored = daemon
            .context
            .wallet_sdk()
            .store()
            .with_read_tx(|tx| tx.transaction_requests_list())
            .unwrap();
        assert!(stored.is_empty(), "a refused request was stored");

        let mut req = create_request(vec![], 60 * 60);
        req.other_signers = vec![KeyId::derived(KeyBranch::Nonce, 0)];
        handle_create(&daemon.context, Some(&tool), req).await.unwrap();
    }
    #[tokio::test]
    async fn a_request_cannot_name_a_view_only_accounts_view_key_as_a_signer() {
        let daemon = TestDaemon::start().await;
        let tool = daemon.api_key("htlc-swap-tool", "transaction_requests:create");
        let (_, account_public_key) = create_key_pair();
        let (view_secret_key, _) = create_key_pair();
        let account = daemon
            .context
            .wallet_sdk()
            .accounts_api()
            .add_view_only_account(
                Some("watch"),
                &account_public_key.to_byte_type(),
                &view_secret_key,
                None,
            )
            .unwrap();
        let view_key_id = account.view_only_key_id();
        assert!(matches!(view_key_id, KeyId::Imported { .. }));

        let mut req = create_request(vec![], 60 * 60);
        req.other_signers = vec![view_key_id];
        let err = handle_create(&daemon.context, Some(&tool), req).await.unwrap_err();
        assert!(err.to_string().contains("other_signers"), "{err}");

        let mut req = create_request(vec![], 60 * 60);
        req.seal_signer = view_key_id;
        let err = handle_create(&daemon.context, Some(&tool), req).await.unwrap_err();
        assert!(err.to_string().contains("seal_signer"), "{err}");

        let (secret, _) = create_key_pair();
        let general_key_id = daemon
            .context
            .wallet_sdk()
            .key_manager_api()
            .import_key("general", &secret, KeyType::GeneralPurpose)
            .unwrap();
        let mut req = create_request(vec![], 60 * 60);
        req.other_signers = vec![general_key_id];
        handle_create(&daemon.context, Some(&tool), req).await.unwrap();
    }
}
