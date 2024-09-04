//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::{rngs::OsRng, RngCore};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, Command, Decision, TransactionAtom, TransactionPoolStage, TransactionPoolStatusUpdate},
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::TransactionId;
use tari_utilities::epoch_time::EpochTime;

fn create_db() -> SqliteStateStore<String> {
    SqliteStateStore::connect(":memory:").unwrap()
}

fn create_tx_atom() -> TransactionAtom {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    TransactionAtom {
        id: TransactionId::new(bytes),
        decision: Decision::Commit,
        evidence: Default::default(),
        transaction_fee: 0,
        leader_fee: None,
    }
}

mod confirm_all_transitions {
    use tari_dan_common_types::{NumPreshards, ShardGroup};

    use super::*;

    #[test]
    fn it_sets_pending_stage_to_stage() {
        let db = create_db();
        // Need FK=off because otherwise we'd have to create transactions for each in the pool
        db.foreign_keys_off().unwrap();
        let mut tx = db.create_write_tx().unwrap();

        let atom1 = create_tx_atom();
        let atom2 = create_tx_atom();
        let atom3 = create_tx_atom();

        let network = Default::default();
        let zero_block = Block::zero_block(network, NumPreshards::P64);
        zero_block.insert(&mut tx).unwrap();
        let block1 = Block::new(
            network,
            *zero_block.id(),
            zero_block.justify().clone(),
            NodeHeight(1),
            Epoch(0),
            ShardGroup::all_shards(NumPreshards::P64),
            Default::default(),
            // Need to have a command in, otherwise this block will not be included internally in the query because it
            // cannot cause a state change without any commands
            [Command::Prepare(atom1.clone())].into_iter().collect(),
            Default::default(),
            Default::default(),
            Default::default(),
            None,
            EpochTime::now().as_u64(),
            0,
            FixedHash::zero(),
        );
        block1.insert(&mut tx).unwrap();

        tx.transaction_pool_insert_new(atom1.id, atom1.decision, true).unwrap();
        tx.transaction_pool_insert_new(atom2.id, atom2.decision, true).unwrap();
        tx.transaction_pool_insert_new(atom3.id, atom3.decision, true).unwrap();
        let block_id = *block1.id();

        let transactions = tx.transaction_pool_get_all().unwrap();
        let mut tx_1 = transactions
            .iter()
            .find(|tx| *tx.transaction_id() == atom1.id)
            .unwrap()
            .clone();
        let mut tx_2 = transactions
            .iter()
            .find(|tx| *tx.transaction_id() == atom2.id)
            .unwrap()
            .clone();
        let mut tx_3 = transactions
            .iter()
            .find(|tx| *tx.transaction_id() == atom3.id)
            .unwrap()
            .clone();

        tx_1.set_next_stage(TransactionPoolStage::Prepared, true).unwrap();
        tx_1.set_next_stage(TransactionPoolStage::LocalPrepared, false).unwrap();

        tx_2.set_next_stage(TransactionPoolStage::Prepared, true).unwrap();
        tx_3.set_next_stage(TransactionPoolStage::Prepared, true).unwrap();

        tx.transaction_pool_add_pending_update(&block_id, &TransactionPoolStatusUpdate { transaction: tx_1 })
            .unwrap();
        tx.transaction_pool_add_pending_update(&block_id, &TransactionPoolStatusUpdate { transaction: tx_2 })
            .unwrap();
        tx.transaction_pool_add_pending_update(&block_id, &TransactionPoolStatusUpdate { transaction: tx_3 })
            .unwrap();

        let rec = tx
            .transaction_pool_get_for_blocks(zero_block.id(), &block_id, &atom1.id)
            .unwrap();
        assert!(rec.committed_stage().is_new());
        assert!(rec.pending_stage().unwrap().is_local_prepared());

        let rec = tx
            .transaction_pool_get_for_blocks(zero_block.id(), &block_id, &atom2.id)
            .unwrap();
        assert!(rec.committed_stage().is_new());
        assert!(rec.pending_stage().unwrap().is_prepared());

        tx.transaction_pool_confirm_all_transitions(&block1.as_locked_block())
            .unwrap();

        let rec = tx
            .transaction_pool_get_for_blocks(zero_block.id(), &block_id, &atom1.id)
            .unwrap();
        assert!(rec.committed_stage().is_local_prepared());
        assert!(rec.pending_stage().is_none());

        let rec = tx
            .transaction_pool_get_for_blocks(zero_block.id(), &block_id, &atom2.id)
            .unwrap();
        assert_eq!(rec.committed_stage(), TransactionPoolStage::Prepared);
        assert_eq!(rec.pending_stage(), None);

        let rec = tx
            .transaction_pool_get_for_blocks(zero_block.id(), &block_id, &atom3.id)
            .unwrap();
        assert_eq!(rec.committed_stage(), TransactionPoolStage::Prepared);
        assert_eq!(rec.pending_stage(), None);

        tx.rollback().unwrap();
    }
}
