//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;
use diesel::{Connection, SqliteConnection};
use rand::rngs::OsRng;
use tari_common_types::types::PublicKey;
use tari_crypto::keys::PublicKey as _;
use tari_dan_common_types::{shard::Shard, Epoch, PeerAddress, SubstateAddress};
use tari_dan_storage::global::{GlobalDb, ValidatorNodeDb};
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_utilities::ByteArray;

fn create_db() -> GlobalDb<SqliteGlobalDbAdapter<PeerAddress>> {
    // std::fs::remove_file("/tmp/tmptmp.db").ok();
    // let conn = SqliteConnection::establish("file:///tmp/tmptmp.db").unwrap();
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
        insert_vn_with_public_key(validator_nodes, new_public_key(), epoch, epoch + Epoch(1), sidechain_id.clone())
    }
}

fn insert_vn_with_public_key(
    validator_nodes: &mut ValidatorNodeDb<'_, '_, SqliteGlobalDbAdapter<PeerAddress>>,
    public_key: PublicKey,
    start_epoch: Epoch,
    end_epoch: Epoch,
    sidechain_id: Option<PublicKey>,
) {
    validator_nodes
        .insert_validator_node(
            public_key.clone().into(),
            public_key.clone(),
            derived_substate_address(&public_key),
            0,
            start_epoch,
            end_epoch,
            public_key,
            sidechain_id,
        )
        .unwrap()
}

fn update_committee_bucket(
    validator_nodes: &mut ValidatorNodeDb<'_, '_, SqliteGlobalDbAdapter<PeerAddress>>,
    public_key: &PublicKey,
    committee_bucket: Shard,
    epoch: Epoch,
) {
    validator_nodes
        .set_committee_bucket(derived_substate_address(public_key), committee_bucket, None, epoch)
        .unwrap();
}

#[test]
fn insert_and_get_within_epoch() {
    let db = create_db();
    let mut tx = db.create_transaction().unwrap();
    let mut validator_nodes = db.validator_nodes(&mut tx);
    insert_vns(&mut validator_nodes, 3, Epoch(0), None);
    insert_vns(&mut validator_nodes, 2, Epoch(1), None);
    let vns = validator_nodes
        .get_all_within_epoch(Epoch(0), None)
        .unwrap();
    assert_eq!(vns.len(), 3);
}

#[test]
fn change_committee_bucket() {
    let db = create_db();
    let mut tx = db.create_transaction().unwrap();
    let mut validator_nodes = db.validator_nodes(&mut tx);
    let pk = new_public_key();
    insert_vn_with_public_key(&mut validator_nodes, pk.clone(), Epoch(0), Epoch(4), None);
    update_committee_bucket(&mut validator_nodes, &pk, Shard::from(1), Epoch(0));
    update_committee_bucket(&mut validator_nodes, &pk, Shard::from(3), Epoch(1));
    update_committee_bucket(&mut validator_nodes, &pk, Shard::from(7), Epoch(2));
    update_committee_bucket(&mut validator_nodes, &pk, Shard::from(4), Epoch(3));
    let vns = validator_nodes
        .get_committee_for_shard(Epoch(3), Shard::from(4))
        .unwrap().unwrap();
    assert_eq!(vns.len(), 1);
}

#[test]
fn insert_and_get_within_shard_range_duplicate_public_keys() {
    // // Testing fetching within a shard range. Specifically, the ability for Sqlite to compare blob columns
    // let db = create_db();
    // let mut tx = db.create_transaction().unwrap();
    // let mut validator_nodes = db.validator_nodes(&mut tx);
    // // Insert lower shard key
    // insert_vn_with_public_key(&mut validator_nodes, PublicKey::default(), Epoch(0), Epoch(1), None);
    //
    // let pk = new_public_key();
    // insert_vn_with_public_key(&mut validator_nodes, pk.clone(), Epoch(0), Epoch(1), None);
    // update_committee_bucket(&mut validator_nodes, &pk, Shard::from(0), Epoch(0));
    // update_committee_bucket(&mut validator_nodes, &pk, Shard::from(2), Epoch(2));
    // let pk2 = new_public_key();
    // insert_vn_with_public_key(&mut validator_nodes, pk2.clone(), Epoch(1), Epoch(2), None);
    // update_committee_bucket(&mut validator_nodes, &pk2, Shard::from(1), Epoch(1));
    // update_committee_bucket(&mut validator_nodes, &pk2, Shard::from(3), Epoch(3));
    //
    // tx.commit().unwrap();
    // let mut tx = db.create_transaction().unwrap();
    // let mut validator_nodes = db.validator_nodes(&mut tx);
    //
    // let shard_id = derived_substate_address(&pk);
    // let shard_id2 = derived_substate_address(&pk2);
    // let (start, end) = if shard_id > shard_id2 {
    //     (shard_id2, shard_id)
    // } else {
    //     (shard_id, shard_id2)
    // };
    //
    // let vns = validator_nodes
    //     .get_by_substate_range(Epoch(0),  None, start..=end)
    //     .unwrap();
    // if shard_id > shard_id2 {
    //     assert_eq!(vns[0].public_key, pk2);
    //     assert_eq!(vns[0].committee_shard, Some(Shard::from(3)));
    //     assert_eq!(vns[0].epoch, Epoch(3));
    //     assert_eq!(vns[1].public_key, pk);
    //     assert_eq!(vns[1].committee_shard, Some(Shard::from(2)));
    //     assert_eq!(vns[1].epoch, Epoch(2));
    // } else {
    //     assert_eq!(vns[0].public_key, pk);
    //     assert_eq!(vns[0].committee_shard, Some(Shard::from(2)));
    //     assert_eq!(vns[0].epoch, Epoch(2));
    //     assert_eq!(vns[1].public_key, pk2);
    //     assert_eq!(vns[1].committee_shard, Some(Shard::from(3)));
    //     assert_eq!(vns[1].epoch, Epoch(3));
    // }
    // assert_eq!(vns.len(), 2);
    //
    // let vn = validator_nodes
    //     .get_by_public_key(Epoch(0), Epoch(10), &pk, None)
    //     .unwrap();
    // assert_eq!(vn.epoch, Epoch(2));
    // assert_eq!(vn.committee_shard, Some(Shard::from(2)));
}
