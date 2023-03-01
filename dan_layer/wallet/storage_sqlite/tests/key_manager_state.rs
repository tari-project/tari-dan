//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::storage::{WalletStore, WalletStoreReader, WalletStoreWriter};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;

#[test]
fn get_and_set_branch_index() {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    let mut tx = db.create_write_tx().unwrap();
    let index = tx.key_manager_get_active_index("").optional().unwrap();
    assert!(index.is_none());
    tx.key_manager_set_active_index("", 123).unwrap();
    tx.key_manager_set_active_index("another", 321).unwrap();
    tx.commit().unwrap();

    let mut tx = db.create_read_tx().unwrap();
    let index = tx.key_manager_get_active_index("").unwrap();
    assert_eq!(index, 123);
    let index = tx.key_manager_get_active_index("another").unwrap();
    assert_eq!(index, 321);
}
