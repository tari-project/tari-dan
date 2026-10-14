//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{create_block, create_rocksdb, create_tx_atom};
use tari_common_types::types::FixedHash;
use tari_consensus_types::ShardGroupAccumulatedData;
use tari_ootle_common_types::{Epoch, ExtraData, NodeHeight, ProtocolVersion};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{Block, BlockCommands, Command},
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
        BlockCommands::init([Command::LocalPrepare(atom1.clone())]).unwrap(),
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

fn parked_block_at(parent: &Block, epoch: Epoch, height: NodeHeight, command: Command) -> Block {
    Block::create(
        Network::LocalNet,
        ProtocolVersion::V0,
        *parent.id(),
        parent.justify().clone(),
        None,
        height,
        epoch,
        parent.shard_group(),
        Default::default(),
        [command].into_iter().collect(),
        Default::default(),
        Default::default(),
        SchnorrSignatureBytes::zero(),
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::default(),
    )
    .unwrap()
}

#[test]
fn parked_blocks_are_removed_up_to_a_height() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let genesis = create_block(None);
    genesis.insert(&mut tx).unwrap();

    let atoms = (0..4).map(|_| create_tx_atom()).collect::<Vec<_>>();
    let blocks = atoms
        .iter()
        .zip(1..)
        .map(|(atom, height)| {
            let block = parked_block_at(
                &genesis,
                Epoch(0),
                NodeHeight(height),
                Command::LocalPrepare(atom.clone()),
            );
            tx.parked_block_insert(&block, &[], [&atom.id]).unwrap();
            block
        })
        .collect::<Vec<_>>();
    let proposer = *blocks[0].proposed_by();
    assert_eq!(
        tx.parked_blocks_count_by_proposer(Epoch(0), NodeHeight(1), &proposer)
            .unwrap(),
        1
    );

    assert_eq!(tx.parked_blocks_remove_up_to(Epoch(0), NodeHeight(2)).unwrap(), 2);
    for block in &blocks[..2] {
        assert!(!tx.parked_block_exists(block.id()).unwrap());
        assert_eq!(
            tx.parked_blocks_count_by_proposer(Epoch(0), block.height(), &proposer)
                .unwrap(),
            0
        );
    }
    // Their missing-transaction records went with them, so the transactions arriving unpark nothing
    for atom in &atoms[..2] {
        assert!(
            tx.parked_block_remove_missing_transaction(NodeHeight(0), atom.id())
                .unwrap()
                .is_none()
        );
    }

    // Unparking a block releases its proposer's slot at that height
    let (unparked, _) = tx
        .parked_block_remove_missing_transaction(NodeHeight(0), atoms[2].id())
        .unwrap()
        .expect("block should be unparked once no transactions are missing");
    assert_eq!(unparked.id(), blocks[2].id());
    assert_eq!(
        tx.parked_blocks_count_by_proposer(Epoch(0), NodeHeight(3), &proposer)
            .unwrap(),
        0
    );

    // Every parked block of an earlier epoch goes, whatever its height
    assert!(tx.parked_block_exists(blocks[3].id()).unwrap());
    assert_eq!(tx.parked_blocks_remove_up_to(Epoch(1), NodeHeight(0)).unwrap(), 1);
    assert!(!tx.parked_block_exists(blocks[3].id()).unwrap());

    tx.rollback().unwrap();
}
