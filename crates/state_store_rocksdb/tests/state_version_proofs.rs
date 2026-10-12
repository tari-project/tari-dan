//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{create_rocksdb_with_opts, substate_id_seed};
use tari_consensus_types::BlockId;
use tari_ootle_common_types::{Epoch, SubstateVersion, VersionedSubstateId, optional::Optional, shard::Shard};
use tari_ootle_storage::{
    EpochCleanupStep,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{
        STATE_VERSION_PROOF_INTERVAL,
        StateVersionProof,
        StateVersionProofSource,
        SubstateDownProofRecord,
    },
};
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
use tari_state_tree::{KeyedProofTree, LeafKey, SparseMerkleProofExt, TreeHash, Version};
use tempfile::TempDir;

const EPOCH: Epoch = Epoch(1);
const HISTORY: u64 = 3;
const SHARD: Shard = Shard::from_u32(1);
const OTHER_SHARD: Shard = Shard::from_u32(2);

fn create_store() -> (RocksDbStateStore<String>, TempDir) {
    create_rocksdb_with_opts(DatabaseOptions::default().with_down_proof_history_length(HISTORY))
}

fn block(seed: u8) -> BlockId {
    BlockId::from([seed; 32])
}

fn proof_ext() -> SparseMerkleProofExt {
    let key = LeafKey::new(TreeHash::new([1; 32]));
    let tree = KeyedProofTree::build([(key, TreeHash::new([1; 32]))]).unwrap();
    tree.get_proof(&key).unwrap().1
}

/// Stores the proof of `shard` at `state_version`, committed by `block_id` in `epoch`, with the block's commit proof.
fn commit_proof_of(
    store: &RocksDbStateStore<String>,
    epoch: Epoch,
    shard: Shard,
    state_version: Version,
    block_id: BlockId,
) {
    store
        .with_write_tx(|tx| {
            tx.state_version_proofs_insert(epoch, &StateVersionProof {
                shard,
                state_version,
                source: StateVersionProofSource::Committed { block_id },
                shard_root_proof: proof_ext(),
            })?;
            tx.block_commit_proofs_insert(&block_id, b"commit proof")
        })
        .unwrap();
}

/// Stores the proof of `shard`'s next version, committed in `epoch` by a block of its own, superseding the proof of
/// `state_version` as the shard's latest.
fn supersede(store: &RocksDbStateStore<String>, epoch: Epoch, shard: Shard, state_version: Version) {
    let next = state_version + 1;
    commit_proof_of(store, epoch, shard, next, block(u8::try_from(next).unwrap()));
}

/// Stores a down proof record, for a substate destroyed in `epoch`, that cites the proof of `shard` at
/// `state_version`.
fn cite(store: &RocksDbStateStore<String>, epoch: Epoch, shard: Shard, state_version: Version, seed: u32) {
    let id = VersionedSubstateId::new(substate_id_seed(seed), SubstateVersion::ZERO);
    let record = SubstateDownProofRecord {
        state_version,
        value_hash: TreeHash::new([2; 32]),
        leaf_proof: proof_ext(),
        shard_root: TreeHash::new([3; 32]),
        shard_root_proof: proof_ext(),
    };
    store
        .with_write_tx(|tx| tx.substate_down_proofs_insert(epoch, shard, &id, &record))
        .unwrap();
}

fn holds_proof(store: &RocksDbStateStore<String>, shard: Shard, state_version: Version) -> bool {
    store
        .with_read_tx(|tx| tx.state_version_proofs_get_range(shard, state_version, state_version))
        .unwrap()
        .into_iter()
        .any(|proof| proof.state_version == state_version)
}

fn holds_commit_proof(store: &RocksDbStateStore<String>, block_id: BlockId) -> bool {
    store
        .with_read_tx(|tx| tx.block_commit_proofs_get(&block_id).optional())
        .unwrap()
        .is_some()
}

fn prune_at(store: &RocksDbStateStore<String>, epoch: Epoch) {
    store
        .with_write_tx(|tx| tx.epoch_cleanup_step(epoch, EpochCleanupStep::StateVersionProofs, usize::MAX))
        .unwrap();
}

#[test]
fn a_proof_is_pruned_once_its_epoch_leaves_the_window_unless_it_is_a_proof_point() {
    let (store, _dir) = create_store();
    let proof_point = STATE_VERSION_PROOF_INTERVAL;
    commit_proof_of(&store, EPOCH, SHARD, 5, block(1));
    commit_proof_of(&store, EPOCH, OTHER_SHARD, proof_point, block(1));
    supersede(&store, EPOCH, SHARD, 5);
    supersede(&store, EPOCH, OTHER_SHARD, proof_point);

    prune_at(&store, EPOCH + Epoch(HISTORY - 1));
    assert!(holds_proof(&store, SHARD, 5), "pruned inside the window");

    prune_at(&store, EPOCH + Epoch(HISTORY));
    assert!(!holds_proof(&store, SHARD, 5), "kept past the window");
    assert!(
        holds_proof(&store, OTHER_SHARD, proof_point),
        "a proof point was pruned"
    );
    assert!(
        holds_commit_proof(&store, block(1)),
        "the commit proof a proof point cites was pruned"
    );
}

#[test]
fn a_down_proof_record_keeps_the_proof_it_cites_until_the_record_is_pruned() {
    let (store, _dir) = create_store();
    let destroyed_in = EPOCH + Epoch(10);
    commit_proof_of(&store, EPOCH, SHARD, 5, block(1));
    supersede(&store, EPOCH, SHARD, 5);
    cite(&store, destroyed_in, SHARD, 5, 1);

    prune_at(&store, EPOCH + Epoch(HISTORY));
    assert!(holds_proof(&store, SHARD, 5), "pruned while a live record cites it");

    let id = VersionedSubstateId::new(substate_id_seed(1), SubstateVersion::ZERO);
    store
        .with_write_tx(|tx| tx.epoch_cleanup(destroyed_in + Epoch(HISTORY)))
        .unwrap();
    assert!(
        store
            .with_read_tx(|tx| tx.substate_down_proofs_get(SHARD, &id))
            .unwrap()
            .is_none()
    );
    assert!(!holds_proof(&store, SHARD, 5), "kept past the citing record");
}

#[test]
fn a_commit_proof_is_pruned_with_the_last_proof_that_cites_it() {
    let (store, _dir) = create_store();
    commit_proof_of(&store, EPOCH, SHARD, 5, block(1));
    commit_proof_of(&store, EPOCH, OTHER_SHARD, 7, block(1));
    supersede(&store, EPOCH, SHARD, 5);
    supersede(&store, EPOCH, OTHER_SHARD, 7);
    cite(&store, EPOCH + Epoch(1), SHARD, 5, 1);

    prune_at(&store, EPOCH + Epoch(HISTORY));
    assert!(!holds_proof(&store, OTHER_SHARD, 7));
    assert!(holds_commit_proof(&store, block(1)), "pruned while a proof cites it");

    prune_at(&store, EPOCH + Epoch(1 + HISTORY));
    assert!(!holds_proof(&store, SHARD, 5));
    assert!(!holds_commit_proof(&store, block(1)), "kept once no proof cites it");
}

#[test]
fn a_rewound_proof_releases_its_retention_and_commit_proof() {
    let (store, _dir) = create_store();
    commit_proof_of(&store, EPOCH, SHARD, 5, block(1));
    cite(&store, EPOCH + Epoch(10), SHARD, 5, 1);

    store
        .with_write_tx(|tx| tx.state_tree_truncate_to_version(SHARD, 4))
        .unwrap();
    assert!(!holds_proof(&store, SHARD, 5));
    assert!(
        !holds_commit_proof(&store, block(1)),
        "kept once its only proof was rewound"
    );

    // The version is committed again, by another block. The rewound proof's retention does not carry over to it.
    commit_proof_of(&store, EPOCH, SHARD, 5, block(2));
    supersede(&store, EPOCH, SHARD, 5);
    prune_at(&store, EPOCH + Epoch(HISTORY));
    assert!(
        !holds_proof(&store, SHARD, 5),
        "kept through the rewound proof's retention"
    );
    assert!(!holds_commit_proof(&store, block(2)));
}

#[test]
fn a_shards_latest_proof_is_kept_until_a_later_proof_supersedes_it() {
    let (store, _dir) = create_store();
    let superseded_in = EPOCH + Epoch(10);
    commit_proof_of(&store, EPOCH, SHARD, 5, block(1));

    prune_at(&store, EPOCH + Epoch(HISTORY));
    assert!(holds_proof(&store, SHARD, 5), "pruned while it is the shard's latest");
    assert!(holds_commit_proof(&store, block(1)));

    supersede(&store, superseded_in, SHARD, 5);
    prune_at(&store, superseded_in + Epoch(HISTORY - 1));
    assert!(
        holds_proof(&store, SHARD, 5),
        "pruned inside the window of its supersession"
    );

    prune_at(&store, superseded_in + Epoch(HISTORY));
    assert!(
        !holds_proof(&store, SHARD, 5),
        "kept past the window of its supersession"
    );
    assert!(!holds_commit_proof(&store, block(1)));
    assert!(holds_proof(&store, SHARD, 6), "the shard's latest proof was pruned");
}
