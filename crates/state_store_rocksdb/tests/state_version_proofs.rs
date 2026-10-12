//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{create_rocksdb_with_opts, num_preshards, substate_id_seed};
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
/// The down proof retention window, in epochs.
const DOWN_PROOF_HISTORY: u64 = 3;
/// The version window, in versions.
const STATE_HISTORY: u64 = 4;
const SHARD: Shard = Shard::from_u32(1);
const OTHER_SHARD: Shard = Shard::from_u32(2);

fn create_store() -> (RocksDbStateStore<String>, TempDir) {
    create_rocksdb_with_opts(
        DatabaseOptions::default()
            .with_down_proof_history_length(DOWN_PROOF_HISTORY)
            .with_state_history_length(STATE_HISTORY),
    )
}

fn block(seed: u8) -> BlockId {
    BlockId::from([seed; 32])
}

fn proof_ext() -> SparseMerkleProofExt {
    let key = LeafKey::new(TreeHash::new([1; 32]));
    let tree = KeyedProofTree::build([(key, TreeHash::new([1; 32]))]).unwrap();
    tree.get_proof(&key).unwrap().1
}

/// Moves `shard`'s tree from `from` to `to`, recording each version's stale nodes as a commit does.
fn advance(store: &RocksDbStateStore<String>, shard: Shard, from: Version, to: Version) {
    store
        .with_write_tx(|tx| {
            for version in from + 1..=to {
                tx.state_tree_nodes_record_stale_tree_nodes(shard, version, &[])?;
            }
            tx.state_tree_shard_versions_set(shard, to)
        })
        .unwrap();
}

/// Commits `shard` at `state_version` in `block_id`, storing the version's proof and the block's commit proof.
fn commit_proof_of(store: &RocksDbStateStore<String>, shard: Shard, state_version: Version, block_id: BlockId) {
    advance(store, shard, state_version - 1, state_version);
    store
        .with_write_tx(|tx| {
            tx.state_version_proofs_insert(&StateVersionProof {
                shard,
                state_version,
                source: StateVersionProofSource::Committed { block_id },
                shard_root_proof: proof_ext(),
            })?;
            tx.block_commit_proofs_insert(&block_id, b"commit proof")
        })
        .unwrap();
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

fn clear_stale(store: &RocksDbStateStore<String>) {
    store
        .with_write_tx(|tx| tx.state_tree_nodes_clear_stale(num_preshards(), usize::MAX))
        .unwrap();
}

#[test]
fn a_proof_is_pruned_once_its_version_leaves_the_window_unless_it_is_a_proof_point() {
    let (store, _dir) = create_store();
    let proof_point = STATE_VERSION_PROOF_INTERVAL;
    commit_proof_of(&store, SHARD, 5, block(1));
    commit_proof_of(&store, OTHER_SHARD, proof_point, block(1));

    advance(&store, SHARD, 5, 5 + STATE_HISTORY);
    advance(&store, OTHER_SHARD, proof_point, proof_point + 1 + STATE_HISTORY);
    clear_stale(&store);
    assert!(holds_proof(&store, SHARD, 5), "pruned inside the version window");

    advance(&store, SHARD, 5 + STATE_HISTORY, 6 + STATE_HISTORY);
    clear_stale(&store);
    assert!(!holds_proof(&store, SHARD, 5), "kept past the version window");
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
fn an_idle_shards_latest_proof_is_kept() {
    let (store, _dir) = create_store();
    commit_proof_of(&store, SHARD, 5, block(1));

    prune_at(&store, EPOCH + Epoch(100));
    clear_stale(&store);
    assert!(holds_proof(&store, SHARD, 5), "the shard's latest proof was pruned");
    assert!(holds_commit_proof(&store, block(1)));
}

#[test]
fn a_commit_proof_is_pruned_with_the_last_proof_that_cites_it() {
    let (store, _dir) = create_store();
    commit_proof_of(&store, SHARD, 5, block(1));
    commit_proof_of(&store, OTHER_SHARD, 7, block(1));

    advance(&store, SHARD, 5, 6 + STATE_HISTORY);
    clear_stale(&store);
    assert!(!holds_proof(&store, SHARD, 5));
    assert!(holds_commit_proof(&store, block(1)), "pruned while a proof cites it");

    advance(&store, OTHER_SHARD, 7, 8 + STATE_HISTORY);
    clear_stale(&store);
    assert!(!holds_proof(&store, OTHER_SHARD, 7));
    assert!(!holds_commit_proof(&store, block(1)), "kept once no proof cites it");
}

#[test]
fn a_cited_proof_below_the_window_is_kept_until_its_citation_expires() {
    let (store, _dir) = create_store();
    let destroyed_in = EPOCH + Epoch(10);
    commit_proof_of(&store, SHARD, 5, block(1));
    cite(&store, destroyed_in, SHARD, 5, 1);

    advance(&store, SHARD, 5, 6 + STATE_HISTORY);
    clear_stale(&store);
    assert!(holds_proof(&store, SHARD, 5), "pruned while a live record cites it");

    prune_at(&store, destroyed_in + Epoch(DOWN_PROOF_HISTORY - 1));
    assert!(holds_proof(&store, SHARD, 5), "pruned inside its citation's window");

    let id = VersionedSubstateId::new(substate_id_seed(1), SubstateVersion::ZERO);
    store
        .with_write_tx(|tx| tx.epoch_cleanup(destroyed_in + Epoch(DOWN_PROOF_HISTORY)))
        .unwrap();
    assert!(
        store
            .with_read_tx(|tx| tx.substate_down_proofs_get(SHARD, &id))
            .unwrap()
            .is_none()
    );
    assert!(!holds_proof(&store, SHARD, 5), "kept past the citing record");
    assert!(!holds_commit_proof(&store, block(1)));
}

#[test]
fn a_cited_proof_inside_the_window_is_kept_when_its_citation_expires() {
    let (store, _dir) = create_store();
    commit_proof_of(&store, SHARD, 5, block(1));
    cite(&store, EPOCH, SHARD, 5, 1);

    prune_at(&store, EPOCH + Epoch(DOWN_PROOF_HISTORY));
    assert!(holds_proof(&store, SHARD, 5), "pruned inside the version window");

    advance(&store, SHARD, 5, 6 + STATE_HISTORY);
    clear_stale(&store);
    assert!(!holds_proof(&store, SHARD, 5), "kept past the version window");
    assert!(!holds_commit_proof(&store, block(1)));
}

#[test]
fn a_rewound_proof_releases_its_retention_and_commit_proof() {
    let (store, _dir) = create_store();
    let destroyed_in = EPOCH + Epoch(10);
    commit_proof_of(&store, SHARD, 5, block(1));
    cite(&store, destroyed_in, SHARD, 5, 1);

    store
        .with_write_tx(|tx| tx.state_tree_truncate_to_version(SHARD, 4))
        .unwrap();
    assert!(!holds_proof(&store, SHARD, 5));
    assert!(
        !holds_commit_proof(&store, block(1)),
        "kept once its only proof was rewound"
    );

    // The version is committed again, by another block. The rewound proof's citation does not carry over to it.
    commit_proof_of(&store, SHARD, 5, block(2));
    advance(&store, SHARD, 5, 6 + STATE_HISTORY);
    clear_stale(&store);
    assert!(
        !holds_proof(&store, SHARD, 5),
        "kept through the rewound proof's citation"
    );
    assert!(!holds_commit_proof(&store, block(2)));
}

#[test]
fn a_proof_that_cites_no_block_is_kept_past_the_window() {
    let (store, _dir) = create_store();
    advance(&store, SHARD, 4, 5);
    store
        .with_write_tx(|tx| {
            tx.state_version_proofs_insert(&StateVersionProof {
                shard: SHARD,
                state_version: 5,
                source: StateVersionProofSource::Received {
                    commit_proof: b"commit proof".to_vec(),
                },
                shard_root_proof: proof_ext(),
            })
        })
        .unwrap();

    advance(&store, SHARD, 5, 6 + STATE_HISTORY);
    clear_stale(&store);
    assert!(holds_proof(&store, SHARD, 5), "pruned without a citation to release");
}
