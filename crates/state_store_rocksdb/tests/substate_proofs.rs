//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use std::collections::{BTreeMap, HashSet};

use helpers::{NETWORK, build_substate_record, create_rocksdb, create_substate_update_batch, num_preshards};
use tari_ootle_common_types::{Epoch, ShardGroup, VersionedSubstateId, shard::Shard};
use tari_ootle_storage::{
    ShardScopedTreeStoreReader,
    ShardScopedTreeStoreWriter,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    SubstateProofGenerator,
    consensus_models::{Block, SubstateRecord},
    generate_substate_proof,
};
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    SpreadPrefixStateTree,
    SubstateTreeChange,
    TreeHash,
    compute_merkle_root_for_hashes,
};

use crate::helpers::substate_id_seed;

/// The state-tree version every shard in these tests is committed at.
const TREE_VERSION: u64 = 1;

/// Commits `substates` to the store and to their shards' state trees, as a validator does when a
/// block commits.
fn commit_substates(db: &impl StateStore, substates: &[SubstateRecord]) {
    let mut by_shard: BTreeMap<Shard, Vec<&SubstateRecord>> = BTreeMap::new();
    for substate in substates {
        by_shard.entry(substate.created().in_shard).or_default().push(substate);
    }

    let mut tx = db.create_write_tx().unwrap();
    Block::zero_block(NETWORK, num_preshards()).insert(&mut tx).unwrap();

    for (shard, substates) in &by_shard {
        let changes = substates.iter().map(|s| SubstateTreeChange::Up {
            id: s.to_versioned_substate_id(),
            value_hash: *s.state_hash(),
        });
        {
            let mut store = ShardScopedTreeStoreWriter::new(&mut tx, *shard);
            SpreadPrefixStateTree::new(&mut store)
                .batch_put_substate_changes(None, TREE_VERSION, changes)
                .unwrap();
        }
        tx.state_tree_shard_versions_set(*shard, TREE_VERSION).unwrap();
    }

    tx.substates_commit_batch(create_substate_update_batch(Epoch::zero(), substates))
        .unwrap();
    tx.commit().unwrap();
}

/// The shard-group state merkle root a block header commits: the root of the tree over the shard
/// group's per-shard roots, in the canonical `[global, shard_0, ...]` order.
fn shard_group_root(tx: &impl StateStoreReadTransaction, shard_group: ShardGroup) -> TreeHash {
    let roots = shard_group.shard_iter_with_global().map(|shard| {
        let Some(version) = tx.state_tree_versions_get_latest(shard).unwrap() else {
            return SPARSE_MERKLE_PLACEHOLDER_HASH;
        };
        let mut store = ShardScopedTreeStoreReader::new(tx, shard);
        SpreadPrefixStateTree::new(&mut store).get_root_hash(version).unwrap()
    });
    compute_merkle_root_for_hashes(roots.collect::<Vec<_>>()).unwrap()
}

/// The leaf value hash a verifier re-derives from the substate value to bind it to the committed leaf.
fn value_hash(substate: &SubstateRecord) -> TreeHash {
    TreeHash::new((*substate.state_hash()).into_array())
}

/// Substates whose ids land in at least `min_shards` distinct shards, so that a batch over them
/// exercises both a fresh and a reused shard-root proof.
fn substates_spanning_shards(count: u32, min_shards: usize) -> Vec<SubstateRecord> {
    // A substate's shard is read off the leading byte of its entity id, and `substate_id_seed` writes
    // the seed there big-endian, so the seed has to vary in its top byte to move between shards.
    let substates = (0..count)
        .map(|seed| build_substate_record(&substate_id_seed(seed << 24), 0, TREE_VERSION))
        .collect::<Vec<_>>();
    let shards = substates.iter().map(|s| s.created().in_shard).collect::<HashSet<_>>();
    assert!(
        shards.len() >= min_shards,
        "{count} substates landed in {} shard(s), need {min_shards}",
        shards.len()
    );
    substates
}

#[test]
fn proofs_for_a_batch_verify_against_one_shard_group_root() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(8, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let group_root = shard_group_root(&tx, shard_group);
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards()).unwrap();

    for substate in &substates {
        let versioned_id = substate.to_versioned_substate_id();
        let proof = generator.generate(&versioned_id).unwrap();
        proof
            .verify_inclusion(&group_root, &versioned_id, &value_hash(substate))
            .unwrap_or_else(|e| panic!("{versioned_id} in {}: {e}", substate.created().in_shard));
    }
}

/// The level-2 proof is cached per shard, so a batch that revisits a shard must not be served the
/// proof of a different shard's root.
#[test]
fn a_reused_shard_root_proof_belongs_to_its_own_shard() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(8, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards()).unwrap();

    // Prove every substate once to fill the cache, then again to read it back.
    let first = substates
        .iter()
        .map(|s| generator.generate(&s.to_versioned_substate_id()).unwrap())
        .collect::<Vec<_>>();
    for (substate, first) in substates.iter().zip(first) {
        let again = generator.generate(&substate.to_versioned_substate_id()).unwrap();
        assert_eq!(
            tari_bor::serde_codec::to_vec(&again).unwrap(),
            tari_bor::serde_codec::to_vec(&first).unwrap(),
            "{} in {}",
            substate.substate_id(),
            substate.created().in_shard
        );
    }
}

#[test]
fn a_batched_proof_matches_the_single_substate_proof() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(4, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards()).unwrap();

    for substate in &substates {
        let versioned_id = substate.to_versioned_substate_id();
        let batched = generator.generate(&versioned_id).unwrap();
        let single = generate_substate_proof(&tx, shard_group, &versioned_id, num_preshards()).unwrap();
        assert_eq!(
            tari_bor::serde_codec::to_vec(&batched).unwrap(),
            tari_bor::serde_codec::to_vec(&single).unwrap(),
            "{versioned_id}"
        );
    }
}

/// A version that is not up gets an exclusion proof - the shape a down substate in a batch is
/// answered with.
#[test]
fn a_version_that_is_not_up_gets_an_exclusion_proof() {
    let (db, _tmp) = create_rocksdb();
    let shard_group = ShardGroup::all_shards(num_preshards());
    let substates = substates_spanning_shards(4, 2);
    commit_substates(&db, &substates);

    let tx = db.create_read_tx().unwrap();
    let group_root = shard_group_root(&tx, shard_group);
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards()).unwrap();

    for substate in &substates {
        let next_version = VersionedSubstateId::new(substate.substate_id().clone(), substate.version() + 1);
        let proof = generator.generate(&next_version).unwrap();
        proof.verify_exclusion(&group_root, &next_version).unwrap();
        proof
            .verify_inclusion(&group_root, &next_version, &value_hash(substate))
            .unwrap_err();
    }
}

#[test]
fn a_substate_outside_the_shard_group_is_refused() {
    let (db, _tmp) = create_rocksdb();
    let substates = substates_spanning_shards(8, 2);
    commit_substates(&db, &substates);

    // A shard group holding the first substate's shard but not every substate's.
    let first_shard = substates[0].created().in_shard;
    let shard_group = ShardGroup::new_checked(first_shard, first_shard).unwrap();
    let outsider = substates
        .iter()
        .find(|s| s.created().in_shard != first_shard)
        .expect("substates span more than one shard");

    let tx = db.create_read_tx().unwrap();
    let mut generator = SubstateProofGenerator::new(&tx, shard_group, num_preshards()).unwrap();

    generator
        .generate(&substates[0].to_versioned_substate_id())
        .expect("in the shard group");
    let err = generator
        .generate(&outsider.to_versioned_substate_id())
        .expect_err("outside the shard group");
    assert!(err.to_string().contains("outside this shard group"), "{err}");
}
