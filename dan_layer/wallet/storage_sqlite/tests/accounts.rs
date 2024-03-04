//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use tari_dan_wallet_sdk::storage::{WalletStore, WalletStoreReader, WalletStoreWriter};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_engine_types::substate::SubstateId;

#[test]
fn update_account() {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    let address = SubstateId::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8").unwrap();
    let mut tx = db.create_write_tx().unwrap();
    tx.accounts_insert(Some("test"), &address, 0, false).unwrap();
    tx.accounts_update(&address, Some("foo")).unwrap();
    tx.commit().unwrap();

    let mut tx = db.create_read_tx().unwrap();
    let account = tx.accounts_get_by_name("foo").unwrap();
    assert_eq!(account.name.as_deref(), Some("foo"));
}
