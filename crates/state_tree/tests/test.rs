//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
// Adapted from https://github.com/radixdlt/radixdlt-scrypto/blob/868ba44ec3b806992864af27c706968c797eb961/radix-engine-stores/src/hash_tree/test.rs

use std::collections::{BTreeSet, HashSet};

use tari_engine_types::ProtocolVersion;
use tari_jellyfish::{
    JmtHashScheme,
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    SparseMerkleProofExt,
    StaleTreeNode,
    TreeHash,
    Version,
};
use tari_ootle_common_types::{NumPreshards, ToSubstateAddress, VersionedSubstateId, shard::Shard};
use tari_state_tree::{
    ShardGroupRootTree,
    StateTreeError,
    compute_merkle_root_for_hashes,
    compute_shard_group_root,
    memory_store::MemoryTreeStore,
    shard_state_leaf,
};

use crate::support::{HashTreeTester, change, hash_value_from_seed, make_value};
mod support;

#[test]
fn hash_of_next_version_differs_when_value_changed() {
    let mut tester = HashTreeTester::new_empty();
    let hash_v1 = tester.put_substate_changes(vec![change(1, Some(30))]);
    let hash_v2 = tester.put_substate_changes(vec![change(1, Some(70))]);
    assert_ne!(hash_v1, hash_v2);
}

#[test]
fn hash_of_next_version_same_when_write_repeated() {
    let mut tester = HashTreeTester::new_empty();
    let hash_v1 = tester.put_substate_changes(vec![change(4, Some(30)), change(3, Some(40))]);
    let hash_v2 = tester.put_substate_changes(vec![change(4, Some(30))]);
    assert_eq!(hash_v1, hash_v2);
}

#[test]
fn hash_of_next_version_same_when_write_empty() {
    let mut tester = HashTreeTester::new_empty();
    let hash_v1 = tester.put_substate_changes(vec![change(1, Some(30)), change(3, Some(40))]);
    let hash_v2 = tester.put_substate_changes(vec![]);
    assert_eq!(hash_v1, hash_v2);
}

#[test]
fn hash_of_next_version_differs_when_entry_added() {
    let mut tester = HashTreeTester::new_empty();
    let hash_v1 = tester.put_substate_changes(vec![change(1, Some(30))]);
    let hash_v2 = tester.put_substate_changes(vec![change(2, Some(30))]);
    assert_ne!(hash_v1, hash_v2);
}

#[test]
fn hash_of_next_version_differs_when_entry_removed() {
    let mut tester = HashTreeTester::new_empty();
    let hash_v1 = tester.put_substate_changes(vec![change(1, Some(30)), change(4, Some(20))]);
    let hash_v2 = tester.put_substate_changes(vec![change(1, None)]);
    assert_ne!(hash_v1, hash_v2);
}

#[test]
fn hash_returns_to_same_when_previous_state_restored() {
    let mut tester = HashTreeTester::new_empty();
    let hash_v1 = tester.put_substate_changes(vec![change(1, Some(30)), change(2, Some(40))]);
    tester.put_substate_changes(vec![change(1, Some(90)), change(2, None), change(3, Some(10))]);
    let hash_v3 = tester.put_substate_changes(vec![change(1, Some(30)), change(2, Some(40)), change(3, None)]);
    assert_eq!(hash_v1, hash_v3);
}

#[test]
fn hash_computed_consistently_after_higher_tier_leafs_deleted() {
    // Compute a "reference" hash of state containing simply [2:3:4, 2:3:5].
    let mut reference_tester = HashTreeTester::new_empty();
    let reference_root = reference_tester.put_substate_changes(vec![change(1, Some(234)), change(2, Some(235))]);

    // Compute a hash of the same state, at which we arrive after deleting some unrelated NodeId.
    let mut tester = HashTreeTester::new_empty();
    tester.put_substate_changes(vec![change(3, Some(162)), change(4, Some(163)), change(1, Some(234))]);
    tester.put_substate_changes(vec![change(3, None), change(4, None)]);
    let root_after_deletes = tester.put_substate_changes(vec![change(2, Some(235))]);

    // We did [3,4,1] - [3,4] + [2] = [1,2] (i.e. same state).
    assert_eq!(root_after_deletes, reference_root);
}

#[test]
fn hash_computed_consistently_after_adding_higher_tier_sibling() {
    // Compute a "reference" hash of state containing simply [1,2,3].
    let mut reference_tester = HashTreeTester::new_empty();
    let reference_root =
        reference_tester.put_substate_changes(vec![change(1, Some(196)), change(2, Some(234)), change(3, Some(235))]);

    // Compute a hash of the same state, at which we arrive after adding some sibling NodeId.
    let mut tester = HashTreeTester::new_empty();
    tester.put_substate_changes(vec![change(2, Some(234))]);
    tester.put_substate_changes(vec![change(1, Some(196))]);
    let root_after_adding_sibling = tester.put_substate_changes(vec![change(3, Some(235))]);

    // We did [2] + [1] + [3] = [1,2,3] (i.e. same state).
    assert_eq!(root_after_adding_sibling, reference_root);
}

#[test]
fn hash_differs_when_states_only_differ_by_node_key() {
    let mut tester_1 = HashTreeTester::new_empty();
    let hash_1 = tester_1.put_substate_changes(vec![change(1, Some(30))]);
    let mut tester_2 = HashTreeTester::new_empty();
    let hash_2 = tester_2.put_substate_changes(vec![change(2, Some(30))]);
    assert_ne!(hash_1, hash_2);
}

#[test]
fn hash_differs_when_states_only_differ_by_value() {
    let mut tester_1 = HashTreeTester::new_empty();
    let hash_1 = tester_1.put_substate_changes(vec![change(1, Some(1))]);
    let mut tester_2 = HashTreeTester::new_empty();
    let hash_2 = tester_2.put_substate_changes(vec![change(1, Some(2))]);
    assert_ne!(hash_1, hash_2);
}

#[test]
fn supports_empty_state() {
    let mut tester = HashTreeTester::new_empty();
    let hash_v1 = tester.put_substate_changes(vec![]);
    assert_eq!(hash_v1, SPARSE_MERKLE_PLACEHOLDER_HASH);
    let hash_v2 = tester.put_substate_changes(vec![change(1, Some(30))]);
    assert_ne!(hash_v2, SPARSE_MERKLE_PLACEHOLDER_HASH);
    let hash_v3 = tester.put_substate_changes(vec![change(1, None)]);
    assert_eq!(hash_v3, SPARSE_MERKLE_PLACEHOLDER_HASH);
}

#[test]
fn records_stale_tree_node_keys() {
    let mut tester = HashTreeTester::new_empty();
    tester.put_substate_changes(vec![change(4, Some(30))]);
    tester.put_substate_changes(vec![change(3, Some(70))]);
    tester.put_substate_changes(vec![change(3, Some(80))]);
    let stale_versions = tester
        .tree_store
        .stale_nodes
        .iter()
        .map(|stale_part| {
            let StaleTreeNode::Node(key) = stale_part else {
                panic!("expected only single node removals");
            };
            key.version()
        })
        .collect::<BTreeSet<Version>>();
    assert_eq!(stale_versions.into_iter().collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn serialized_keys_are_strictly_increasing() {
    let mut tester = HashTreeTester::new(MemoryTreeStore::new(), None);
    tester.put_substate_changes(vec![change(3, Some(90))]);
    let previous_keys = tester.tree_store.nodes.keys().cloned().collect::<HashSet<_>>();
    tester.put_substate_changes(vec![change(1, Some(80))]);
    let min_next_key = tester
        .tree_store
        .nodes
        .keys()
        .filter(|key| !previous_keys.contains(*key))
        .max()
        .unwrap();
    let max_previous_key = previous_keys.iter().max().unwrap();
    assert!(min_next_key > max_previous_key);
}

#[test]
fn proofs() {
    let mut tester = HashTreeTester::new_empty();
    let root_v1 = tester.put_substate_changes(vec![change(1, Some(30))]);
    tester.put_substate_changes(vec![change(2, Some(40))]);
    let root_hash = tester.put_substate_changes(vec![change(3, Some(50))]);

    let tree = tester.create_state_tree();
    let (key, proof_value, proof) = tree.get_proof(3, &make_value(1)).unwrap();
    let hash = hash_value_from_seed(30);
    assert_eq!(proof_value, Some((hash, make_value(1).to_substate_address(), 1)));
    proof
        .verify_inclusion(JmtHashScheme::V1, &root_hash, &key, &hash)
        .unwrap();
    let (key, proof_value, proof) = tree.get_proof(3, &make_value(2)).unwrap();
    let hash = hash_value_from_seed(40);
    assert_eq!(proof_value, Some((hash, make_value(2).to_substate_address(), 2)));
    proof
        .verify_inclusion(JmtHashScheme::V1, &root_hash, &key, &hash)
        .unwrap();
    let (key, proof_value, proof) = tree.get_proof(3, &make_value(3)).unwrap();
    let hash = hash_value_from_seed(50);
    assert_eq!(proof_value, Some((hash, make_value(3).to_substate_address(), 3)));
    proof
        .verify_inclusion(JmtHashScheme::V1, &root_hash, &key, &hash)
        .unwrap();
    let (key, proof_value, proof) = tree.get_proof(3, &make_value(3)).unwrap();
    proof
        .verify_inclusion(JmtHashScheme::V1, &root_hash, &key, &proof_value.unwrap().0)
        .unwrap();

    // Fail cases:
    // Fail to proof exclusion for included value
    let (key, _, proof) = tree.get_proof(3, &make_value(3)).unwrap();
    proof.verify_exclusion(JmtHashScheme::V1, &root_hash, &key).unwrap_err();
    // Fail to proof inclusion for excluded value
    let (key, _, proof) = tree.get_proof(3, &make_value(1)).unwrap();
    let hash = hash_value_from_seed(50);
    proof
        .verify_inclusion(JmtHashScheme::V1, &root_hash, &key, &hash)
        .unwrap_err();
    // Fail to proof inclusion for old/incorrect merkle root
    let (key, proof_value, proof) = tree.get_proof(3, &make_value(3)).unwrap();
    proof
        .verify_inclusion(JmtHashScheme::V1, &root_v1, &key, &proof_value.unwrap().0)
        .unwrap_err();

    // Exclusion proof
    let (key, proof_value, proof) = tree.get_proof(3, &make_value(4)).unwrap();
    assert!(proof_value.is_none());
    proof.verify_exclusion(JmtHashScheme::V1, &root_hash, &key).unwrap();

    // Fail to verify exclusion proof
    let (key, _, proof) = tree.get_proof(3, &make_value(4)).unwrap();
    let hash = hash_value_from_seed(50);
    proof
        .verify_inclusion(JmtHashScheme::V1, &root_hash, &key, &hash)
        .unwrap_err();
}

const NUM_PRESHARDS: NumPreshards = NumPreshards::P256;

/// A shard that is neither global nor `shard`.
fn sibling_of(shard: Shard) -> Shard {
    if shard == Shard::first() {
        Shard::from_u32(2)
    } else {
        Shard::first()
    }
}

// Builds the shard-group root tree the same way a validator does for a block header: an ephemeral
// tree over the global shard (empty), the shard `substate` lives in at `shard_root` and
// `state_version`, and an unrelated sibling shard.
fn shard_group_root_and_proof(
    protocol_version: ProtocolVersion,
    substate: &VersionedSubstateId,
    shard_root: TreeHash,
    state_version: Version,
) -> (TreeHash, SparseMerkleProofExt) {
    let shard = substate.to_shard(NUM_PRESHARDS);
    let tree = ShardGroupRootTree::build(protocol_version, [
        (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
        (shard, shard_root, state_version),
        (sibling_of(shard), hash_value_from_seed(99), 5),
    ])
    .unwrap();
    let (_, shard_root_proof) = tree.get_proof(shard).unwrap();
    (tree.root(), shard_root_proof)
}

#[test]
fn v0_shard_group_root_is_the_set_of_shard_roots() {
    let roots = [
        SPARSE_MERKLE_PLACEHOLDER_HASH,
        hash_value_from_seed(1),
        hash_value_from_seed(2),
    ];
    let shard_states = roots
        .iter()
        .enumerate()
        .map(|(i, root)| (Shard::from_u32(i as u32), *root, 7));
    assert_eq!(
        compute_shard_group_root(ProtocolVersion::V0, shard_states).unwrap(),
        compute_merkle_root_for_hashes(roots).unwrap()
    );
}

/// Esmeralda's history was committed under V1, so its roots must stay the set of V1 shard state leaves.
#[test]
fn v1_shard_group_root_is_the_set_of_shard_state_leaves() {
    let shard_states = [
        (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
        (Shard::from_u32(1), hash_value_from_seed(1), 3),
        (Shard::from_u32(2), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
        (Shard::from_u32(3), hash_value_from_seed(2), 7),
    ];
    let leaves = shard_states
        .iter()
        .map(|(shard, root, version)| shard_state_leaf(ProtocolVersion::V1, *shard, root, *version));
    assert_eq!(
        compute_shard_group_root(ProtocolVersion::V1, shard_states).unwrap(),
        compute_merkle_root_for_hashes(leaves).unwrap()
    );
}

#[test]
fn a_shard_listed_twice_is_rejected() {
    for protocol_version in [ProtocolVersion::V0, ProtocolVersion::V1, ProtocolVersion::V2] {
        let result = compute_shard_group_root(protocol_version, [
            (Shard::from_u32(1), hash_value_from_seed(1), 1),
            (Shard::from_u32(1), hash_value_from_seed(2), 2),
        ]);
        assert!(
            matches!(
                result,
                Err(StateTreeError::DuplicateShardInShardGroupTree { shard }) if shard == Shard::from_u32(1)
            ),
            "{protocol_version}: {result:?}"
        );
    }
}

#[test]
fn v2_shard_group_root_commits_to_the_shard_of_each_root() {
    let a = hash_value_from_seed(1);
    let b = hash_value_from_seed(2);
    let root = compute_shard_group_root(ProtocolVersion::V2, [
        (Shard::from_u32(1), a, 1),
        (Shard::from_u32(2), b, 1),
    ])
    .unwrap();
    let swapped = compute_shard_group_root(ProtocolVersion::V2, [
        (Shard::from_u32(1), b, 1),
        (Shard::from_u32(2), a, 1),
    ])
    .unwrap();
    assert_ne!(root, swapped);
}

#[test]
fn v2_empty_shard_has_no_leaf_in_the_shard_group_root() {
    let root = hash_value_from_seed(1);
    assert_eq!(
        compute_shard_group_root(ProtocolVersion::V2, [
            (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
            (Shard::first(), root, 1)
        ])
        .unwrap(),
        compute_shard_group_root(ProtocolVersion::V2, [(Shard::first(), root, 1)]).unwrap()
    );
    assert_eq!(
        compute_shard_group_root(ProtocolVersion::V2, [(
            Shard::global(),
            SPARSE_MERKLE_PLACEHOLDER_HASH,
            0
        )])
        .unwrap(),
        SPARSE_MERKLE_PLACEHOLDER_HASH
    );
    // A shard whose substates have all gone down has the empty-tree root at a later state version,
    // which its leaf commits to.
    assert_ne!(
        compute_shard_group_root(ProtocolVersion::V2, [(
            Shard::first(),
            SPARSE_MERKLE_PLACEHOLDER_HASH,
            3
        )])
        .unwrap(),
        SPARSE_MERKLE_PLACEHOLDER_HASH
    );
}

/// An empty shard's state is proved by the absence of its leaf from the shard-group root, and that
/// absence can only be claimed for a shard that has no state.
fn assert_substate_in_an_empty_shard_is_provably_absent(protocol_version: ProtocolVersion) {
    use tari_state_tree::{SpreadPrefixStateTree, StateTreePayload, SubstateValueProof, memory_store::MemoryTreeStore};

    let substate = make_value(2);
    let own_shard = substate.to_shard(NUM_PRESHARDS);
    let sibling_shard = sibling_of(own_shard);

    let mut empty = MemoryTreeStore::<StateTreePayload>::new();
    SpreadPrefixStateTree::new(&mut empty)
        .put_substate_changes(None, 1, vec![])
        .unwrap();
    let (_, _, empty_leaf_proof) = SpreadPrefixStateTree::new(&mut empty).get_proof(1, &substate).unwrap();

    let empty_group = ShardGroupRootTree::build(protocol_version, [
        (own_shard, SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
        (sibling_shard, hash_value_from_seed(99), 1),
    ])
    .unwrap();
    let (value, absent_proof) = empty_group.get_proof(own_shard).unwrap();
    assert!(value.is_none());
    SubstateValueProof::new(
        SPARSE_MERKLE_PLACEHOLDER_HASH,
        0,
        absent_proof,
        empty_leaf_proof.clone(),
    )
    .verify_exclusion(
        JmtHashScheme::V1,
        protocol_version,
        &empty_group.root(),
        NUM_PRESHARDS,
        &substate,
    )
    .unwrap();

    // Once the shard holds state, its leaf is present and no state cannot be claimed for it - even
    // when every substate in it has gone down.
    for own_state in [(hash_value_from_seed(1), 1), (SPARSE_MERKLE_PLACEHOLDER_HASH, 3)] {
        let populated_group = ShardGroupRootTree::build(protocol_version, [
            (own_shard, own_state.0, own_state.1),
            (sibling_shard, hash_value_from_seed(99), 1),
        ])
        .unwrap();
        let (_, present_proof) = populated_group.get_proof(own_shard).unwrap();
        SubstateValueProof::new(
            SPARSE_MERKLE_PLACEHOLDER_HASH,
            0,
            present_proof,
            empty_leaf_proof.clone(),
        )
        .verify_exclusion(
            JmtHashScheme::V1,
            protocol_version,
            &populated_group.root(),
            NUM_PRESHARDS,
            &substate,
        )
        .unwrap_err();
    }
}

#[test]
fn v2_substate_in_an_empty_shard_is_provably_absent() {
    assert_substate_in_an_empty_shard_is_provably_absent(ProtocolVersion::V2);
}

#[test]
fn two_level_substate_inclusion_proof() {
    use tari_state_tree::{SpreadPrefixStateTree, StateTreePayload, SubstateValueProof, memory_store::MemoryTreeStore};

    // Build a shard JMT with the production (SpreadPrefix) key mapper, as a real validator would.
    let mut store = MemoryTreeStore::<StateTreePayload>::new();
    SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(None, 1, vec![change(1, Some(30))])
        .unwrap();
    SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(Some(1), 2, vec![change(2, Some(40))])
        .unwrap();
    let shard_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(Some(2), 3, vec![change(3, Some(50))])
        .unwrap();

    // Level 1: leaf proof for substate make_value(2) against the shard root.
    let (_key, proof_value, leaf_proof) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(3, &make_value(2))
        .unwrap();
    let value_hash = proof_value.unwrap().0;

    // Level 2: shard root within the shard-group root.
    let (group_root, shard_root_proof) = shard_group_root_and_proof(ProtocolVersion::V2, &make_value(2), shard_root, 3);

    let proof = SubstateValueProof::new(shard_root, 3, shard_root_proof, leaf_proof);

    // Verifies against the trusted group root.
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V2,
            &group_root,
            NUM_PRESHARDS,
            &make_value(2),
            &value_hash,
        )
        .unwrap();
    // A tampered value hash is rejected (binds the value to the committed leaf).
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V2,
            &group_root,
            NUM_PRESHARDS,
            &make_value(2),
            &hash_value_from_seed(200),
        )
        .unwrap_err();
    // A wrong group root is rejected (level-2 failure).
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V2,
            &hash_value_from_seed(7),
            NUM_PRESHARDS,
            &make_value(2),
            &value_hash,
        )
        .unwrap_err();
    // An included substate cannot be proven absent.
    proof
        .verify_exclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V2,
            &group_root,
            NUM_PRESHARDS,
            &make_value(2),
        )
        .unwrap_err();
}

#[test]
fn two_level_substate_exclusion_proof() {
    use tari_state_tree::{SpreadPrefixStateTree, StateTreePayload, SubstateValueProof, memory_store::MemoryTreeStore};

    let mut store = MemoryTreeStore::<StateTreePayload>::new();
    SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(None, 1, vec![change(1, Some(30))])
        .unwrap();
    SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(Some(1), 2, vec![change(2, Some(40))])
        .unwrap();
    let shard_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(Some(2), 3, vec![change(3, Some(50))])
        .unwrap();

    // make_value(4) was never created in this shard - the leaf proof is an exclusion proof.
    let (_key, proof_value, leaf_proof) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(3, &make_value(4))
        .unwrap();
    assert!(proof_value.is_none());

    let (group_root, shard_root_proof) = shard_group_root_and_proof(ProtocolVersion::V2, &make_value(4), shard_root, 3);
    let proof = SubstateValueProof::new(shard_root, 3, shard_root_proof, leaf_proof);

    // The absent substate is provably absent under the trusted group root...
    proof
        .verify_exclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V2,
            &group_root,
            NUM_PRESHARDS,
            &make_value(4),
        )
        .unwrap();
    // ...but cannot be proven present.
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V2,
            &group_root,
            NUM_PRESHARDS,
            &make_value(4),
            &hash_value_from_seed(50),
        )
        .unwrap_err();
}

/// A substate is absent from every shard but its own, so only its own shard's root can prove it
/// absent - however genuine the proofs that place another root in the group root.
fn assert_exclusion_proof_against_another_shards_root_is_rejected(protocol_version: ProtocolVersion) {
    use tari_state_tree::{SpreadPrefixStateTree, StateTreePayload, SubstateValueProof, memory_store::MemoryTreeStore};

    let substate = make_value(2);
    let own_shard = substate.to_shard(NUM_PRESHARDS);
    let sibling_shard = sibling_of(own_shard);

    let mut own = MemoryTreeStore::<StateTreePayload>::new();
    let own_root = SpreadPrefixStateTree::new(&mut own)
        .put_substate_changes(None, 1, vec![change(1, Some(30)), change(2, Some(40))])
        .unwrap();
    let mut sibling = MemoryTreeStore::<StateTreePayload>::new();
    let sibling_root = SpreadPrefixStateTree::new(&mut sibling)
        .put_substate_changes(None, 1, vec![change(77, Some(11))])
        .unwrap();
    let (_, proof_value, sibling_leaf_proof) = SpreadPrefixStateTree::new(&mut sibling)
        .get_proof(1, &substate)
        .unwrap();
    assert!(proof_value.is_none());
    let mut empty = MemoryTreeStore::<StateTreePayload>::new();
    let empty_root = SpreadPrefixStateTree::new(&mut empty)
        .put_substate_changes(None, 1, vec![])
        .unwrap();
    assert_eq!(empty_root, SPARSE_MERKLE_PLACEHOLDER_HASH);
    let (_, _, empty_leaf_proof) = SpreadPrefixStateTree::new(&mut empty).get_proof(1, &substate).unwrap();

    let tree = ShardGroupRootTree::build(protocol_version, [
        (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
        (own_shard, own_root, 1),
        (sibling_shard, sibling_root, 1),
    ])
    .unwrap();
    let group_root = tree.root();
    let (_, own_root_proof) = tree.get_proof(own_shard).unwrap();
    let (_, sibling_root_proof) = tree.get_proof(sibling_shard).unwrap();
    let (_, global_root_proof) = tree.get_proof(Shard::global()).unwrap();

    // The substate is present in its own shard.
    let (_, proof_value, own_leaf_proof) = SpreadPrefixStateTree::new(&mut own).get_proof(1, &substate).unwrap();
    SubstateValueProof::new(own_root, 1, own_root_proof.clone(), own_leaf_proof)
        .verify_inclusion(
            JmtHashScheme::V1,
            protocol_version,
            &group_root,
            NUM_PRESHARDS,
            &substate,
            &proof_value.unwrap().0,
        )
        .unwrap();

    // A sibling shard's root, with the proof of its own place in the group root.
    SubstateValueProof::new(sibling_root, 1, sibling_root_proof, sibling_leaf_proof.clone())
        .verify_exclusion(
            JmtHashScheme::V1,
            protocol_version,
            &group_root,
            NUM_PRESHARDS,
            &substate,
        )
        .unwrap_err();
    // A sibling shard's root, with the proof of the substate's shard's place in the group root.
    SubstateValueProof::new(sibling_root, 1, own_root_proof, sibling_leaf_proof)
        .verify_exclusion(
            JmtHashScheme::V1,
            protocol_version,
            &group_root,
            NUM_PRESHARDS,
            &substate,
        )
        .unwrap_err();
    // The global shard's empty root.
    SubstateValueProof::new(SPARSE_MERKLE_PLACEHOLDER_HASH, 0, global_root_proof, empty_leaf_proof)
        .verify_exclusion(
            JmtHashScheme::V1,
            protocol_version,
            &group_root,
            NUM_PRESHARDS,
            &substate,
        )
        .unwrap_err();
}

#[test]
fn v2_exclusion_proof_against_another_shards_root_is_rejected() {
    assert_exclusion_proof_against_another_shards_root_is_rejected(ProtocolVersion::V2);
}

/// Before V2 a shard-group leaf is keyed by its value, not its shard, so a sibling shard's genuine root proves the
/// absence of a substate that is live in its own shard. Such exclusion proofs verify against any root committed
/// before V2, which is why a down proof whose exclusion root precedes V2 is not taken as proof of a Down.
#[test]
fn v1_exclusion_proof_against_another_shards_root_verifies() {
    use tari_state_tree::{SpreadPrefixStateTree, StateTreePayload, SubstateValueProof, memory_store::MemoryTreeStore};

    let substate = make_value(2);
    let own_shard = substate.to_shard(NUM_PRESHARDS);
    let sibling_shard = sibling_of(own_shard);

    let mut own = MemoryTreeStore::<StateTreePayload>::new();
    let own_root = SpreadPrefixStateTree::new(&mut own)
        .put_substate_changes(None, 1, vec![change(1, Some(30)), change(2, Some(40))])
        .unwrap();
    let mut sibling = MemoryTreeStore::<StateTreePayload>::new();
    let sibling_root = SpreadPrefixStateTree::new(&mut sibling)
        .put_substate_changes(None, 1, vec![change(77, Some(11))])
        .unwrap();
    let (_, _, sibling_leaf_proof) = SpreadPrefixStateTree::new(&mut sibling)
        .get_proof(1, &substate)
        .unwrap();

    let tree = ShardGroupRootTree::build(ProtocolVersion::V1, [
        (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
        (own_shard, own_root, 1),
        (sibling_shard, sibling_root, 1),
    ])
    .unwrap();
    let (_, sibling_root_proof) = tree.get_proof(sibling_shard).unwrap();
    SubstateValueProof::new(sibling_root, 1, sibling_root_proof, sibling_leaf_proof)
        .verify_exclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V1,
            &tree.root(),
            NUM_PRESHARDS,
            &substate,
        )
        .unwrap();
}

/// Before V2 two empty shards share a leaf, so the empty-tree proof of one shard's leaf proves the absence of a
/// substate that is live in another shard of the group.
#[test]
fn v1_exclusion_proof_against_another_empty_shard_verifies() {
    use tari_state_tree::{SpreadPrefixStateTree, StateTreePayload, SubstateValueProof, memory_store::MemoryTreeStore};

    let substate = make_value(2);
    let own_shard = substate.to_shard(NUM_PRESHARDS);
    let sibling_shard = sibling_of(own_shard);

    let mut own = MemoryTreeStore::<StateTreePayload>::new();
    let own_root = SpreadPrefixStateTree::new(&mut own)
        .put_substate_changes(None, 1, vec![change(1, Some(30)), change(2, Some(40))])
        .unwrap();
    let mut empty = MemoryTreeStore::<StateTreePayload>::new();
    SpreadPrefixStateTree::new(&mut empty)
        .put_substate_changes(None, 1, vec![])
        .unwrap();
    let (_, _, empty_leaf_proof) = SpreadPrefixStateTree::new(&mut empty).get_proof(1, &substate).unwrap();

    let tree = ShardGroupRootTree::build(ProtocolVersion::V1, [
        (own_shard, own_root, 1),
        (sibling_shard, SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
    ])
    .unwrap();
    let (_, sibling_root_proof) = tree.get_proof(sibling_shard).unwrap();
    SubstateValueProof::new(SPARSE_MERKLE_PLACEHOLDER_HASH, 0, sibling_root_proof, empty_leaf_proof)
        .verify_exclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V1,
            &tree.root(),
            NUM_PRESHARDS,
            &substate,
        )
        .unwrap();
}

#[test]
fn two_level_substate_inclusion_proof_for_genesis_version_zero() {
    use tari_state_tree::{SpreadPrefixStateTree, StateTreePayload, SubstateValueProof, memory_store::MemoryTreeStore};

    // Genesis substates are committed to the shard JMT at version 0 (mirrors `commit_genesis_substates`).
    let mut store = MemoryTreeStore::<StateTreePayload>::new();
    SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(None, 0, vec![change(1, Some(30)), change(2, Some(40))])
        .unwrap();
    // Consensus then builds version 1 on top; an untouched (read-only) genesis leaf must remain provable.
    let shard_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(Some(0), 1, vec![change(3, Some(50))])
        .unwrap();

    // Level 1: leaf proof for the genesis substate make_value(1) against the latest shard root.
    let (_key, proof_value, leaf_proof) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(1, &make_value(1))
        .unwrap();
    let value_hash = proof_value.unwrap().0;

    // Level 2: shard root within the shard-group root.
    let (group_root, shard_root_proof) = shard_group_root_and_proof(ProtocolVersion::V2, &make_value(1), shard_root, 3);
    let proof = SubstateValueProof::new(shard_root, 3, shard_root_proof, leaf_proof);

    // The genesis substate committed at version 0 is provably included under the trusted group root.
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V2,
            &group_root,
            NUM_PRESHARDS,
            &make_value(1),
            &value_hash,
        )
        .unwrap();
    // A tampered value hash is still rejected.
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V2,
            &group_root,
            NUM_PRESHARDS,
            &make_value(1),
            &hash_value_from_seed(200),
        )
        .unwrap_err();
}

#[test]
fn rewriting_an_already_written_version_is_rejected() {
    let mut tester = HashTreeTester::new_empty();
    tester.put_substate_changes(vec![change(1, Some(30))]);

    // Writing v1 a second time would overwrite the live nodes keyed at v1 while recording those same
    // keys as stale at v1, leaving the tree pointing at nodes the stale-node GC is free to delete.
    let err = tester
        .try_put_changes_at_version(Some(1), 1, vec![change(2, Some(40))])
        .unwrap_err();

    assert!(matches!(err, StateTreeError::NonMonotonicVersion {
        current_version: 1,
        next_version: 1
    }));
}

#[test]
fn writing_a_version_behind_the_current_one_is_rejected() {
    let mut tester = HashTreeTester::new_empty();
    tester.put_substate_changes(vec![change(1, Some(30))]);
    tester.put_substate_changes(vec![change(2, Some(40))]);

    let err = tester
        .try_put_changes_at_version(Some(2), 1, vec![change(3, Some(50))])
        .unwrap_err();

    assert!(matches!(err, StateTreeError::NonMonotonicVersion {
        current_version: 2,
        next_version: 1
    }));
}

#[test]
fn v1_shard_leaf_proof_binds_the_shard_state_version() {
    use tari_state_tree::{SpreadPrefixStateTree, StateTreePayload, SubstateValueProof, memory_store::MemoryTreeStore};

    let mut store = MemoryTreeStore::<StateTreePayload>::new();
    let shard_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(None, 1, vec![change(1, Some(30))])
        .unwrap();
    let (_key, proof_value, leaf_proof) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(1, &make_value(1))
        .unwrap();
    let value_hash = proof_value.unwrap().0;

    let (group_root, shard_root_proof) = shard_group_root_and_proof(ProtocolVersion::V1, &make_value(1), shard_root, 1);
    let proof = SubstateValueProof::new(shard_root, 1, shard_root_proof.clone(), leaf_proof.clone());
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V1,
            &group_root,
            NUM_PRESHARDS,
            &make_value(1),
            &value_hash,
        )
        .unwrap();
    // The same root claimed at another state version is not what the quorum signed.
    SubstateValueProof::new(shard_root, 2, shard_root_proof, leaf_proof.clone())
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V1,
            &group_root,
            NUM_PRESHARDS,
            &make_value(1),
            &value_hash,
        )
        .unwrap_err();
    // A V1 root does not verify under V0 leaves, nor the reverse.
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V0,
            &group_root,
            NUM_PRESHARDS,
            &make_value(1),
            &value_hash,
        )
        .unwrap_err();
    let (v0_root, v0_proof) = shard_group_root_and_proof(ProtocolVersion::V0, &make_value(1), shard_root, 1);
    let proof = SubstateValueProof::new(shard_root, 1, v0_proof, leaf_proof);
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V0,
            &v0_root,
            NUM_PRESHARDS,
            &make_value(1),
            &value_hash,
        )
        .unwrap();
    proof
        .verify_inclusion(
            JmtHashScheme::V1,
            ProtocolVersion::V1,
            &v0_root,
            NUM_PRESHARDS,
            &make_value(1),
            &value_hash,
        )
        .unwrap_err();
}
