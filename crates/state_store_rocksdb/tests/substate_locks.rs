//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use std::collections::HashMap;

use indexmap::IndexMap;
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{SubstateLockType, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::SubstateLock,
};
use tari_ootle_transaction::TransactionId;

use crate::helpers::{
    commit_chain,
    create_chain,
    create_random_substate_id,
    create_rocksdb,
    substate_id_tx_seed,
    transaction_id_from_seed,
};

#[test]
fn rocksdb() {
    // env_logger::builder().filter_level(log::LevelFilter::Debug).init();
    let (db, _tmp) = create_rocksdb();
    run_test(db);
}

fn run_test(db: impl StateStore) {
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b7 = chain[7].as_leaf();
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    log::debug!("b7: {}, b8: {}, b9: {}", b7, b8, b9);

    let s1 = create_random_substate_id();
    let s = tx
        .substate_locks_get_latest_for_substate(&chain[0].as_leaf(), &s1)
        .optional()
        .unwrap();
    assert!(s.is_none());

    let tx_1 = transaction_id_from_seed(1);
    let tx_1_locks = gen_locks(tx_1, 5).collect::<IndexMap<_, _>>();
    let tx_2 = transaction_id_from_seed(2);
    let tx_2_locks = gen_locks(tx_2, 5).collect::<IndexMap<_, _>>();
    let tx_3 = transaction_id_from_seed(3);
    let tx_3_locks = gen_locks(tx_3, 5).collect::<IndexMap<_, _>>();
    let tx_4 = transaction_id_from_seed(4);
    let tx_4_locks = gen_locks(tx_4, 5).collect::<IndexMap<_, _>>();

    let mut locks_for_b8 = IndexMap::new();
    for (substate_id, lock) in tx_1_locks.iter().chain(tx_2_locks.iter()) {
        let v = locks_for_b8.entry(substate_id.clone()).or_insert_with(Vec::new);
        v.push(*lock);
    }

    tx.substate_locks_insert_all(&b8, &locks_for_b8).unwrap();
    let mut locks_for_b9 = IndexMap::new();
    for (substate_id, lock) in tx_3_locks.iter().chain(tx_4_locks.iter()) {
        let v = locks_for_b9.entry(substate_id.clone()).or_insert_with(Vec::new);
        v.push(*lock);
    }
    tx.substate_locks_insert_all(&b9, &locks_for_b9).unwrap();

    let mut all_locks = IndexMap::new();
    for (substate_id, lock) in tx_1_locks
        .iter()
        .chain(tx_2_locks.iter())
        .chain(tx_3_locks.iter())
        .chain(tx_4_locks.iter())
    {
        let v = all_locks.entry(substate_id.clone()).or_insert_with(Vec::new);
        v.push(*lock);
    }

    let mut tx_id_counts = HashMap::new();
    for locks in all_locks.values() {
        for lock in locks {
            let count = tx_id_counts.entry(lock.transaction_id()).or_insert(0usize);
            *count += 1;
        }
    }

    for (id, locks) in &all_locks {
        let s = tx.substate_locks_get_latest_for_substate(&b9, id).unwrap();
        let l = locks.last().unwrap();
        assert_eq!(s.lock_type(), l.lock_type());
        assert_eq!(s.version(), l.version());

        let locked_by_tx = tx
            .substate_locks_get_locked_substates_for_transaction(l.transaction_id())
            .unwrap();
        assert_eq!(locked_by_tx.len(), *tx_id_counts.get(l.transaction_id()).unwrap());
    }
    for id in locks_for_b9.keys() {
        let s = tx.substate_locks_get_latest_for_substate(&b8, id).optional().unwrap();
        assert!(s.is_none());
    }

    tx.substate_locks_remove_many_for_transactions(Some(&tx_1)).unwrap();
    let locked_by_tx = tx.substate_locks_get_locked_substates_for_transaction(&tx_1).unwrap();
    assert_eq!(locked_by_tx.len(), 0);

    tx.substate_locks_remove_any_by_block_id(b9.block_id()).unwrap();
    let locked_by_tx = tx.substate_locks_get_locked_substates_for_transaction(&tx_3).unwrap();
    assert_eq!(locked_by_tx.len(), 0);
    let locked_by_tx = tx.substate_locks_get_locked_substates_for_transaction(&tx_4).unwrap();
    assert_eq!(locked_by_tx.len(), 0);

    tx.rollback().unwrap();
}

fn gen_locks(transaction_id: TransactionId, num: usize) -> impl Iterator<Item = (SubstateId, SubstateLock)> {
    (0..num as u64).map(move |i| {
        let id = substate_id_tx_seed(transaction_id, i as u32);
        let lock = SubstateLock::new(transaction_id, i, SubstateLockType::Write, false);
        (id, lock)
    })
}
