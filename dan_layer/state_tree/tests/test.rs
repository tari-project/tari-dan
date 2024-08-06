//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
// Adapted from https://github.com/radixdlt/radixdlt-scrypto/blob/868ba44ec3b806992864af27c706968c797eb961/radix-engine-stores/src/hash_tree/test.rs

use std::collections::HashSet;

use itertools::Itertools;
use tari_state_tree::{memory_store::MemoryTreeStore, StaleTreeNode, Version, SPARSE_MERKLE_PLACEHOLDER_HASH};

use crate::support::{change, HashTreeTester};
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
fn hash_allows_putting_in_same_version() {
    let mut tester_1 = HashTreeTester::new_empty();
    tester_1.put_changes_at_version(None, 1, vec![change(1, Some(30))]);
    tester_1.put_changes_at_version(Some(1), 1, vec![change(2, Some(31))]);
    tester_1.put_changes_at_version(Some(1), 1, vec![change(3, Some(32))]);
    tester_1.put_changes_at_version(Some(1), 1, vec![change(4, Some(33))]);
    tester_1.put_changes_at_version(Some(1), 1, vec![change(5, Some(34))]);
    let hash_1 = tester_1.put_changes_at_version(Some(1), 1, vec![change(6, Some(35))]);
    let mut tester_2 = HashTreeTester::new_empty();
    tester_2.put_changes_at_version(None, 1, vec![
        change(1, Some(30)),
        change(2, Some(31)),
        change(3, Some(32)),
    ]);
    let hash_2 = tester_2.put_changes_at_version(Some(1), 2, vec![
        change(4, Some(33)),
        change(5, Some(34)),
        change(6, Some(35)),
    ]);
    assert_eq!(hash_1, hash_2);
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
        .unique()
        .sorted()
        .collect::<Vec<Version>>();
    assert_eq!(stale_versions, vec![1, 2]);
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
