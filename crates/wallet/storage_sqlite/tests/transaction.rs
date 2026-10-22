//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, str::FromStr};

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_common_types::{Epoch, optional::Optional};
use tari_ootle_transaction::{Transaction, TransactionId, args};
use tari_ootle_wallet_sdk::{
    models::{KeyBranch, KeyId, TransactionStatus},
    storage::{CommittableStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_template_lib_types::ComponentAddress;

fn build_transaction() -> Transaction {
    let key = RistrettoSecretKey::from(123u64);
    Transaction::builder_localnet(Epoch(1))
        .allocate_component_address("component")
        .put_last_instruction_output_on_workspace("bucket")
        .call_method("component", "new", args!["bucket"])
        .with_dry_run(true)
        .build_and_seal(&key)
}

/// Builds a distinct, non-dry-run transaction. Varying `seed` varies the seal signer and therefore the
/// transaction id, so multiple transactions can be inserted in one test.
fn build_linked_transaction(seed: u64) -> Transaction {
    let key = RistrettoSecretKey::from(seed);
    Transaction::builder_localnet(Epoch(1))
        .allocate_component_address("component")
        .put_last_instruction_output_on_workspace("bucket")
        .call_method("component", "new", args!["bucket"])
        .build_and_seal(&key)
}

fn insert_account(tx: &mut impl WalletStoreWriter, name: &str, address: &ComponentAddress, key_index: u64) {
    // accounts.owner_public_key is unique, so derive a distinct key per account.
    let owner_public_key =
        RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(key_index + 1000)).to_byte_type();
    tx.accounts_insert(
        Some(name),
        address,
        KeyId::derived(KeyBranch::Account, key_index),
        Some(KeyId::derived(KeyBranch::Account, key_index)),
        &owner_public_key,
        &Default::default(),
        Epoch::zero(),
        false,
        false,
    )
    .unwrap();
}

#[test]
fn get_and_insert_transaction() {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    let mut tx = db.create_write_tx().unwrap();
    let transaction = tx.transactions_get(TransactionId::default()).optional().unwrap();
    assert!(transaction.is_none());

    let transaction = build_transaction();
    assert!(transaction.verify_all_signatures());
    let tx_id = transaction.calculate_id();
    tx.transactions_insert(&transaction, None, &[], false).unwrap();
    tx.commit().unwrap();

    let returned = tx.transactions_get(tx_id).unwrap();
    // Transaction was not malleated in the database
    assert!(returned.transaction.verify_all_signatures());
    assert_eq!(returned.transaction.calculate_id(), tx_id);
    assert_eq!(returned.status, TransactionStatus::default());
}

#[test]
fn fetch_all_filters_by_linked_account() {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();

    let account_a =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8aaaaaaaa")
            .unwrap();
    let account_b =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8bbbbbbbb")
            .unwrap();
    let unknown_account =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8cccccccc")
            .unwrap();

    let mut tx = db.create_write_tx().unwrap();
    insert_account(&mut tx, "a", &account_a, 0);
    insert_account(&mut tx, "b", &account_b, 1);

    // tx1 -> A, tx2 -> A & B (self-transfer), tx3 -> B
    let tx1 = build_linked_transaction(1);
    let tx1_id = tx1.calculate_id();
    tx.transactions_insert(&tx1, None, &[account_a], false).unwrap();

    let tx2 = build_linked_transaction(2);
    let tx2_id = tx2.calculate_id();
    tx.transactions_insert(&tx2, None, &[account_a, account_b], false)
        .unwrap();

    let tx3 = build_linked_transaction(3);
    let tx3_id = tx3.calculate_id();
    tx.transactions_insert(&tx3, None, &[account_b], false).unwrap();

    tx.commit().unwrap();

    // No account filter -> all transactions.
    let all = tx.transactions_fetch_all(None, None).unwrap();
    assert_eq!(all.len(), 3);

    // Filter by account A -> tx1 and tx2.
    let by_a: HashSet<TransactionId> = tx
        .transactions_fetch_all(None, Some(account_a))
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(by_a, HashSet::from([tx1_id, tx2_id]));

    // Filter by account B -> tx2 and tx3.
    let by_b: HashSet<TransactionId> = tx
        .transactions_fetch_all(None, Some(account_b))
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(by_b, HashSet::from([tx2_id, tx3_id]));

    // Filter by an account the wallet does not know -> no transactions.
    let by_unknown = tx.transactions_fetch_all(None, Some(unknown_account)).unwrap();
    assert!(by_unknown.is_empty());
}

#[test]
fn unknown_linked_accounts_are_ignored() {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();

    let known =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8aaaaaaaa")
            .unwrap();
    let unknown =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8cccccccc")
            .unwrap();

    let mut tx = db.create_write_tx().unwrap();
    insert_account(&mut tx, "known", &known, 0);

    // Link to both a known and an unknown account. The unknown one is silently skipped (no FK violation).
    let transaction = build_linked_transaction(7);
    let tx_id = transaction.calculate_id();
    tx.transactions_insert(&transaction, None, &[known, unknown], false)
        .unwrap();
    tx.commit().unwrap();

    let by_known: Vec<_> = tx.transactions_fetch_all(None, Some(known)).unwrap();
    assert_eq!(by_known.len(), 1);
    assert_eq!(by_known[0].id, tx_id);
}
