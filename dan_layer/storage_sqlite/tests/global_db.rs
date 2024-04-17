//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::{Connection, SqliteConnection};
use rand::rngs::OsRng;
use tari_common_types::types::PublicKey;
use tari_crypto::keys::PublicKey as _;
use tari_dan_common_types::{Epoch, PeerAddress, SubstateAddress};
use tari_dan_storage::global::{GlobalDb, ValidatorNodeDb};
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_utilities::ByteArray;

fn create_db() -> GlobalDb<SqliteGlobalDbAdapter<PeerAddress>> {
    let conn = SqliteConnection::establish(":memory:").unwrap();
    let db = GlobalDb::new(SqliteGlobalDbAdapter::new(conn));
    db.adapter().migrate().unwrap();
    db
}

fn new_public_key() -> PublicKey {
    PublicKey::random_keypair(&mut OsRng).1
}

fn derived_substate_address(public_key: &PublicKey) -> SubstateAddress {
    SubstateAddress::from_bytes(public_key.as_bytes()).unwrap()
}

fn insert_vns(
    validator_nodes: &mut ValidatorNodeDb<'_, '_, SqliteGlobalDbAdapter<PeerAddress>>,
    num: usize,
    epoch: Epoch,
    sidechain_id: Option<PublicKey>,
) {
    for _ in 0..num {
        insert_vn_with_public_key(validator_nodes, new_public_key(), epoch, sidechain_id.clone())
    }
}

fn insert_vn_with_public_key(
    validator_nodes: &mut ValidatorNodeDb<'_, '_, SqliteGlobalDbAdapter<PeerAddress>>,
    public_key: PublicKey,
    epoch: Epoch,
    sidechain_id: Option<PublicKey>,
) {
    validator_nodes
        .insert_validator_node(
            public_key.clone().into(),
            public_key.clone(),
            derived_substate_address(&public_key),
            epoch,
            public_key,
            sidechain_id,
        )
        .unwrap()
}

#[test]
fn insert_and_get_within_epoch() {
    let db = create_db();
    let mut tx = db.create_transaction().unwrap();
    let mut validator_nodes = db.validator_nodes(&mut tx);
    insert_vns(&mut validator_nodes, 2, Epoch(0), None);
    insert_vns(&mut validator_nodes, 1, Epoch(10), None);

    let vns = validator_nodes.get_all_within_epochs(Epoch(0), Epoch(10)).unwrap();
    assert_eq!(vns.len(), 3);
}

#[test]
fn insert_and_get_within_epoch_duplicate_public_keys() {
    let db = create_db();
    let mut tx = db.create_transaction().unwrap();
    let mut validator_nodes = db.validator_nodes(&mut tx);
    insert_vns(&mut validator_nodes, 2, Epoch(0), None);
    insert_vns(&mut validator_nodes, 1, Epoch(10), None);
    let pk = new_public_key();
    insert_vn_with_public_key(&mut validator_nodes, pk.clone(), Epoch(0), None);
    insert_vn_with_public_key(&mut validator_nodes, pk, Epoch(1), None);

    let vns = validator_nodes.get_all_within_epochs(Epoch(0), Epoch(10)).unwrap();
    assert_eq!(vns.len(), 4);
}

#[test]
fn insert_and_get_within_shard_range_duplicate_public_keys() {
    // Testing fetching within a shard range. Specifically, the ability for Sqlite to compare blob columns
    let db = create_db();
    let mut tx = db.create_transaction().unwrap();
    let mut validator_nodes = db.validator_nodes(&mut tx);
    // Insert lower shard key
    insert_vn_with_public_key(&mut validator_nodes, PublicKey::default(), Epoch(0), None);

    let pk = new_public_key();
    insert_vn_with_public_key(&mut validator_nodes, pk.clone(), Epoch(0), None);
    insert_vn_with_public_key(&mut validator_nodes, pk.clone(), Epoch(1), None);
    let pk2 = new_public_key();
    insert_vn_with_public_key(&mut validator_nodes, pk2.clone(), Epoch(1), None);

    let shard_id = derived_substate_address(&pk);
    let shard_id2 = derived_substate_address(&pk2);
    let (start, end) = if shard_id > shard_id2 {
        (shard_id2, shard_id)
    } else {
        (shard_id, shard_id2)
    };

    let vns = validator_nodes
        .get_by_shard_range(Epoch(0), Epoch(10), start..=end)
        .unwrap();
    if shard_id > shard_id2 {
        assert_eq!(vns[0].public_key, pk2);
        assert_eq!(vns[1].public_key, pk);
    } else {
        assert_eq!(vns[0].public_key, pk);
        assert_eq!(vns[1].public_key, pk2);
    }
    assert_eq!(vns.len(), 2);
}
