//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::{
    models::VersionedSubstateId,
    storage::{WalletStore, WalletStoreReader, WalletStoreWriter},
};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_engine_types::substate::SubstateId;
use tari_transaction::TransactionId;

#[test]
fn get_and_insert_substates() {
    let example_addr = "component_1f019e4d434cbf2b99c0af89ee212f422af86de7280a169d2e392dfbffffffff"
        .parse()
        .unwrap();

    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    let mut tx = db.create_write_tx().unwrap();
    let substate = tx.substates_get(&example_addr).optional().unwrap();
    assert!(substate.is_none());
    let hash = TransactionId::default();
    let address =
        SubstateId::from_str("component_1f019e4d434cbf2b99c0af89ee212f422af86de7280a169d2e392dfbffffffff").unwrap();
    tx.substates_upsert_root(
        hash,
        VersionedSubstateId {
            substate_id: address.clone(),
            version: 0,
        },
        None,
        None,
    )
    .unwrap();

    let child_address =
        SubstateId::from_str("component_d9e4a7ce7dbaa73ce10aabf309dd702054756a813f454ef13564f298ffffffff").unwrap();
    tx.substates_upsert_child(hash, address.clone(), VersionedSubstateId {
        substate_id: child_address.clone(),
        version: 0,
    })
    .unwrap();

    tx.commit().unwrap();

    let mut tx = db.create_read_tx().unwrap();
    let returned = tx.substates_get(&address).unwrap();
    assert!(returned.parent_address.is_none());
    assert_eq!(returned.address.substate_id, address);
    assert_eq!(returned.address.version, 0);

    let returned = tx.substates_get(&child_address).unwrap();
    assert_eq!(returned.parent_address, Some(address));
    assert_eq!(returned.address.substate_id, child_address);
    assert_eq!(returned.address.version, 0);
}
