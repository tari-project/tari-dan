//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::storage::{WalletStore, WalletStoreReader};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;

#[test]
fn get_and_insert_substates() {
    let example_addr = "component_1f019e4d434cbf2b99c0af89ee212f422af86de7280a169d2e392dfb66ab34d4"
        .parse()
        .unwrap();

    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    let tx = db.create_write_tx().unwrap();
    let address = tx.substates_get(&example_addr).optional().unwrap();
    assert!(address.is_none());
    // let hash = *transaction.hash();
    // tx.transactions_insert(&transaction).unwrap();
    // tx.commit().unwrap();

    // let tx = db.create_read_tx().unwrap();
    // let returned = tx.transaction_get(hash.into_array().into()).unwrap();
    // assert_eq!(transaction, returned.transaction);
    // assert_eq!(returned.status, TransactionStatus::default());
}
