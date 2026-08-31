//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{create_block, create_rocksdb, create_tx_atom};
use tari_common_types::types::FixedHash;
use tari_consensus_types::ShardGroupAccumulatedData;
use tari_ootle_common_types::{Epoch, ExtraData, NodeHeight, ProtocolVersion};
use tari_ootle_storage::{
    StateStore,
    StateStoreWriteTransaction,
    consensus_models::{Block, Command},
};
use tari_ootle_transaction::Network;
use tari_template_lib_types::crypto::SchnorrSignatureBytes;
use tari_utilities::epoch_time::EpochTime;

#[test]
fn missing_transactions_rocksdb() {
    let (db, _tmp) = create_rocksdb();
    missing_transactions_operations(db);
}

fn missing_transactions_operations(db: impl StateStore) {
    let mut tx = db.create_write_tx().unwrap();

    let network = Network::LocalNet;

    // add some blocks to the database
    let genesis = create_block(None);
    genesis.insert(&mut tx).unwrap();

    let atom1 = create_tx_atom();
    let atom2 = create_tx_atom();
    let block1 = Block::create(
        network,
        ProtocolVersion::V0,
        *genesis.id(),
        genesis.justify().clone(),
        None,
        NodeHeight(1),
        Epoch(0),
        genesis.shard_group(),
        Default::default(),
        // Need to have a command in, otherwise this block will not be included internally in the query because it
        // cannot cause a state change without any commands
        [Command::LocalPrepare(atom1.clone())].into_iter().collect(),
        Default::default(),
        Default::default(),
        SchnorrSignatureBytes::zero(),
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::default(),
    )
    .unwrap();

    // missing_transactions_insert
    let missing_transaction_ids = vec![&atom1.id, &atom2.id];
    tx.parked_block_insert(&block1, &[], missing_transaction_ids).unwrap();

    // The block stays parked while it is still waiting on atom2
    let unparked = tx
        .parked_block_remove_missing_transaction(block1.height(), atom1.id())
        .unwrap();
    assert!(unparked.is_none());

    // The last missing transaction unparks the block
    let (unparked, foreign_proposals) = tx
        .parked_block_remove_missing_transaction(block1.height(), atom2.id())
        .unwrap()
        .expect("block should be unparked once no transactions are missing");
    assert_eq!(unparked.id(), block1.id());
    assert!(foreign_proposals.is_empty());

    tx.rollback().unwrap();
}
