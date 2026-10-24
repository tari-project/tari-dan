//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Storage-level tests for lock timeout behaviour.
//!
//! A wallet lock holds selected inputs (`LockedForSpend`) and the change
//! outputs created alongside them (`LockedUnconfirmed`). The transfer-selection
//! handlers create locks with a short timeout, and the transaction service
//! periodically sweeps any lock past its deadline. A transaction request must
//! hold its inputs across a human approval window that is longer than that
//! deadline, so it extends the timeout of the locks it is given.

use std::time::Duration;

use tari_ootle_common_types::optional::IsNotFoundError;
use tari_ootle_transaction::TransactionId;
use tari_ootle_wallet_sdk::{
    models::LockOwner,
    storage::{CommittableStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;

fn open_store() -> SqliteWalletStore {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    db
}

#[test]
fn a_lock_past_its_deadline_is_swept() {
    // Control for the tests below: without it they would pass even if a zero
    // timeout were never stale in the first place.
    let db = open_store();
    let mut tx = db.create_write_tx().unwrap();

    tx.locks_create(Some(Duration::ZERO), LockOwner::Wallet).unwrap();

    assert_eq!(tx.locks_release_stale().unwrap(), 1, "a lock at its deadline is stale");
    tx.commit().unwrap();
}

#[test]
fn a_lock_linked_to_a_transaction_is_not_swept() {
    // A lock tied to an in-flight transaction is released by that
    // transaction's resolution, never by the stale sweep -- reaping it mid-flight
    // would unlock the inputs and delete the change outputs of a live
    // transaction. The link protects it regardless of its timeout.
    let db = open_store();
    let mut tx = db.create_write_tx().unwrap();

    let lock = tx.locks_create(Some(Duration::ZERO), LockOwner::Wallet).unwrap();
    tx.locks_link_transaction(lock, TransactionId::new([1u8; 32])).unwrap();

    assert_eq!(
        tx.locks_release_stale().unwrap(),
        0,
        "a lock linked to a transaction is not stale even past its deadline"
    );
    tx.commit().unwrap();
}

#[test]
fn extending_the_timeout_of_an_unknown_lock_is_not_found() {
    // `transaction_requests.create` extends caller-named locks. A bad id must
    // surface here, not silently create a request whose locks were never
    // extended past the approval window.
    let db = open_store();
    let mut tx = db.create_write_tx().unwrap();

    let err = tx.locks_extend_timeout(9999, Duration::from_secs(60)).unwrap_err();
    assert!(err.is_not_found_error(), "expected NotFound, got: {err}");
}

#[test]
fn a_linked_lock_has_no_timeout_to_extend() {
    let db = open_store();
    let mut tx = db.create_write_tx().unwrap();

    let lock = tx.locks_create(Some(Duration::ZERO), LockOwner::Wallet).unwrap();
    tx.locks_link_transaction(lock, TransactionId::new([1u8; 32])).unwrap();

    let err = tx.locks_extend_timeout(lock, Duration::from_secs(60)).unwrap_err();
    assert!(err.is_not_found_error(), "expected NotFound, got: {err}");
}

#[test]
fn extending_a_timeout_saves_a_lock_from_the_stale_sweep() {
    let db = open_store();
    let mut tx = db.create_write_tx().unwrap();

    // A zero timeout puts `timeout_at` at the current second, so the sweep
    // considers the lock stale immediately. See the companion test.
    let lock = tx.locks_create(Some(Duration::ZERO), LockOwner::Wallet).unwrap();
    tx.locks_extend_timeout(lock, Duration::from_secs(60 * 60)).unwrap();

    assert_eq!(
        tx.locks_release_stale().unwrap(),
        0,
        "a lock whose timeout was extended must survive the sweep"
    );
    tx.commit().unwrap();
}

#[test]
fn extending_never_brings_a_deadline_forward() {
    let db = open_store();
    let mut tx = db.create_write_tx().unwrap();

    let later = tx
        .locks_create(Some(Duration::from_secs(24 * 60 * 60)), LockOwner::Wallet)
        .unwrap();
    let deadline = tx.locks_get(later).unwrap().timeout_at;
    tx.locks_extend_timeout(later, Duration::ZERO).unwrap();
    assert_eq!(tx.locks_get(later).unwrap().timeout_at, deadline);

    let open = tx.locks_create(None, LockOwner::Wallet).unwrap();
    tx.locks_extend_timeout(open, Duration::ZERO).unwrap();
    assert_eq!(tx.locks_get(open).unwrap().timeout_at, None, "no deadline stays none");

    assert_eq!(tx.locks_release_stale().unwrap(), 0);
    tx.commit().unwrap();
}

#[test]
fn a_lock_records_its_owner() {
    let db = open_store();
    let mut tx = db.create_write_tx().unwrap();

    for owner in [
        LockOwner::Wallet,
        LockOwner::LocalCookie,
        LockOwner::Delegated(u64::MAX),
        LockOwner::Delegated(3),
        LockOwner::ApiKey(7),
    ] {
        let lock = tx.locks_create(None, owner).unwrap();
        assert_eq!(tx.locks_get(lock).unwrap().owner, owner);
    }
}
