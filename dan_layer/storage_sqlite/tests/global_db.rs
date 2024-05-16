//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

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
        insert_vn_with_public_key(
            validator_nodes,
            new_public_key(),
            epoch,
            epoch + Epoch(1),
            sidechain_id.clone(),
        )
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
    let vns = validator_nodes.get_all_within_epoch(Epoch(0), None).unwrap();
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
        .unwrap()
        .unwrap();
    assert_eq!(vns.len(), 1);
}
