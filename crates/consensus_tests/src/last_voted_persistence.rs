//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Persistence regression test for `LastVoted`.
//!
//! HotStuff's safety argument requires that a node never votes twice at the same view. The
//! voter checks this via `LastVoted::get(tx, epoch)` and refuses to vote if
//! `candidate.height() <= last_voted.height()` (see
//! `crates/consensus/src/hotstuff/on_ready_to_vote_on_local_block.rs:215-237`).
//!
//! If the validator restarts between sending a vote and seeing the resulting QC, the `LastVoted`
//! record must still be on disk afterwards — otherwise the validator could legally vote a second
//! time at the same height, equivocating across restarts. This test verifies the record survives
//! a full RocksDB close/reopen cycle, and that the `should_vote` decision logic (height check)
//! comes out the way the protocol requires.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::{BlockId, Decision, LastSentVote, LastVoted};
use tari_ootle_common_types::{Epoch, NodeHeight, optional::Optional};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::{StateStore, consensus_models::BookkeepingModel};
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};

use crate::support::{NetworkSendObserver, Test, logging::setup_logger};

type TestStore = RocksDbStateStore<PeerAddress>;

const TEST_EPOCH: Epoch = Epoch(0);

/// Mirror of the height-only safety check performed inside `OnReadyToVoteOnLocalBlock::should_vote`.
/// Refusing to vote when `candidate_height <= last_voted.height()` is what prevents per-restart
/// equivocation; if this predicate disagrees with the consensus code, the safety argument breaks.
fn would_vote(last_voted: Option<&LastVoted>, candidate_height: NodeHeight) -> bool {
    match last_voted {
        None => true,
        Some(lv) => candidate_height > lv.height(),
    }
}

#[test]
fn last_voted_survives_close_and_reopen() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().to_path_buf();
    let block_id = BlockId::new(tari_sidechain::FixedHash::new([7u8; 32]));
    let height = NodeHeight(42);

    // Open, write, drop.
    {
        let store: TestStore = RocksDbStateStore::open(&path, DatabaseOptions::default()).unwrap();
        let last_voted = LastVoted {
            block_id,
            height,
            epoch: TEST_EPOCH,
        };
        store.with_write_tx(|tx| last_voted.set(tx)).unwrap();
    }

    // Reopen.
    let store: TestStore = RocksDbStateStore::open(&path, DatabaseOptions::default()).unwrap();
    let reloaded = store
        .with_read_tx(|tx| LastVoted::get(tx, TEST_EPOCH))
        .expect("LastVoted record must persist across restart");

    assert_eq!(reloaded.height(), height);
    assert_eq!(*reloaded.block_id(), block_id);
    assert_eq!(reloaded.epoch(), TEST_EPOCH);
}

/// After restart, the safety predicate must refuse to vote at any height the node has already
/// voted at, and must allow voting at strictly higher heights. The classic equivocation surface
/// is "voted at H, crashed before seeing QC, restarted, and now considering another candidate at
/// H from a competing leader" — the answer must be NO VOTE.
#[test]
fn safety_predicate_respects_persisted_last_voted_after_reopen() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().to_path_buf();
    let voted_block = BlockId::new(tari_sidechain::FixedHash::new([1u8; 32]));
    let voted_height = NodeHeight(10);

    {
        let store: TestStore = RocksDbStateStore::open(&path, DatabaseOptions::default()).unwrap();
        let last_voted = LastVoted {
            block_id: voted_block,
            height: voted_height,
            epoch: TEST_EPOCH,
        };
        store.with_write_tx(|tx| last_voted.set(tx)).unwrap();
    }

    let store: TestStore = RocksDbStateStore::open(&path, DatabaseOptions::default()).unwrap();
    let reloaded = store.with_read_tx(|tx| LastVoted::get(tx, TEST_EPOCH)).unwrap();

    // Same height as the prior vote — even for a different block_id — must be rejected.
    assert!(
        !would_vote(Some(&reloaded), voted_height),
        "must not vote at the same height after restart (equivocation risk)"
    );
    // Lower height — must be rejected.
    assert!(
        !would_vote(Some(&reloaded), NodeHeight(voted_height.as_u64() - 1)),
        "must not vote at a lower height after restart"
    );
    // Strictly higher height — must be allowed.
    assert!(
        would_vote(Some(&reloaded), NodeHeight(voted_height.as_u64() + 1)),
        "voting at a higher height after restart is correct"
    );
}

/// When no `LastVoted` record exists (e.g. first vote in an epoch, or fresh state store), the
/// safety predicate has nothing to gate against and must allow the vote. Documents the "no prior
/// vote" branch of `should_vote`.
#[test]
fn safety_predicate_allows_first_vote_when_no_record_exists() {
    let temp_dir = tempfile::tempdir().unwrap();
    let store: TestStore = RocksDbStateStore::open(temp_dir.path(), DatabaseOptions::default()).unwrap();

    // No LastVoted written. Reading should return NotFound, not a phantom record.
    let result = store.with_read_tx(|tx| LastVoted::get(tx, TEST_EPOCH));
    assert!(result.is_err(), "no LastVoted record should exist on a fresh store");

    // With no record, the safety predicate must permit a vote at any height.
    assert!(would_vote(None, NodeHeight(0)));
    assert!(would_vote(None, NodeHeight(1_000_000)));
}

/// A vote must be on disk before anyone can see it. If the record is written only after the send,
/// a crash in between leaves the restarted node with no memory of having voted at that height and
/// free to vote again there for a competing block — the equivocation `LastVoted` exists to prevent.
///
/// The observer runs on the sending task with the message in hand and nothing sent yet, so what it
/// reads from the store is exactly what a crash at that instant would leave behind.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_vote_is_persisted_before_it_is_sent() {
    setup_logger();

    let violations: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let observed_votes = Arc::new(Mutex::new(0usize));

    let observer = {
        let violations = violations.clone();
        let observed_votes = observed_votes.clone();
        Arc::new(move |address: &_, store: &_, message: &HotstuffMessage| {
            let HotstuffMessage::Vote(msg) = message else {
                return;
            };
            let vote = &msg.vote;
            *observed_votes.lock().unwrap() += 1;

            let (last_voted, last_sent_vote) = RocksDbStateStore::with_read_tx(store, |tx| {
                Ok::<_, tari_ootle_storage::StorageError>((
                    LastVoted::get(tx, vote.epoch).optional()?,
                    LastSentVote::get(tx, vote.epoch).optional()?,
                ))
            })
            .unwrap();

            let mut violations = violations.lock().unwrap();
            match last_voted {
                Some(last_voted) if last_voted.height == vote.block_height => {},
                other => violations.push(format!(
                    "{address} sent a vote at height {} with LastVoted {:?}",
                    vote.block_height,
                    other.map(|lv| lv.height)
                )),
            }
            match last_sent_vote {
                Some(last_sent_vote) if last_sent_vote.vote.block_id == vote.block_id => {},
                other => violations.push(format!(
                    "{address} sent a vote for block {} with LastSentVote {:?}",
                    vote.block_id,
                    other.map(|lsv| lsv.vote.block_id)
                )),
            }
        }) as NetworkSendObserver
    };

    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .with_send_observer(observer)
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
    test.start_epoch(Epoch(1)).await;

    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;
        if test.is_transaction_pool_empty() {
            break;
        }
        if committed_height > NodeHeight(20) {
            panic!("Transaction was not committed after {committed_height} blocks");
        }
    }

    test.assert_clean_shutdown().await;

    let observed = *observed_votes.lock().unwrap();
    assert!(observed > 0, "No votes were observed; the test proves nothing");
    let violations = violations.lock().unwrap();
    assert!(
        violations.is_empty(),
        "{} of {observed} votes were sent before being persisted:\n{}",
        violations.len(),
        violations.join("\n")
    );
}
