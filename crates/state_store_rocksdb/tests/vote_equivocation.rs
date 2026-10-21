//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for the vote-equivocation evidence column family.

pub mod helpers;

use helpers::{create_rocksdb, create_rocksdb_with_opts};
use tari_consensus_types::{BlockId, ProposalVote, ValidatorSignatureBytes};
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_storage::{
    EpochCleanupStep,
    Ordering,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::VoteEquivocation,
};
use tari_sidechain::QuorumDecision;
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore, column_families::vote_equivocation};
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, Scalar32Bytes, SchnorrSignatureBytes};

const EPOCH: Epoch = Epoch(7);
const HEIGHT: NodeHeight = NodeHeight(42);

/// The column family has no read method on the state store: nothing in the node reads the evidence
/// back yet. Operators reach it through db_inspector, which iterates the CF the same way this does.
fn stored_for_epoch(db: &RocksDbStateStore<String>, epoch: Epoch) -> Vec<VoteEquivocation> {
    let tx = db.create_read_tx().unwrap();
    tx.db()
        .cf(vote_equivocation::ByEpochQuery)
        .unwrap()
        .query_prefix_range_value_iterator(Ordering::Ascending, &epoch)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn signer(byte: u8) -> RistrettoPublicKeyBytes {
    RistrettoPublicKeyBytes::from_bytes(&[byte; 32]).unwrap()
}

fn signature(signer_byte: u8, nonce_byte: u8) -> ValidatorSignatureBytes {
    ValidatorSignatureBytes::new(
        signer(signer_byte),
        SchnorrSignatureBytes::new([nonce_byte; 32].into(), Scalar32Bytes::zero()),
    )
}

fn block_id(byte: u8) -> BlockId {
    BlockId::new(tari_sidechain::FixedHash::new([byte; 32]))
}

fn vote(signer_byte: u8, nonce_byte: u8, block_byte: u8, decision: QuorumDecision) -> ProposalVote {
    ProposalVote {
        epoch: EPOCH,
        block_id: block_id(block_byte),
        block_height: HEIGHT,
        decision,
        signature: signature(signer_byte, nonce_byte),
    }
}

fn evidence(signer_byte: u8) -> VoteEquivocation {
    VoteEquivocation::from_conflicting_votes(
        EPOCH,
        HEIGHT,
        signer(signer_byte),
        vote(signer_byte, 1, 0xA, QuorumDecision::Accept),
        vote(signer_byte, 2, 0xB, QuorumDecision::Accept),
    )
    .expect("votes name different blocks")
}

/// Two signatures over one message are not equivocation. Signing draws a fresh nonce, so any signer
/// can produce arbitrarily many, and each says exactly what the others do.
#[test]
fn votes_attesting_to_the_same_thing_are_not_evidence() {
    let same = VoteEquivocation::from_conflicting_votes(
        EPOCH,
        HEIGHT,
        signer(1),
        vote(1, 1, 0xA, QuorumDecision::Accept),
        vote(1, 2, 0xA, QuorumDecision::Accept),
    );
    assert!(
        same.is_none(),
        "a re-signed vote for the same block is not equivocation"
    );

    let differing_block = VoteEquivocation::from_conflicting_votes(
        EPOCH,
        HEIGHT,
        signer(1),
        vote(1, 1, 0xA, QuorumDecision::Accept),
        vote(1, 1, 0xB, QuorumDecision::Accept),
    );
    assert!(differing_block.is_some());

    let differing_decision = VoteEquivocation::from_conflicting_votes(
        EPOCH,
        HEIGHT,
        signer(1),
        vote(1, 1, 0xA, QuorumDecision::Accept),
        vote(1, 1, 0xA, QuorumDecision::Reject),
    );
    assert!(differing_decision.is_some());
}

#[test]
fn record_round_trips_both_votes() {
    let (db, _tmp) = create_rocksdb();
    let evidence = evidence(1);
    assert!(db.with_write_tx(|tx| tx.vote_equivocation_record(&evidence)).unwrap());

    let stored = stored_for_epoch(&db, EPOCH);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].epoch, EPOCH);
    assert_eq!(stored[0].height, HEIGHT);
    assert_eq!(stored[0].public_key, signer(1));
    assert_eq!(stored[0].first.signature, signature(1, 1));
    assert_eq!(stored[0].first.block_id, block_id(0xA));
    assert_eq!(stored[0].second.signature, signature(1, 2));
    assert_eq!(stored[0].second.block_id, block_id(0xB));
}

/// An equivocator can sign arbitrarily many conflicting votes for one view; only the first pair is
/// kept so that what it makes this node store is bounded.
#[test]
fn the_first_evidence_for_a_view_and_signer_is_kept() {
    let (db, _tmp) = create_rocksdb();
    let first = evidence(1);
    let later = VoteEquivocation::from_conflicting_votes(
        EPOCH,
        HEIGHT,
        signer(1),
        vote(1, 3, 0xC, QuorumDecision::Accept),
        vote(1, 4, 0xD, QuorumDecision::Accept),
    )
    .unwrap();

    assert!(db.with_write_tx(|tx| tx.vote_equivocation_record(&first)).unwrap());
    assert!(!db.with_write_tx(|tx| tx.vote_equivocation_record(&later)).unwrap());

    let stored = stored_for_epoch(&db, EPOCH);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].first.signature, signature(1, 1));
}

#[test]
fn evidence_is_scoped_to_its_view_and_signer() {
    let (db, _tmp) = create_rocksdb();
    let mut other_epoch = evidence(1);
    other_epoch.epoch = EPOCH + Epoch(1);

    db.with_write_tx(|tx| tx.vote_equivocation_record(&evidence(1)))
        .unwrap();
    db.with_write_tx(|tx| tx.vote_equivocation_record(&evidence(2)))
        .unwrap();
    db.with_write_tx(|tx| tx.vote_equivocation_record(&other_epoch))
        .unwrap();

    db.with_read_tx(|tx| {
        assert!(tx.vote_equivocation_exists(EPOCH, HEIGHT, &signer(1))?);
        assert!(tx.vote_equivocation_exists(EPOCH, HEIGHT, &signer(2))?);
        assert!(!tx.vote_equivocation_exists(EPOCH, HEIGHT, &signer(3))?);
        assert!(!tx.vote_equivocation_exists(EPOCH, HEIGHT + NodeHeight(1), &signer(1))?);
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    assert_eq!(stored_for_epoch(&db, EPOCH).len(), 2);
    assert_eq!(stored_for_epoch(&db, EPOCH + Epoch(1)).len(), 1);
}

/// Evidence ages out with the blocks of the view it indicts, so an equivocator cannot grow this
/// column family without bound across epochs.
#[test]
fn epoch_cleanup_prunes_evidence_past_the_retention_window() {
    let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default().with_epoch_history_length(2));
    let mut newer = evidence(1);
    newer.epoch = EPOCH + Epoch(2);

    db.with_write_tx(|tx| tx.vote_equivocation_record(&evidence(1)))
        .unwrap();
    db.with_write_tx(|tx| tx.vote_equivocation_record(&newer)).unwrap();

    // Retention is 2 epochs, so cleaning up at EPOCH + 3 prunes everything at or below EPOCH + 1.
    db.with_write_tx(|tx| tx.epoch_cleanup(EPOCH + Epoch(3))).unwrap();

    assert!(stored_for_epoch(&db, EPOCH).is_empty());
    assert_eq!(stored_for_epoch(&db, EPOCH + Epoch(2)).len(), 1);
}

/// Each call is its own transaction in the GC task, so a node stopping between calls leaves the rest for the next
/// run, which picks up where this one stopped.
#[test]
fn epoch_cleanup_step_is_bounded_and_resumes() {
    let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default().with_epoch_history_length(2));
    for signer_byte in 1..=5 {
        db.with_write_tx(|tx| tx.vote_equivocation_record(&evidence(signer_byte)))
            .unwrap();
    }

    let prune = |limit| {
        db.with_write_tx(|tx| tx.epoch_cleanup_step(EPOCH + Epoch(3), EpochCleanupStep::VoteEquivocations, limit))
            .unwrap()
    };
    assert_eq!(prune(2), 2);
    assert_eq!(stored_for_epoch(&db, EPOCH).len(), 3);
    assert_eq!(prune(2), 2);
    assert_eq!(stored_for_epoch(&db, EPOCH).len(), 1);
    // Fewer than the limit signals the step is done.
    assert_eq!(prune(2), 1);
    assert!(stored_for_epoch(&db, EPOCH).is_empty());
    assert_eq!(prune(2), 0);
}

/// Two writers taking the same keys in opposite orders: whichever closes the cycle fails as soon as it forms, and the
/// other completes with nothing lost.
#[test]
fn a_lock_order_cycle_fails_the_writer_that_closes_it() {
    let (db, _tmp) = create_rocksdb();
    let both_hold_first = std::sync::Barrier::new(2);

    let run = |own: u8, other: u8| {
        let mut tx = db.create_write_tx().unwrap();
        assert!(tx.vote_equivocation_record(&evidence(own)).unwrap());
        both_hold_first.wait();
        let timer = std::time::Instant::now();
        match tx.vote_equivocation_record(&evidence(other)) {
            Ok(recorded) => {
                assert!(recorded);
                tx.commit().unwrap();
                None
            },
            Err(err) => {
                let elapsed = timer.elapsed();
                tx.rollback().unwrap();
                Some((err, elapsed))
            },
        }
    };
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| run(1, 2));
        let b = s.spawn(|| run(2, 1));
        (a.join().unwrap(), b.join().unwrap())
    });

    let mut failures = a.into_iter().chain(b).collect::<Vec<_>>();
    assert_eq!(failures.len(), 1, "exactly one writer closes the cycle: {failures:?}");
    let (err, elapsed) = failures.pop().unwrap();
    assert!(err.to_string().contains("Deadlock"), "{err}");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "deadlock was detected only after {elapsed:?}"
    );
    assert_eq!(stored_for_epoch(&db, EPOCH).len(), 2);
}
