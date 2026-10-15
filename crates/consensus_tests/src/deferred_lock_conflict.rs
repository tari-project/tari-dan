//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for how a proposer treats a transaction whose inputs are locked by another transaction that has not yet
//! released them.

use tari_common_types::types::PrivateKey;
use tari_consensus_types::Decision;
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_common_types::{Epoch, InputDeclaration, NodeHeight, SubstateLockType};
use tari_ootle_storage::consensus_models::TransactionRecord;
use tari_ootle_transaction::Transaction;

use crate::support::{ExecuteSpec, Test, TestAddress, TestVnDestination, logging::setup_logger};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_transaction_deferred_for_a_lock_conflict_is_executed_once() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1"])
        .add_committee(1, vec!["2"])
        .start()
        .await;

    let local = test
        .create_substates_on_vns(TestVnDestination::Committee(0), 1)
        .remove(0);
    let foreign = test
        .create_substates_on_vns(TestVnDestination::Committee(1), 1)
        .remove(0);

    // The blocker spans both committees, so committee 0 holds its write lock on `local` for the blocks its foreign
    // pledges take to arrive.
    let blocker = Transaction::builder_localnet(Epoch(1))
        .add_input(InputDeclaration::write_versioned(
            local.substate_id().clone(),
            local.version(),
        ))
        .add_input(InputDeclaration::write_versioned(
            foreign.substate_id().clone(),
            foreign.version(),
        ))
        .build_and_seal(&PrivateKey::from_canonical_bytes(&[1u8; 32]).unwrap());
    let blocker = TransactionRecord::new(blocker);

    // Writing `local` without naming a version, the blocked transaction meets a lock conflict that defers it until the
    // blocker releases the lock, rather than one that aborts it.
    let blocked = Transaction::builder_localnet(Epoch(1))
        .add_input(InputDeclaration::write(local.substate_id().clone()))
        .build_and_seal(&PrivateKey::from_canonical_bytes(&[2u8; 32]).unwrap());
    let blocked = TransactionRecord::new(blocked);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: blocker.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![
            (local.substate_id().clone(), SubstateLockType::Write),
            (foreign.substate_id().clone(), SubstateLockType::Write),
        ],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    })
    .add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: blocked.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![(local.substate_id().clone(), SubstateLockType::Write)],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    });

    test.send_transaction_to_destination(TestVnDestination::All, blocker.clone())
        .await;
    test.start_epoch(Epoch(1)).await;
    // The blocker is locked in the first block, so the blocked transaction arrives while the lock is held.
    test.on_block_committed().await;
    test.send_transaction_to_destination(TestVnDestination::All, blocked.clone())
        .await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height > NodeHeight(40) {
            test.dump_pool_info();
            panic!("Not all transactions committed after {} blocks", leaf.height);
        }
    }

    test.stop();

    test.assert_all_validators_have_decision(blocker.id(), Decision::Commit)
        .await;
    // Committee 1 is involved in the blocked transaction only if its receipt happens to shard there.
    test.assert_validators_have_decision(TestVnDestination::Committee(0), blocked.id(), Decision::Commit)
        .await;
    let execute_count = test
        .get_validator(&TestAddress::new("1"))
        .transaction_executor
        .get_execute_count(blocked.id());
    assert_eq!(
        execute_count, 1,
        "the blocked transaction was executed {execute_count} times"
    );

    test.assert_clean_shutdown().await;
}
