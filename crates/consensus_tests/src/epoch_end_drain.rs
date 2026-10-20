//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for draining cross-shard transactions before an end of epoch that changes the number of committees.

use std::time::{Duration, Instant};

use tari_consensus::{hotstuff::ConsensusCurrentState, messages::HotstuffMessage};
use tari_consensus_types::{Decision, PcId};
use tari_engine_types::commit_result::AbortReason;
use tari_ootle_common_types::{Epoch, ShardGroup, SubstateVersion, VersionedSubstateId, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{EpochCheckpoint, TransactionPoolRecord, TransactionPoolStage, TransactionRecord},
};
use tari_ootle_transaction::TransactionId;

use crate::support::{
    TEST_NUM_PRESHARDS,
    Test,
    TestAddress,
    TestVnDestination,
    build_transaction,
    committee_number_to_shard_group,
    helpers::random_substate_in_shard_group,
    logging::setup_logger,
};

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

/// What a validator holds for a transaction once it has committed the end of `epoch`.
#[derive(Debug)]
struct AtEpochEnd {
    decision: Option<Decision>,
    num_locks: usize,
}

fn at_epoch_end(test: &Test, addr: &str, epoch: Epoch, transaction_id: &TransactionId) -> Option<AtEpochEnd> {
    let vn = test.get_validator(&TestAddress::new(addr));
    let leaf = vn.get_leaf_block();
    vn.state_store
        .with_read_tx(|tx| {
            let checkpoint = EpochCheckpoint::get_last_checkpoint(tx).optional()?;
            if checkpoint.is_none_or(|c| c.epoch() < epoch) {
                return Ok(None);
            }
            let decision = TransactionRecord::get_finalized_decision(tx, transaction_id)?;
            let num_locks = tx
                .substate_locks_get_locked_substates_for_transaction(&leaf, transaction_id)?
                .len();
            Ok::<_, tari_ootle_storage::StorageError>(Some(AtEpochEnd { decision, num_locks }))
        })
        .unwrap()
}

/// A cross-shard transaction is prepared by committee 0 and accepted by committee 1, but committee 0 never receives
/// committee 1's proposals. The next epoch changes the number of committees, so every validator moves to another
/// shard group and drops its transaction pool. Committee 0 must abort the transaction from LocalPrepared, and both
/// committees must finalize it and release its locks, before either ends the epoch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_shard_transaction_is_finalized_before_an_epoch_end_that_changes_the_committees() {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|cfg| {
            cfg.epoch_end_grace_period = Duration::from_millis(10);
            // Committee 0's abort must reach committee 1 within one drain period of the abort being proposed, before
            // committee 1's drain is overdue and EndEpoch stops waiting for its LocalAccepted transaction.
            cfg.epoch_end_drain_period = Duration::from_secs(15);
        })
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(1);
        })
        .with_test_timeout(Duration::from_secs(90))
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
    let (transaction, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    let transaction_id = *transaction.id();

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

    // Every validator shares the epoch manager state, so this applies to all of them.
    test.get_validator(&TestAddress::new("1"))
        .epoch_manager
        .set_epoch_num_committees(Epoch(2), 4)
        .await;
    test.start_epoch(Epoch(2)).await;

    // After the end of epoch 1 commits, validators move to new shard groups, which the harness cannot sync them
    // into. Only the state each validator wrote up to that point is inspected, so the store is polled rather than
    // the event stream.
    let addresses = ["1", "2", "3", "4"];
    let mut results = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    for addr in addresses {
        let result = loop {
            if let Some(result) = at_epoch_end(&test, addr, Epoch(1), &transaction_id) {
                break result;
            }
            assert!(Instant::now() < deadline, "{addr} did not end epoch 1");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        results.push((addr, result));
    }

    for (addr, result) in results {
        assert_eq!(
            result.decision,
            Some(Decision::Abort(AbortReason::EpochExpired)),
            "{addr} ended epoch 1 without finalizing {transaction_id}: {result:?}"
        );
        assert_eq!(
            result.num_locks, 0,
            "{addr} ended epoch 1 holding locks for {transaction_id}: {result:?}"
        );
    }

    test.stop();
}

/// Waits until every validator in `addresses` pools each of `transaction_ids`, and asserts it holds them in `New`.
async fn wait_until_held_back(test: &Test, addresses: &[&str], transaction_ids: &[TransactionId]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    for addr in addresses {
        for id in transaction_ids {
            loop {
                if let Some(rec) = pool_record(test, addr, id) {
                    assert_eq!(
                        rec.committed_stage(),
                        TransactionPoolStage::New,
                        "{addr} did not hold {id} back"
                    );
                    break;
                }
                assert!(Instant::now() < deadline, "{addr} did not pool {id}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Waits until every validator in `addresses` has completed a sync and left `Syncing`.
async fn wait_until_synced(test: &Test, addresses: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(30);
    for &addr in addresses {
        let vn = test.get_validator(&TestAddress::new(addr));
        while vn.sync_manager.num_syncs() == 0 || vn.current_state_machine_state() == ConsensusCurrentState::Syncing {
            assert!(Instant::now() < deadline, "{addr} did not sync");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Rewrites `addr`'s pool record for `transaction_id` as it stands once `addr` has committed `foreign`'s LocalPrepare
/// of it.
fn record_foreign_prepare(test: &Test, addr: &str, transaction_id: &TransactionId, foreign: ShardGroup) {
    test.get_validator(&TestAddress::new(addr))
        .state_store
        .with_write_tx(|tx| {
            let rec = tx
                .transaction_pool_remove_all([transaction_id])?
                .pop()
                .expect("transaction is pooled");
            let mut evidence = rec.evidence().clone();
            evidence
                .get_mut(&foreign)
                .expect("foreign shard group is involved")
                .set_prepare_qc(PcId::zero());
            tx.transaction_pool_insert_new(
                *transaction_id,
                rec.original_decision(),
                &evidence,
                rec.is_ready(),
                rec.is_global(),
                rec.max_epoch(),
                rec.transaction_weight(),
            )
        })
        .unwrap();
}

/// Two cross-shard transactions are submitted while the drain holds new cross-shard transactions back, and are still
/// `New` when the epoch ends. Committee 0 has committed committee 1's LocalPrepare of the second one. Every validator
/// then moves to a new shard group and goes through `Syncing`, which drops its pool.
///
/// The validators whose new shard group holds one of the first transaction's inputs must have it back in their pools,
/// with evidence for the new committees. The second one is committee 1's to abort, so committee 0's validators must
/// not sequence it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transaction_held_back_by_the_drain_is_sequenced_again_after_the_sync() {
    const NEXT_NUM_COMMITTEES: u32 = 4;
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|cfg| {
            // Leaves time to submit the transaction once the next epoch is known and before EndEpoch is proposed.
            cfg.epoch_end_grace_period = Duration::from_secs(3);
        })
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(1);
        })
        .with_test_timeout(Duration::from_secs(60))
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    test.start_epoch(Epoch(1)).await;
    test.wait_for_all_validators_to_start_consensus().await;

    let addresses = ["1", "2", "3", "4"];
    let next_shard_group = |test: &Test, addr: &str| {
        test.get_validator(&TestAddress::new(addr))
            .shard_address
            .to_shard_group(TEST_NUM_PRESHARDS, NEXT_NUM_COMMITTEES)
    };
    // One input in the next shard group of "1" and one in that of "3": a cross-shard transaction between the current
    // committees, which each of those validators holds an input of after the split.
    let build_inputs = |test: &Test| {
        ["1", "3"].map(|addr| {
            VersionedSubstateId::new(
                random_substate_in_shard_group(next_shard_group(test, addr), TEST_NUM_PRESHARDS),
                SubstateVersion::ZERO,
            )
        })
    };
    let inputs = build_inputs(&test);
    let transaction = build_transaction(inputs.iter().cloned().map(Into::into).collect());
    let transaction_id = *transaction.id();
    let foreign_prepared = build_transaction(build_inputs(&test).into_iter().map(Into::into).collect());
    let foreign_prepared_id = *foreign_prepared.id();

    test.get_validator(&TestAddress::new("1"))
        .epoch_manager
        .set_epoch_num_committees(Epoch(2), NEXT_NUM_COMMITTEES)
        .await;
    test.start_epoch(Epoch(2)).await;
    // The drain starts once a validator has seen the next epoch.
    tokio::time::sleep(Duration::from_millis(500)).await;
    test.send_transaction_to_destination(TestVnDestination::All, transaction)
        .await;
    test.send_transaction_to_destination(TestVnDestination::All, foreign_prepared)
        .await;

    wait_until_held_back(&test, &addresses, &[transaction_id, foreign_prepared_id]).await;

    let committee_1 = committee_number_to_shard_group(TEST_NUM_PRESHARDS, 1, 2);
    for addr in ["1", "2"] {
        record_foreign_prepare(&test, addr, &foreign_prepared_id, committee_1);
    }

    // A validator whose shard group moves state-syncs the new group.
    for addr in addresses {
        test.get_validator(&TestAddress::new(addr))
            .sync_manager
            .report_behind_on_next_check();
    }
    wait_until_synced(&test, &addresses).await;

    let next_shard_groups = (0..NEXT_NUM_COMMITTEES)
        .map(|n| committee_number_to_shard_group(TEST_NUM_PRESHARDS, n, NEXT_NUM_COMMITTEES))
        .collect::<Vec<_>>();
    for addr in addresses {
        let next_shard_group = next_shard_group(&test, addr);
        let holds_an_input = inputs
            .iter()
            .any(|input| next_shard_group.contains(&input.to_shard(TEST_NUM_PRESHARDS)));
        let rec = pool_record(&test, addr, &transaction_id);
        assert_eq!(
            rec.is_some(),
            holds_an_input,
            "{addr} (next shard group {next_shard_group}) pooled {transaction_id} after the sync: expected \
             {holds_an_input}"
        );
        if let Some(rec) = rec {
            for shard_group in rec.evidence().shard_groups_iter() {
                assert!(
                    next_shard_groups.contains(shard_group),
                    "{addr} holds {transaction_id} with evidence for {shard_group}, which is not a shard group of \
                     epoch 2"
                );
            }
        }
    }
    for addr in ["1", "2"] {
        assert!(
            pool_record(&test, addr, &foreign_prepared_id).is_none(),
            "{addr} sequenced {foreign_prepared_id} again after committee 1 prepared it"
        );
    }

    test.stop();
}
