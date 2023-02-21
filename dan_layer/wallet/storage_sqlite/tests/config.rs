//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::storage::{WalletStore, WalletStoreReader, WalletStoreWriter};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;

#[test]
fn get_and_set_value() {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    let mut tx = db.create_write_tx().unwrap();
    let rec = tx.config_get::<()>("dummy").optional().unwrap();
    assert!(rec.is_none());
    tx.config_set("dummy", &123u32, false).unwrap();
    tx.commit().unwrap();

    let mut tx = db.create_read_tx().unwrap();
    let rec = tx.config_get::<u32>("dummy").unwrap();
    assert_eq!(rec.value, 123);
}
