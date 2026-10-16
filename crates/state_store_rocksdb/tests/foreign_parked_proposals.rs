//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{create_foreign_proposal, create_rocksdb, transaction_id_from_seed};
use tari_consensus_types::BlockId;
use tari_ootle_common_types::Epoch;
use tari_ootle_storage::{
    StateStore,
    StateStoreWriteTransaction,
    consensus_models::{Block, BookkeepingModel, ForeignParkedProposal},
};
use tari_ootle_transaction::{Network, TransactionId};

use crate::helpers::num_preshards;

#[test]
fn rocksdb() {
    let (db, _tmp) = create_rocksdb();
    run_test(db);
}

fn run_test(db: impl StateStore) {
    let mut tx = db.create_write_tx().unwrap();

    let zero_block = Block::zero_block(Network::LocalNet, num_preshards());
    tx.blocks_insert(&zero_block).unwrap();
    zero_block.as_locked().set(&mut tx).unwrap();

    let fp = create_foreign_proposal(*zero_block.id(), Epoch(1));

    let tx_1 = transaction_id_from_seed(1);
    let tx_2 = transaction_id_from_seed(2);
    let tx_3 = transaction_id_from_seed(3);

    // Doesnt exist - no error
    let blocks = tx.foreign_parked_blocks_remove_all_by_transaction(&tx_1).unwrap();
    assert!(blocks.is_empty());

    let (commit_proof, block_pledge, block_metadata) = fp.into_proposal().into_parts();
    let parked = ForeignParkedProposal::new(commit_proof, block_pledge, block_metadata);
    parked.insert(&mut tx).unwrap();
    parked.add_missing_transactions(&mut tx, &[tx_1, tx_2, tx_3]).unwrap();

    let blocks = tx.foreign_parked_blocks_remove_all_by_transaction(&tx_1).unwrap();
    assert!(blocks.is_empty());

    let blocks = tx.foreign_parked_blocks_remove_all_by_transaction(&tx_2).unwrap();
    assert!(blocks.is_empty());

    let blocks = tx.foreign_parked_blocks_remove_all_by_transaction(&tx_3).unwrap();
    assert_eq!(blocks.len(), 1);

    tx.rollback().unwrap();
}

/// A parked block is only released once every one of its missing transactions has arrived, however many other parked
/// blocks are waiting on the same transactions.
#[test]
fn it_unparks_each_block_once_its_last_transaction_arrives() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let zero_block = Block::zero_block(Network::LocalNet, num_preshards());
    tx.blocks_insert(&zero_block).unwrap();
    zero_block.as_locked().set(&mut tx).unwrap();

    let tx_1 = transaction_id_from_seed(1);
    let tx_2 = transaction_id_from_seed(2);

    let waits_on_both = park(&mut tx, &zero_block, Epoch(1), &[tx_1, tx_2]);
    let waits_on_one = park(&mut tx, &zero_block, Epoch(2), &[tx_1]);

    let blocks = tx.foreign_parked_blocks_remove_all_by_transaction(&tx_1).unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(*blocks[0].block_id(), waits_on_one);

    let blocks = tx.foreign_parked_blocks_remove_all_by_transaction(&tx_2).unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(*blocks[0].block_id(), waits_on_both);

    tx.rollback().unwrap();
}

fn park<TTx: StateStoreWriteTransaction>(
    tx: &mut TTx,
    parent: &Block,
    epoch: Epoch,
    missing: &[TransactionId],
) -> BlockId {
    let (commit_proof, block_pledge, block_metadata) = create_foreign_proposal(*parent.id(), epoch)
        .into_proposal()
        .into_parts();
    let parked = ForeignParkedProposal::new(commit_proof, block_pledge, block_metadata);
    parked.insert(tx).unwrap();
    parked.add_missing_transactions(tx, missing).unwrap();
    *parked.block_id()
}
