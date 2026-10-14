//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for a cross-shard transaction whose foreign shard groups never deliver their proposals.

use std::time::{Duration, Instant};

use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::Decision;
use tari_engine_types::commit_result::AbortReason;
use tari_ootle_common_types::Epoch;
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{TransactionPoolRecord, TransactionPoolStage},
};
use tari_ootle_transaction::TransactionId;

use crate::support::{Test, TestAddress, TestVnDestination, logging::setup_logger};

fn is_committee_1(addr: &TestAddress) -> bool {
    addr == "3" || addr == "4"
}

fn pool_record(test: &Test, addr: &str, transaction_id: &TransactionId) -> Option<TransactionPoolRecord> {
    test.get_validator(&TestAddress::new(addr))
        .state_store
        .with_read_tx(|tx| tx.transaction_pool_get_all(usize::MAX))
        .unwrap()
        .into_iter()
        .find(|rec| rec.id() == transaction_id)
}

async fn wait_for_epoch(test: &mut Test, epoch: Epoch, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let _unused = tokio::time::timeout(Duration::from_millis(100), test.on_block_committed()).await;
        if test.validators_iter().all(|v| v._current_view.get_epoch() >= epoch) {
            return;
        }
    }
    panic!("Timed out waiting for all validators to reach {epoch}");
}

/// Committee 0 prepares a transaction that also has an input in committee 1, but never receives a proposal from
/// committee 1. Once the transaction's `max_epoch` and one further epoch have passed, committee 0 aborts it from
/// `LocalPrepared` and releases its locks, and committee 1 follows that abort.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prepared_transaction_aborts_after_max_epoch_when_foreign_group_never_delivers() {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|cfg| {
            cfg.epoch_end_grace_period = Duration::from_millis(10);
        })
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(1);
        })
        .with_test_timeout(Duration::from_secs(60))
        .with_message_filter(Box::new(|from: &TestAddress, to: &TestAddress, msg| {
            let is_foreign_proposal = matches!(
                msg,
                HotstuffMessage::ForeignProposal(_) | HotstuffMessage::ForeignProposalNotification(_)
            );
            !(is_foreign_proposal && is_committee_1(from) && !is_committee_1(to))
        }))
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    test.start_epoch(Epoch(1)).await;
    // One input in each committee. The test transactions declare max_epoch 1.
    let (transaction, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    let transaction_id = *transaction.id();
    assert_eq!(transaction.transaction().max_epoch(), Epoch(1));

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        test.on_block_committed().await;
        let prepared = pool_record(&test, "1", &transaction_id)
            .is_some_and(|rec| rec.committed_stage() == TransactionPoolStage::LocalPrepared);
        let accepted = pool_record(&test, "3", &transaction_id)
            .is_some_and(|rec| rec.committed_stage() == TransactionPoolStage::LocalAccepted);
        if prepared && accepted {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Committee 0 did not reach LocalPrepared and committee 1 LocalAccepted"
        );
    }

    // max_epoch + 1 is the grace epoch: the transaction is still waiting on committee 1.
    test.start_epoch(Epoch(2)).await;
    wait_for_epoch(&mut test, Epoch(2), Duration::from_secs(30)).await;
    for _ in 0..5 {
        test.on_block_committed().await;
    }
    let rec = pool_record(&test, "1", &transaction_id).expect("transaction still in committee 0's pool");
    assert_eq!(rec.current_stage(), TransactionPoolStage::LocalPrepared);
    assert!(rec.current_decision().is_commit());

    test.start_epoch(Epoch(3)).await;
    wait_for_epoch(&mut test, Epoch(3), Duration::from_secs(30)).await;

    let deadline = Instant::now() + Duration::from_secs(30);
    while !test.is_transaction_finalized_at_destination(TestVnDestination::All, &transaction_id) {
        assert!(
            Instant::now() < deadline,
            "Transaction {transaction_id} was not finalized after its max_epoch passed"
        );
        let _unused = tokio::time::timeout(Duration::from_millis(100), test.on_block_committed()).await;
    }

    test.assert_all_validators_have_decision(&transaction_id, Decision::Abort(AbortReason::EpochExpired))
        .await;
    test.assert_all_validators_did_not_commit(&transaction_id);
    for addr in ["1", "2"] {
        let vn = test.get_validator(&TestAddress::new(addr));
        let leaf = vn.get_leaf_block();
        let locks = vn
            .state_store
            .with_read_tx(|tx| tx.substate_locks_get_locked_substates_for_transaction(&leaf, &transaction_id))
            .unwrap();
        assert!(
            locks.is_empty(),
            "{addr} still holds locks for {transaction_id}: {locks:?}"
        );
    }

    test.stop();
    test.assert_clean_shutdown().await;
}
