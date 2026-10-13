//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests that pledged substate values never make a proposal too large to deliver.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tari_common_types::types::PrivateKey;
use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::Decision;
use tari_engine_types::commit_result::AbortReason;
use tari_ootle_common_types::{Epoch, InputDeclaration, NodeHeight, SubstateLockType, VersionedSubstateId};
use tari_ootle_storage::consensus_models::TransactionRecord;
use tari_ootle_transaction::Transaction;

use crate::support::{Test, TestAddress, TestVnDestination, build_transaction_from, logging::setup_logger};

/// The largest message the test network delivers between two validators, standing in for the transport's frame
/// limit at a scale the test can reach.
const TEST_FRAME_BYTES: usize = 256 * 1024;
const TEST_MAX_BLOCK_PLEDGE_BYTES: usize = 128 * 1024;
const TEST_MAX_BLOCK_FOREIGN_PROPOSAL_BYTES: usize = 160 * 1024;
const TEST_MAX_BLOCK_VALIDATION_FOREIGN_PROPOSAL_BYTES: usize = 192 * 1024;
const LARGE_COMPONENT_STATE_BYTES: usize = 100 * 1024;

fn message_size(msg: &HotstuffMessage) -> usize {
    match msg {
        HotstuffMessage::Proposal(msg) | HotstuffMessage::CatchUpSyncResponse(msg) => {
            tari_bor::encoded_len(&msg.block) + msg.foreign_proposals.iter().map(tari_bor::encoded_len).sum::<usize>()
        },
        HotstuffMessage::ForeignProposal(msg) => tari_bor::encoded_len(&*msg.proposal),
        _ => 0,
    }
}

/// Committee 0 holds large components and committee 1 small ones. The network drops any message between two
/// validators that is over [`TEST_FRAME_BYTES`], counting it in the returned counter.
async fn start_test() -> (Test, Arc<AtomicUsize>) {
    setup_logger();
    let num_oversized = Arc::new(AtomicUsize::new(0));
    let test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .with_message_filter({
            let num_oversized = num_oversized.clone();
            Box::new(move |from, to, msg| {
                if from == to || message_size(msg) <= TEST_FRAME_BYTES {
                    return true;
                }
                log::warn!("🗑️ [TEST] Dropping oversized {} from {from} to {to}", msg.as_type_str());
                num_oversized.fetch_add(1, Ordering::Relaxed);
                false
            })
        })
        .modify_consensus_constants(|constants| {
            constants.max_block_pledge_bytes = TEST_MAX_BLOCK_PLEDGE_BYTES;
            constants.max_block_foreign_proposal_bytes = TEST_MAX_BLOCK_FOREIGN_PROPOSAL_BYTES;
            constants.max_block_validation_foreign_proposal_bytes = TEST_MAX_BLOCK_VALIDATION_FOREIGN_PROPOSAL_BYTES;
        })
        .start()
        .await;
    (test, num_oversized)
}

/// A transaction that reads `large_inputs` from committee 0 and writes `small_input` in committee 1, so committee 0
/// pledges the large values to committee 1.
async fn send_cross_shard_read(
    test: &mut Test,
    large_inputs: &[VersionedSubstateId],
    small_input: &VersionedSubstateId,
    decision: Decision,
) -> TransactionRecord {
    let mut builder = Transaction::builder_localnet(Epoch(1));
    for large in large_inputs {
        builder = builder.add_input(InputDeclaration::read_versioned(
            large.substate_id().clone(),
            large.version(),
        ));
    }
    let tx = build_transaction_from(
        builder
            .add_input(InputDeclaration::write_versioned(
                small_input.substate_id().clone(),
                small_input.version(),
            ))
            .build_and_seal(&PrivateKey::default()),
    );
    let input_locks = large_inputs
        .iter()
        .map(|large| (large.substate_id().clone(), SubstateLockType::Read))
        .chain([(small_input.substate_id().clone(), SubstateLockType::Write)])
        .collect();
    test.create_execution_at_destination_for_transaction(TestVnDestination::All, &tx, decision, 5, input_locks, vec![]);
    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;
    tx
}

async fn run_until_pool_is_empty(test: &mut Test, num_oversized: &AtomicUsize) {
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf3 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(60) || leaf3.height > NodeHeight(60) {
            panic!(
                "Not all transactions finalized after {}/{} blocks ({} oversized message(s) dropped)",
                leaf1.height,
                leaf3.height,
                num_oversized.load(Ordering::Relaxed),
            );
        }
    }

    test.stop();
}

/// Each transaction pledges one large value. Committee 0 spreads them over enough blocks that each foreign proposal
/// fits, and committee 1 spreads the foreign proposals over enough blocks that each of its proposals fits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_pledges_do_not_stall_the_receiving_committee() {
    const NUM_TRANSACTIONS: usize = 6;
    let (mut test, num_oversized) = start_test().await;

    let large_inputs = test.create_substates_on_vns_with_state_size(
        TestVnDestination::Committee(0),
        NUM_TRANSACTIONS,
        LARGE_COMPONENT_STATE_BYTES,
    );
    let small_inputs = test.create_substates_on_vns(TestVnDestination::Committee(1), NUM_TRANSACTIONS);

    let mut transactions = Vec::with_capacity(NUM_TRANSACTIONS);
    for (large, small) in large_inputs.iter().zip(&small_inputs) {
        let tx = send_cross_shard_read(&mut test, std::slice::from_ref(large), small, Decision::Commit).await;
        transactions.push(tx);
    }

    run_until_pool_is_empty(&mut test, &num_oversized).await;

    for tx in &transactions {
        test.assert_all_validators_have_decision(tx.id(), Decision::Commit)
            .await;
        // Committee 0 only reads, so the transaction's substates are all committed in committee 1
        test.assert_destination_has_committed(TestVnDestination::Committee(1), tx.id());
    }
    assert_eq!(
        num_oversized.load(Ordering::Relaxed),
        0,
        "a validator sent a message larger than the frame limit"
    );
    test.assert_clean_shutdown().await;
}

/// No block could carry a pledge of two large values, so the transaction is aborted rather than left locked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_transaction_pledging_more_than_a_block_can_carry_is_aborted() {
    let (mut test, num_oversized) = start_test().await;

    let large_inputs =
        test.create_substates_on_vns_with_state_size(TestVnDestination::Committee(0), 2, LARGE_COMPONENT_STATE_BYTES);
    let small_inputs = test.create_substates_on_vns(TestVnDestination::Committee(1), 1);
    let tx = send_cross_shard_read(&mut test, &large_inputs, &small_inputs[0], Decision::Commit).await;

    run_until_pool_is_empty(&mut test, &num_oversized).await;

    test.assert_all_validators_have_decision(tx.id(), Decision::Abort(AbortReason::InputPledgeTooLarge))
        .await;
    test.assert_all_validators_did_not_commit(tx.id());
    assert_eq!(
        num_oversized.load(Ordering::Relaxed),
        0,
        "a validator sent a message larger than the frame limit"
    );
    test.assert_clean_shutdown().await;
}
