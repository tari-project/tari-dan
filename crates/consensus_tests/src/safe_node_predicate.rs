//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Edge-case tests for the safeNode predicate.
//!
//! These are non-malicious scenarios that a correct HotStuff implementation must handle. The
//! safeNode predicate is defined in `Block::is_safe`
//! (`crates/storage/src/consensus_models/block.rs`) and gates whether a proposed block is allowed
//! to be voted on. Two rules apply:
//!
//! - Safety: the candidate extends the currently locked block.
//! - Liveness: the candidate's `max_certificate_height` (max of justify and TC height) exceeds the locked block's
//!   height.
//!
//! Either rule independently makes the candidate safe. The tests below construct minimal real
//! chains in a tempdir-backed state store and exercise both rules and the unsafe corner.

use std::collections::BTreeSet;

use tari_common_types::types::FixedHash;
use tari_consensus::traits::CertificateStore;
use tari_consensus_types::{BlockId, LeafBlock, ProposalCertificate, ShardGroupAccumulatedData};
use tari_crypto::tari_utilities::epoch_time::EpochTime;
use tari_ootle_common_types::{Epoch, ExtraData, NodeHeight, NumPreshards, ProtocolVersion, ShardGroup};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::{
    StateStore,
    StorageError,
    consensus_models::{Block, BlockHeader, BookkeepingModel},
};
use tari_ootle_transaction::Network;
use tari_sidechain::QuorumDecision;
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tempfile::TempDir;

type TestStore = RocksDbStateStore<PeerAddress>;

const NUM_PRESHARDS: NumPreshards = NumPreshards::P256;
const NETWORK: Network = Network::LocalNet;
const TEST_EPOCH: Epoch = Epoch(0);

fn create_store() -> (TestStore, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = RocksDbStateStore::open(temp_dir.path(), DatabaseOptions::default()).unwrap();
    store
        .with_write_tx(|tx| {
            let zero = Block::zero_block(NETWORK, NUM_PRESHARDS);
            zero.justify().save(tx)?;
            zero.insert(tx)
        })
        .unwrap();
    (store, temp_dir)
}

fn qc_of(target: &LeafBlock) -> ProposalCertificate {
    ProposalCertificate::new(
        *target.block_id().hash(),
        *target.block_id(),
        target.height(),
        target.epoch(),
        ShardGroup::all_shards(NUM_PRESHARDS),
        vec![],
        QuorumDecision::Accept,
    )
}

/// Build a real block extending `parent_id` at `height` with `justify`. `marker` is mixed into
/// the state Merkle root so two siblings at the same height produce distinct block ids.
fn build_block(parent_id: BlockId, justify: ProposalCertificate, height: NodeHeight, marker: u8) -> Block {
    let mut state_root = [0u8; FixedHash::byte_size()];
    state_root[0] = marker;
    state_root[1] = height.as_u64() as u8;
    let header = BlockHeader::create_unsigned(
        NETWORK,
        ProtocolVersion::V0,
        parent_id,
        justify.calculate_id(),
        height,
        TEST_EPOCH,
        ShardGroup::all_shards(NUM_PRESHARDS),
        RistrettoPublicKeyBytes::default(),
        FixedHash::new(state_root),
        &BTreeSet::new(),
        0,
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::new(),
    )
    .unwrap();
    Block::new(header, justify, BTreeSet::new(), None)
}

/// Safety rule: a candidate that extends the locked block is safe even when its justify is at
/// the locked block's height (i.e. no liveness signal).
#[test]
fn safe_when_candidate_extends_locked_at_equal_height() {
    let (store, _tmp) = create_store();
    let zero = Block::zero_block(NETWORK, NUM_PRESHARDS);

    let locked = build_block(*zero.id(), zero.justify().clone(), NodeHeight(1), 1);
    let qc_locked = qc_of(&locked.as_leaf());

    // Candidate extends `locked` directly; its justify is qc(locked) (same height as locked).
    let candidate = build_block(*locked.id(), qc_locked.clone(), NodeHeight(2), 1);

    store
        .with_write_tx(|tx| {
            locked.insert(tx)?;
            qc_locked.save(tx)?;
            locked.as_locked().set(tx)?;
            Ok::<_, StorageError>(())
        })
        .unwrap();

    let is_safe = store.with_read_tx(|tx| candidate.is_safe(tx)).unwrap();
    assert!(
        is_safe,
        "candidate that extends the locked block must be safe (safety rule)"
    );
}

/// Liveness rule: a candidate whose justify height is strictly greater than the locked block's
/// height is safe even if its parent chain forks off the locked block. The liveness rule is a
/// signal that consensus has moved past the locked block and we must keep up.
#[test]
fn safe_when_justify_height_exceeds_locked_even_on_sibling_chain() {
    let (store, _tmp) = create_store();
    let zero = Block::zero_block(NETWORK, NUM_PRESHARDS);

    // chain A: zero -> a1 (locked)
    let locked = build_block(*zero.id(), zero.justify().clone(), NodeHeight(1), 1);

    // chain B: zero -> b1 -> b2, then candidate c at height 3 with justify(b2). b1/b2 are
    // siblings of `locked`, so the candidate does NOT extend `locked`. But justify(b2).height
    // = 2 > 1 = locked.height, so the liveness rule applies.
    let b1 = build_block(*zero.id(), zero.justify().clone(), NodeHeight(1), 2);
    let b2 = build_block(*b1.id(), qc_of(&b1.as_leaf()), NodeHeight(2), 2);
    let candidate = build_block(*b2.id(), qc_of(&b2.as_leaf()), NodeHeight(3), 2);

    store
        .with_write_tx(|tx| {
            locked.insert(tx)?;
            b1.insert(tx)?;
            b2.insert(tx)?;
            qc_of(&b1.as_leaf()).save(tx)?;
            qc_of(&b2.as_leaf()).save(tx)?;
            locked.as_locked().set(tx)?;
            Ok::<_, StorageError>(())
        })
        .unwrap();

    let is_safe = store.with_read_tx(|tx| candidate.is_safe(tx)).unwrap();
    assert!(
        is_safe,
        "candidate with justify height > locked must be safe (liveness rule), even when forked"
    );
}

/// Unsafe: candidate forks off the locked block AND its `max_certificate_height` does not exceed
/// locked.height. Neither rule applies, so the predicate must reject.
#[test]
fn unsafe_when_neither_rule_applies() {
    let (store, _tmp) = create_store();
    let zero = Block::zero_block(NETWORK, NUM_PRESHARDS);

    // zero -> locked (height 1). Sibling chain: zero -> sibling (height 1). Candidate at height
    // 2 extends `sibling`, justify(sibling) has height 1 (NOT > locked.height).
    let locked = build_block(*zero.id(), zero.justify().clone(), NodeHeight(1), 1);
    let sibling = build_block(*zero.id(), zero.justify().clone(), NodeHeight(1), 2);
    let candidate = build_block(*sibling.id(), qc_of(&sibling.as_leaf()), NodeHeight(2), 2);

    store
        .with_write_tx(|tx| {
            locked.insert(tx)?;
            sibling.insert(tx)?;
            qc_of(&sibling.as_leaf()).save(tx)?;
            locked.as_locked().set(tx)?;
            Ok::<_, StorageError>(())
        })
        .unwrap();

    let is_safe = store.with_read_tx(|tx| candidate.is_safe(tx)).unwrap();
    assert!(
        !is_safe,
        "candidate that forks off locked and has no liveness signal must be unsafe"
    );
}
