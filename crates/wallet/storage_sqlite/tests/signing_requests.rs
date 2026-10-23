//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Storage-level tests for the signing-request state machine: a request is
//! born `Pending`, is decided at most once, cannot be decided after its window
//! closes, and carries a signature exactly when it is `Signed`.

use std::{thread, time::Duration};

use tari_ootle_common_types::{Epoch, optional::IsNotFoundError};
use tari_ootle_transaction::{Sr25519SignatureBytes, TransactionSignature, UnsignedTransaction};
use tari_ootle_wallet_sdk::{
    models::{
        KeyBranch,
        KeyId,
        NewSigningRequest,
        SigningRequestEffectiveStatus,
        SigningRequestId,
        SigningRequestStatus,
        SigningRequester,
    },
    storage::{CommittableStore, ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

fn open_store() -> SqliteWalletStore {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    db
}

fn key_id() -> KeyId {
    KeyId::Derived {
        key_branch: KeyBranch::Account,
        index: 3u64,
    }
}

fn signature() -> TransactionSignature {
    TransactionSignature::new(RistrettoPublicKeyBytes::default(), Sr25519SignatureBytes::zero())
}

fn insert_request(db: &SqliteWalletStore, ttl: Duration) -> SigningRequestId {
    let transaction = UnsignedTransaction::new(0x10u8, Epoch(50));
    let mut tx = db.create_write_tx().unwrap();
    let model = tx
        .signing_request_insert(NewSigningRequest {
            unsigned_transaction: &transaction,
            seal_public_key: &RistrettoPublicKeyBytes::default(),
            key_id: key_id(),
            signer_public_key: &RistrettoPublicKeyBytes::default(),
            message_hash: &[0xab; 64],
            memo: "set burn rate to 7%",
            requester: &SigningRequester::ApiKey {
                name: "governance-signer".to_string(),
            },
            ttl,
        })
        .unwrap();
    tx.commit().unwrap();
    model.id
}

#[test]
fn insert_and_get_round_trips_as_pending() {
    let db = open_store();
    let id = insert_request(&db, Duration::from_secs(600));

    let found = db.with_read_tx(|tx| tx.signing_request_get(id)).unwrap();
    assert_eq!(found.status, SigningRequestStatus::Pending);
    assert_eq!(found.key_id, key_id());
    assert_eq!(found.message_hash, [0xab; 64]);
    assert_eq!(found.memo, "set burn rate to 7%");
    assert_eq!(found.requester, SigningRequester::ApiKey {
        name: "governance-signer".to_string()
    });
    assert_eq!(found.unsigned_transaction.max_epoch(), Epoch(50));
    assert!(found.signature.is_none());
    assert!(found.decided_at.is_none());
}

#[test]
fn signing_stores_the_signature() {
    let db = open_store();
    let id = insert_request(&db, Duration::from_secs(600));

    let signed = db
        .with_write_tx(|tx| tx.signing_request_mark_signed(id, &signature()))
        .unwrap();
    assert_eq!(signed.status, SigningRequestStatus::Signed);
    assert_eq!(signed.signature, Some(signature()));
    assert!(signed.decided_at.is_some());

    let found = db.with_read_tx(|tx| tx.signing_request_get(id)).unwrap();
    assert_eq!(found.signature, Some(signature()));
}

#[test]
fn a_request_is_decided_at_most_once() {
    let db = open_store();
    let id = insert_request(&db, Duration::from_secs(600));

    db.with_write_tx(|tx| tx.signing_request_mark_signed(id, &signature()))
        .unwrap();
    assert!(
        db.with_write_tx(|tx| tx.signing_request_mark_signed(id, &signature()))
            .is_err()
    );
    assert!(db.with_write_tx(|tx| tx.signing_request_reject(id)).is_err());

    let other = insert_request(&db, Duration::from_secs(600));
    let rejected = db.with_write_tx(|tx| tx.signing_request_reject(other)).unwrap();
    assert_eq!(rejected.status, SigningRequestStatus::Rejected);
    assert!(rejected.signature.is_none());
    assert!(
        db.with_write_tx(|tx| tx.signing_request_mark_signed(other, &signature()))
            .is_err(),
        "a rejected request can never be signed"
    );
}

#[test]
fn an_expired_request_cannot_be_signed() {
    let db = open_store();
    let id = insert_request(&db, Duration::ZERO);
    thread::sleep(Duration::from_millis(5));

    let err = db
        .with_write_tx(|tx| tx.signing_request_mark_signed(id, &signature()))
        .unwrap_err();
    assert!(!err.is_not_found_error(), "expected a state error, got: {err}");

    let found = db.with_read_tx(|tx| tx.signing_request_get(id)).unwrap();
    assert_eq!(found.status, SigningRequestStatus::Pending);
    assert_eq!(found.effective_status_now(), SigningRequestEffectiveStatus::Expired);
    assert!(found.signature.is_none());
}

#[test]
fn deleting_expired_requests_keeps_open_and_decided_ones() {
    let db = open_store();
    let expired = insert_request(&db, Duration::ZERO);
    let decided = insert_request(&db, Duration::from_secs(600));
    let open = insert_request(&db, Duration::from_secs(600));
    db.with_write_tx(|tx| tx.signing_request_mark_signed(decided, &signature()))
        .unwrap();
    thread::sleep(Duration::from_millis(5));

    assert_eq!(db.with_read_tx(|tx| tx.signing_requests_count_pending()).unwrap(), 1);
    assert_eq!(db.with_write_tx(|tx| tx.signing_requests_delete_expired()).unwrap(), 1);

    let ids = db
        .with_read_tx(|tx| tx.signing_requests_list())
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![open, decided]);
    assert!(!ids.contains(&expired));
}

#[test]
fn deciding_an_unknown_request_is_not_found() {
    let db = open_store();
    let err = db
        .with_write_tx(|tx| tx.signing_request_mark_signed(42, &signature()))
        .unwrap_err();
    assert!(err.is_not_found_error(), "expected not found, got: {err}");
}

#[test]
fn list_returns_newest_first() {
    let db = open_store();
    let first = insert_request(&db, Duration::from_secs(600));
    let second = insert_request(&db, Duration::from_secs(600));

    let ids = db
        .with_read_tx(|tx| tx.signing_requests_list())
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![second, first]);
}
