//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use std::collections::HashMap;

use indexmap::IndexMap;
use tari_consensus_types::{Decision, LeafBlock};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{Epoch, SubstateLockType, SubstateVersion, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{Evidence, SubstateLock, TransactionPoolRecord},
};
use tari_ootle_transaction::TransactionId;
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};

use crate::helpers::{
    chain_across_an_epoch_change,
    commit_chain,
    create_block_with_qc,
    create_chain,
    create_random_substate_id,
    create_rocksdb,
    substate_id_seed,
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
            .substate_locks_get_locked_substates_for_transaction(&b9, l.transaction_id())
            .unwrap();
        assert_eq!(locked_by_tx.len(), *tx_id_counts.get(l.transaction_id()).unwrap());
    }
    for id in locks_for_b9.keys() {
        let s = tx.substate_locks_get_latest_for_substate(&b8, id).optional().unwrap();
        assert!(s.is_none());
    }

    tx.substate_locks_remove_many_for_transactions(Some(&tx_1)).unwrap();
    let locked_by_tx = tx
        .substate_locks_get_locked_substates_for_transaction(&b9, &tx_1)
        .unwrap();
    assert_eq!(locked_by_tx.len(), 0);

    tx.substate_locks_remove_any_by_block_id(b9.block_id()).unwrap();
    let locked_by_tx = tx
        .substate_locks_get_locked_substates_for_transaction(&b9, &tx_3)
        .unwrap();
    assert_eq!(locked_by_tx.len(), 0);
    let locked_by_tx = tx
        .substate_locks_get_locked_substates_for_transaction(&b9, &tx_4)
        .unwrap();
    assert_eq!(locked_by_tx.len(), 0);

    tx.rollback().unwrap();
}

fn gen_locks(transaction_id: TransactionId, num: usize) -> impl Iterator<Item = (SubstateId, SubstateLock)> {
    (0..num as u64).map(move |i| {
        let id = substate_id_tx_seed(transaction_id, i as u32);
        let lock = SubstateLock::new(transaction_id, SubstateVersion::new(i), SubstateLockType::Write, false);
        (id, lock)
    })
}

/// A block grants several locks on one substate in command order, and the chain's answer is the last of them.
///
/// The lock granted first also sorts first by transaction id, so the last-granted lock is not the one key order reaches
/// first and the assertion can tell the two apart. The sibling branch is written between the two reads: the answer
/// belongs to b9's chain, so neither read may move it.
#[test]
fn the_latest_lock_does_not_depend_on_other_branches() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();

    tx.substate_locks_insert_all(&b8, &two_locks(&substate_id, granted_first, granted_last))
        .unwrap();

    let lock = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
    assert_eq!(lock.transaction_id(), &granted_last);
    assert_eq!(lock.lock_type(), SubstateLockType::Output);

    // A sibling of b9 locks the same substate. It is not on b9's chain, so b9's answer must not move.
    let fork = create_block_with_qc(&b8);
    tx.proposal_certificates_save(fork.justify()).unwrap();
    fork.insert(&mut tx).unwrap();
    let fork_locks = IndexMap::from([(substate_id.clone(), vec![SubstateLock::new(
        transaction_id_from_seed(9),
        SubstateVersion::new(1),
        SubstateLockType::Output,
        true,
    )])]);
    tx.substate_locks_insert_all(&fork.as_leaf(), &fork_locks).unwrap();

    let lock = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
    assert_eq!(
        lock.transaction_id(),
        &granted_last,
        "writing a sibling branch changed the answer for b9"
    );
    assert_eq!(lock.lock_type(), SubstateLockType::Output);

    tx.rollback().unwrap();
}

/// A lock a committed block granted is ordered the same way as a pending one, so the answer is still the last granted.
#[test]
fn the_latest_committed_lock_is_the_one_granted_last() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    // commit_chain commits every block below the last three, so this block is beneath the commit block and no longer in
    // the pending chain.
    let committed = chain[6].as_leaf();
    let b9 = chain[9].as_leaf();

    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();

    tx.substate_locks_insert_all(&committed, &two_locks(&substate_id, granted_first, granted_last))
        .unwrap();

    let lock = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
    assert_eq!(lock.transaction_id(), &granted_last);
    assert_eq!(lock.lock_type(), SubstateLockType::Output);

    tx.rollback().unwrap();
}

/// A branch the leaf does not extend holds no locks as far as that leaf is concerned.
#[test]
fn a_lock_from_another_branch_is_not_found() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    let fork = create_block_with_qc(&b8);
    tx.proposal_certificates_save(fork.justify()).unwrap();
    fork.insert(&mut tx).unwrap();

    let substate_id = create_random_substate_id();
    let locks = IndexMap::from([(substate_id.clone(), vec![SubstateLock::new(
        transaction_id_from_seed(1),
        SubstateVersion::new(0),
        SubstateLockType::Write,
        false,
    )])]);
    tx.substate_locks_insert_all(&fork.as_leaf(), &locks).unwrap();

    let lock = tx
        .substate_locks_get_latest_for_substate(&b9, &substate_id)
        .optional()
        .unwrap();
    assert!(lock.is_none());

    // The branch that granted it sees it
    let lock = tx
        .substate_locks_get_latest_for_substate(&fork.as_leaf(), &substate_id)
        .unwrap();
    assert_eq!(lock.transaction_id(), &transaction_id_from_seed(1));

    tx.rollback().unwrap();
}

/// Releasing one transaction's lock must leave the other locks on the substate reachable.
#[test]
fn releasing_one_transactions_lock_leaves_the_rest() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();

    tx.substate_locks_insert_all(&b8, &two_locks(&substate_id, granted_first, granted_last))
        .unwrap();

    tx.substate_locks_remove_many_for_transactions(Some(&granted_last))
        .unwrap();

    let lock = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
    assert_eq!(lock.transaction_id(), &granted_first);
    assert_eq!(lock.lock_type(), SubstateLockType::Read);

    tx.substate_locks_remove_many_for_transactions(Some(&granted_first))
        .unwrap();

    let lock = tx
        .substate_locks_get_latest_for_substate(&b9, &substate_id)
        .optional()
        .unwrap();
    assert!(lock.is_none());

    tx.rollback().unwrap();
}

/// Removing a block releases every lock it granted.
#[test]
fn removing_a_block_releases_every_lock_it_granted() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();

    tx.substate_locks_insert_all(&b8, &two_locks(&substate_id, granted_first, granted_last))
        .unwrap();
    tx.substate_locks_remove_any_by_block_id(b8.block_id()).unwrap();

    let lock = tx
        .substate_locks_get_latest_for_substate(&b9, &substate_id)
        .optional()
        .unwrap();
    assert!(lock.is_none());

    tx.rollback().unwrap();
}

/// A substate lock belongs to a pooled transaction, so removing a transaction from the pool releases its locks and
/// leaves the locks of transactions still in the pool.
#[test]
fn removing_a_transaction_from_the_pool_releases_its_locks() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();
    for id in [granted_first, granted_last] {
        tx.transaction_pool_insert_new(id, Decision::Commit, &Evidence::empty(), true, false, Epoch(1), 0)
            .unwrap();
    }
    tx.substate_locks_insert_all(&b8, &two_locks(&substate_id, granted_first, granted_last))
        .unwrap();

    let removed = TransactionPoolRecord::remove_all(&mut tx, [&granted_last]).unwrap();
    assert_eq!(removed.len(), 1);

    let lock = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
    assert_eq!(lock.transaction_id(), &granted_first);
    let locked_by_removed = tx
        .substate_locks_get_locked_substates_for_transaction(&b9, &granted_last)
        .unwrap();
    assert!(locked_by_removed.is_empty());

    TransactionPoolRecord::remove_all(&mut tx, [&granted_first]).unwrap();

    let lock = tx
        .substate_locks_get_latest_for_substate(&b9, &substate_id)
        .optional()
        .unwrap();
    assert!(lock.is_none());

    tx.rollback().unwrap();
}

/// Two transaction ids, the lower first. Granting in this order makes transaction-id order agree with grant order, so
/// the last-granted lock is the last in key order rather than the first.
fn ordered_transaction_ids() -> (TransactionId, TransactionId) {
    let mut ids = [transaction_id_from_seed(1), transaction_id_from_seed(2)];
    ids.sort();
    (ids[0], ids[1])
}

/// A READ lock followed by an OUTPUT lock on one substate, as local-only rules allow within one block.
fn two_locks(
    substate_id: &SubstateId,
    granted_first: TransactionId,
    granted_last: TransactionId,
) -> IndexMap<SubstateId, Vec<SubstateLock>> {
    IndexMap::from([(substate_id.clone(), vec![
        SubstateLock::new(granted_first, SubstateVersion::new(0), SubstateLockType::Read, true),
        SubstateLock::new(granted_last, SubstateVersion::new(1), SubstateLockType::Output, true),
    ])])
}

/// A branch block below the commit height holds no locks for the chain that committed that height.
///
/// Height alone does not place a lock on the committed chain: a branch block can sit below the commit block. It is
/// still in the pending chain, which is what tells the two apart.
#[test]
fn a_lock_from_a_branch_below_the_commit_height_is_not_found() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b9 = chain[9].as_leaf();

    // A sibling of chain[6], so it sits below the commit block that commit_chain leaves at chain[7]
    let branch = create_block_with_qc(&chain[5].as_leaf());
    tx.proposal_certificates_save(branch.justify()).unwrap();
    branch.insert(&mut tx).unwrap();
    assert!(branch.height() < chain[7].height());

    let substate_id = create_random_substate_id();
    let locks = IndexMap::from([(substate_id.clone(), vec![SubstateLock::new(
        transaction_id_from_seed(1),
        SubstateVersion::new(0),
        SubstateLockType::Write,
        false,
    )])]);
    tx.substate_locks_insert_all(&branch.as_leaf(), &locks).unwrap();

    let lock = tx
        .substate_locks_get_latest_for_substate(&b9, &substate_id)
        .optional()
        .unwrap();
    assert!(lock.is_none());

    tx.rollback().unwrap();
}

/// Releasing locks leaves none behind, by block or by transaction, over enough substates that a lock the lock table
/// indexes under the wrong key would be left reachable.
#[test]
fn releasing_many_locks_leaves_none_behind() {
    let (db, _tmp) = create_rocksdb();

    for by_block in [true, false] {
        let mut tx = db.create_write_tx().unwrap();
        let chain = create_chain(10);
        commit_chain(&mut tx, &chain);
        let b8 = chain[8].as_leaf();
        let b9 = chain[9].as_leaf();

        let (granted_first, granted_last) = ordered_transaction_ids();
        let substate_ids = (0..50).map(substate_id_seed).collect::<Vec<_>>();

        let mut locks = IndexMap::new();
        for id in &substate_ids {
            locks.insert(id.clone(), vec![
                SubstateLock::new(granted_first, SubstateVersion::new(0), SubstateLockType::Read, true),
                SubstateLock::new(granted_last, SubstateVersion::new(1), SubstateLockType::Output, true),
            ]);
        }
        tx.substate_locks_insert_all(&b8, &locks).unwrap();

        for id in &substate_ids {
            assert!(tx.substate_locks_get_latest_for_substate(&b9, id).is_ok());
        }

        if by_block {
            tx.substate_locks_remove_any_by_block_id(b8.block_id()).unwrap();
        } else {
            tx.substate_locks_remove_many_for_transactions([&granted_first, &granted_last])
                .unwrap();
        }

        for id in &substate_ids {
            let lock = tx.substate_locks_get_latest_for_substate(&b9, id).optional().unwrap();
            assert!(lock.is_none(), "lock left behind for {id} (by_block={by_block})");
        }
        for tx_id in [&granted_first, &granted_last] {
            let locked = tx
                .substate_locks_get_locked_substates_for_transaction(&b9, tx_id)
                .unwrap();
            assert!(locked.is_empty(), "{tx_id} still holds locks (by_block={by_block})");
        }

        tx.rollback().unwrap();
    }
}

/// A transaction's locks are read per chain: a branch we are not extending has not locked anything for us.
///
/// This read backs the local pledges a transaction ships to foreign committees. Pledging a lock another branch granted
/// would offer a foreign shard group a value this chain never locked.
#[test]
fn a_transactions_locks_are_scoped_to_one_chain() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    let branch = create_block_with_qc(&b8);
    tx.proposal_certificates_save(branch.justify()).unwrap();
    branch.insert(&mut tx).unwrap();

    let tx_id = transaction_id_from_seed(1);
    let on_b9 = create_random_substate_id();
    let on_branch = create_random_substate_id();

    let lock = |id: &SubstateId| {
        IndexMap::from([(id.clone(), vec![SubstateLock::new(
            tx_id,
            SubstateVersion::new(0),
            SubstateLockType::Write,
            false,
        )])])
    };
    tx.substate_locks_insert_all(&b9, &lock(&on_b9)).unwrap();
    tx.substate_locks_insert_all(&branch.as_leaf(), &lock(&on_branch))
        .unwrap();

    let from_b9 = tx
        .substate_locks_get_locked_substates_for_transaction(&b9, &tx_id)
        .unwrap();
    assert_eq!(from_b9.len(), 1);
    assert_eq!(from_b9[0].substate_id, on_b9);

    let from_branch = tx
        .substate_locks_get_locked_substates_for_transaction(&branch.as_leaf(), &tx_id)
        .unwrap();
    assert_eq!(from_branch.len(), 1);
    assert_eq!(from_branch[0].substate_id, on_branch);

    tx.rollback().unwrap();
}

/// A write lock another branch granted is not a conflict for the chain asking.
#[test]
fn a_conflicting_write_lock_is_scoped_to_one_chain() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    let branch = create_block_with_qc(&b8);
    tx.proposal_certificates_save(branch.justify()).unwrap();
    branch.insert(&mut tx).unwrap();

    let branch_tx = transaction_id_from_seed(1);
    let asking_tx = transaction_id_from_seed(2);
    let substate_id = create_random_substate_id();

    tx.substate_locks_insert_all(
        &branch.as_leaf(),
        &IndexMap::from([(substate_id.clone(), vec![SubstateLock::new(
            branch_tx,
            SubstateVersion::new(0),
            SubstateLockType::Write,
            false,
        )])]),
    )
    .unwrap();

    let conflict = tx
        .substate_locks_has_any_write_locks_for_substates(&b9, Some(&asking_tx), Some(&substate_id))
        .unwrap();
    assert!(conflict.is_none(), "a branch's write lock conflicted for b9");

    let conflict = tx
        .substate_locks_has_any_write_locks_for_substates(&branch.as_leaf(), Some(&asking_tx), Some(&substate_id))
        .unwrap();
    assert_eq!(conflict, Some(branch_tx));

    tx.rollback().unwrap();
}

/// A transaction that mutates a substate locks its input version for write and its next version as an output, both in
/// the block that prepares it. Both locks are held: the write lock backs the pledge sent to foreign committees, and the
/// output lock backs the substate's next version.
#[test]
fn a_transaction_keeps_both_its_input_and_output_lock_on_one_substate() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = create_chain(10);
    commit_chain(&mut tx, &chain);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();

    let tx_id = transaction_id_from_seed(1);
    let substate_id = create_random_substate_id();
    tx.substate_locks_insert_all(
        &b8,
        &IndexMap::from([(substate_id.clone(), vec![
            SubstateLock::new(tx_id, SubstateVersion::new(0), SubstateLockType::Write, false),
            SubstateLock::new(tx_id, SubstateVersion::new(1), SubstateLockType::Output, false),
        ])]),
    )
    .unwrap();

    let mut locked = tx
        .substate_locks_get_locked_substates_for_transaction(&b9, &tx_id)
        .unwrap()
        .into_iter()
        .map(|l| (l.substate_id, l.lock.lock_type(), l.lock.version()))
        .collect::<Vec<_>>();
    locked.sort_by_key(|(_, _, v)| *v);
    assert_eq!(locked, vec![
        (substate_id.clone(), SubstateLockType::Write, SubstateVersion::new(0)),
        (substate_id.clone(), SubstateLockType::Output, SubstateVersion::new(1)),
    ]);

    let latest = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
    assert_eq!(latest.lock_type(), SubstateLockType::Output);

    let conflict = tx
        .substate_locks_has_any_write_locks_for_substates(&b9, None, Some(&substate_id))
        .unwrap();
    assert_eq!(conflict, Some(tx_id));
    tx.rollback().unwrap();

    for by_block in [true, false] {
        let mut tx = db.create_write_tx().unwrap();
        commit_chain(&mut tx, &chain);
        tx.substate_locks_insert_all(
            &b8,
            &IndexMap::from([(substate_id.clone(), vec![
                SubstateLock::new(tx_id, SubstateVersion::new(0), SubstateLockType::Write, false),
                SubstateLock::new(tx_id, SubstateVersion::new(1), SubstateLockType::Output, false),
            ])]),
        )
        .unwrap();
        if by_block {
            tx.substate_locks_remove_any_by_block_id(b8.block_id()).unwrap();
        } else {
            tx.substate_locks_remove_many_for_transactions(Some(&tx_id)).unwrap();
        }
        assert!(
            tx.substate_locks_get_latest_for_substate(&b9, &substate_id)
                .optional()
                .unwrap()
                .is_none(),
            "a lock was left behind (by_block={by_block})"
        );
        assert!(
            tx.substate_locks_has_any_write_locks_for_substates(&b9, None, Some(&substate_id))
                .unwrap()
                .is_none(),
            "a write lock was left behind (by_block={by_block})"
        );
        tx.rollback().unwrap();
    }
}

/// A lock the previous epoch committed is on the chain, however high its block sits relative to this epoch's commit
/// block.
#[test]
fn a_lock_committed_in_the_previous_epoch_is_found() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = chain_across_an_epoch_change(10, 5);
    commit_chain(&mut tx, &chain);
    let prev_epoch_block = chain[8].as_leaf();
    let leaf = chain.last().unwrap().as_leaf();
    assert!(prev_epoch_block.epoch() < leaf.epoch());
    let commit_block = &chain[chain.len() - 4];
    assert!(prev_epoch_block.height() > commit_block.height());

    let substate_id = create_random_substate_id();
    let transaction_id = transaction_id_from_seed(1);
    let locks = IndexMap::from([(substate_id.clone(), vec![SubstateLock::new(
        transaction_id,
        SubstateVersion::new(0),
        SubstateLockType::Write,
        false,
    )])]);
    tx.substate_locks_insert_all(&prev_epoch_block, &locks).unwrap();

    let locked = tx
        .substate_locks_get_locked_substates_for_transaction(&leaf, &transaction_id)
        .unwrap();
    assert_eq!(
        locked.len(),
        1,
        "the previous epoch's lock is missing from the transaction's locks"
    );

    let lock = tx
        .substate_locks_get_latest_for_substate(&leaf, &substate_id)
        .optional()
        .unwrap();
    assert_eq!(lock.map(|l| *l.transaction_id()), Some(transaction_id));

    let conflict = tx
        .substate_locks_has_any_write_locks_for_substates(&leaf, None, [&substate_id])
        .unwrap();
    assert_eq!(conflict, Some(transaction_id));

    tx.rollback().unwrap();
}

/// A lock granted in this epoch is more recent than any the previous epoch granted, whatever the two blocks' heights.
#[test]
fn the_latest_lock_across_an_epoch_change_is_this_epochs() {
    let (db, _tmp) = create_rocksdb();
    let mut tx = db.create_write_tx().unwrap();

    let chain = chain_across_an_epoch_change(10, 5);
    commit_chain(&mut tx, &chain);
    let prev_epoch_block = chain[8].as_leaf();
    let this_epoch_block = chain.last().unwrap().as_leaf();
    assert!(prev_epoch_block.height() > this_epoch_block.height());

    let substate_id = create_random_substate_id();
    let insert_read_lock = |tx: &mut _, block: &LeafBlock, seed: u32| {
        let locks = IndexMap::from([(substate_id.clone(), vec![SubstateLock::new(
            transaction_id_from_seed(seed),
            SubstateVersion::new(0),
            SubstateLockType::Read,
            false,
        )])]);
        StateStoreWriteTransaction::substate_locks_insert_all(tx, block, &locks).unwrap();
    };
    insert_read_lock(&mut tx, &prev_epoch_block, 1);
    insert_read_lock(&mut tx, &this_epoch_block, 2);

    let lock = tx
        .substate_locks_get_latest_for_substate(&this_epoch_block, &substate_id)
        .unwrap();
    assert_eq!(lock.transaction_id(), &transaction_id_from_seed(2));

    tx.rollback().unwrap();
}

/// Locks a committed transaction granted survive reopening the store, and so do partial releases: a release is
/// persisted by rewriting the lock-set record it shrinks.
#[test]
fn locks_and_releases_survive_reopening_the_store() {
    let (db, tmp) = create_rocksdb();
    let chain = create_chain(10);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();
    let substate_id = create_random_substate_id();
    let other_substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();

    let mut locks = two_locks(&substate_id, granted_first, granted_last);
    locks.insert(other_substate_id.clone(), vec![SubstateLock::new(
        granted_first,
        SubstateVersion::new(0),
        SubstateLockType::Write,
        false,
    )]);
    db.with_write_tx(|tx| {
        commit_chain(tx, &chain);
        tx.substate_locks_insert_all(&b8, &locks)
    })
    .unwrap();

    let db = reopen(db, &tmp);
    {
        let tx = db.create_read_tx().unwrap();
        let lock = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
        assert_eq!(lock.transaction_id(), &granted_last);
        assert_eq!(
            tx.substate_locks_get_locked_substates_for_transaction(&b9, &granted_first)
                .unwrap()
                .len(),
            2
        );
    }

    db.with_write_tx(|tx| tx.substate_locks_remove_many_for_transactions(Some(&granted_last)))
        .unwrap();

    let db = reopen(db, &tmp);
    {
        let tx = db.create_read_tx().unwrap();
        let lock = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
        assert_eq!(
            lock.transaction_id(),
            &granted_first,
            "a released lock came back after reopening"
        );
        assert!(
            tx.substate_locks_get_locked_substates_for_transaction(&b9, &granted_last)
                .unwrap()
                .is_empty()
        );
    }

    db.with_write_tx(|tx| tx.substate_locks_remove_many_for_transactions(Some(&granted_first)))
        .unwrap();

    let db = reopen(db, &tmp);
    let tx = db.create_read_tx().unwrap();
    for id in [&substate_id, &other_substate_id] {
        let lock = tx.substate_locks_get_latest_for_substate(&b9, id).optional().unwrap();
        assert!(lock.is_none(), "a released lock came back after reopening");
    }
}

/// A rolled-back transaction's locks are never seen by later transactions.
#[test]
fn a_rolled_back_transactions_locks_are_discarded() {
    let (db, _tmp) = create_rocksdb();
    let chain = create_chain(10);
    db.with_write_tx(|tx| {
        commit_chain(tx, &chain);
        Ok::<_, StorageError>(())
    })
    .unwrap();
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();
    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();

    let mut tx = db.create_write_tx().unwrap();
    tx.substate_locks_insert_all(&b8, &two_locks(&substate_id, granted_first, granted_last))
        .unwrap();
    assert!(tx.substate_locks_get_latest_for_substate(&b9, &substate_id).is_ok());
    tx.rollback().unwrap();

    let tx = db.create_read_tx().unwrap();
    let lock = tx
        .substate_locks_get_latest_for_substate(&b9, &substate_id)
        .optional()
        .unwrap();
    assert!(lock.is_none());
}

/// A read view sees the locks committed when it was opened, and none committed after.
#[test]
fn a_read_view_sees_the_locks_of_its_snapshot() {
    let (db, _tmp) = create_rocksdb();
    let chain = create_chain(10);
    db.with_write_tx(|tx| {
        commit_chain(tx, &chain);
        Ok::<_, StorageError>(())
    })
    .unwrap();
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();
    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();

    let before = db.create_read_tx().unwrap();
    db.with_write_tx(|tx| tx.substate_locks_insert_all(&b8, &two_locks(&substate_id, granted_first, granted_last)))
        .unwrap();
    let after = db.create_read_tx().unwrap();
    db.with_write_tx(|tx| tx.substate_locks_remove_many_for_transactions([&granted_first, &granted_last]))
        .unwrap();

    assert!(
        before
            .substate_locks_get_latest_for_substate(&b9, &substate_id)
            .optional()
            .unwrap()
            .is_none(),
        "a read view saw a lock committed after it was opened"
    );
    assert_eq!(
        after
            .substate_locks_get_latest_for_substate(&b9, &substate_id)
            .unwrap()
            .transaction_id(),
        &granted_last,
        "a read view lost a lock released after it was opened"
    );
}

/// Of two write transactions that change locks concurrently, the second to commit is refused and changes nothing,
/// in memory or on disk: any lock-set record it wrote was computed from a table the first has since changed.
#[test]
fn a_write_transaction_that_staged_from_an_outdated_table_is_not_committed() {
    let (db, tmp) = create_rocksdb();
    let chain = create_chain(10);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();
    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();
    db.with_write_tx(|tx| {
        commit_chain(tx, &chain);
        tx.substate_locks_insert_all(&b8, &two_locks(&substate_id, granted_first, granted_last))
    })
    .unwrap();

    let mut first = db.create_write_tx().unwrap();
    let mut second = db.create_write_tx().unwrap();
    first
        .substate_locks_remove_many_for_transactions(Some(&granted_first))
        .unwrap();
    let refused_substate = create_random_substate_id();
    second
        .substate_locks_insert_all(&b9, &two_locks(&refused_substate, granted_first, granted_last))
        .unwrap();
    first.commit().unwrap();
    second.commit().unwrap_err();
    drop((first, second));

    let check = |db: &RocksDbStateStore<String>| {
        let tx = db.create_read_tx().unwrap();
        let lock = tx.substate_locks_get_latest_for_substate(&b9, &substate_id).unwrap();
        assert_eq!(lock.transaction_id(), &granted_last);
        assert!(
            tx.substate_locks_get_latest_for_substate(&b9, &refused_substate)
                .optional()
                .unwrap()
                .is_none(),
            "the refused transaction's locks took effect"
        );
        assert!(
            tx.substate_locks_get_locked_substates_for_transaction(&b9, &granted_first)
                .unwrap()
                .is_empty(),
            "the committed release was lost"
        );
    };
    check(&db);
    let db = reopen(db, &tmp);
    check(&db);
}

/// Recording an empty lock set for a block leaves it holding no locks.
#[test]
fn an_empty_lock_set_replaces_the_blocks_locks() {
    let (db, tmp) = create_rocksdb();
    let chain = create_chain(10);
    let b8 = chain[8].as_leaf();
    let b9 = chain[9].as_leaf();
    let substate_id = create_random_substate_id();
    let (granted_first, granted_last) = ordered_transaction_ids();
    db.with_write_tx(|tx| {
        commit_chain(tx, &chain);
        tx.substate_locks_insert_all(&b8, &two_locks(&substate_id, granted_first, granted_last))?;
        tx.substate_locks_insert_all(&b8, &IndexMap::new())
    })
    .unwrap();

    let db = reopen(db, &tmp);
    let tx = db.create_read_tx().unwrap();
    let lock = tx
        .substate_locks_get_latest_for_substate(&b9, &substate_id)
        .optional()
        .unwrap();
    assert!(lock.is_none());
}

fn reopen(db: RocksDbStateStore<String>, tmp: &tempfile::TempDir) -> RocksDbStateStore<String> {
    drop(db);
    RocksDbStateStore::open(tmp.path().join("rocksdb"), DatabaseOptions::default()).unwrap()
}
