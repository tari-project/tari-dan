//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{FixedHash, PrivateKey};
use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::{
    models::TransactionStatus,
    storage::{WalletStore, WalletStoreReader, WalletStoreWriter},
};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_transaction::Transaction;

fn build_transaction() -> Transaction {
    Transaction::builder().sign(&PrivateKey::default()).build()
}

#[test]
fn get_and_insert_transaction() {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    let mut tx = db.create_write_tx().unwrap();
    let transaction = tx.transaction_get(FixedHash::zero()).optional().unwrap();
    assert!(transaction.is_none());
    let transaction = build_transaction();
    let hash = *transaction.hash();
    tx.transactions_insert(&transaction, false).unwrap();
    tx.commit().unwrap();

    let mut tx = db.create_read_tx().unwrap();
    let returned = tx.transaction_get(hash.into_array().into()).unwrap();
    assert_eq!(transaction, returned.transaction);
    assert_eq!(returned.status, TransactionStatus::default());
}
