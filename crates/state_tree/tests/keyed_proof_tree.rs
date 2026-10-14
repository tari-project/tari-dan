//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_state_tree::{
    JmtHashScheme,
    KeyedProofTree,
    LeafKey,
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    StateTreeError,
    TreeHash,
};

fn leaf(i: u8) -> (LeafKey, TreeHash) {
    (
        LeafKey::new(TreeHash::new([i; 32])),
        TreeHash::new([i.wrapping_add(100); 32]),
    )
}

#[test]
fn an_empty_tree_has_the_placeholder_root_and_proves_absence() {
    let tree = KeyedProofTree::build([]).unwrap();
    assert_eq!(tree.root(), SPARSE_MERKLE_PLACEHOLDER_HASH);

    let (key, _) = leaf(1);
    let (value, proof) = tree.get_proof(&key).unwrap();
    assert!(value.is_none());
    proof
        .verify_exclusion_or_empty_tree(JmtHashScheme::V1, &tree.root(), &key)
        .unwrap();
}

#[test]
fn every_leaf_proves_against_the_root_and_other_keys_prove_absent() {
    let leaves = (1..=5).map(leaf).collect::<Vec<_>>();
    let tree = KeyedProofTree::build(leaves.clone()).unwrap();

    for (key, value) in &leaves {
        let (found, proof) = tree.get_proof(key).unwrap();
        assert!(found.is_some());
        proof
            .verify_inclusion(JmtHashScheme::V1, &tree.root(), key, value)
            .unwrap();
        proof
            .verify_inclusion(JmtHashScheme::V1, &tree.root(), key, &TreeHash::new([0; 32]))
            .unwrap_err();
    }

    let (absent, _) = leaf(9);
    let (found, proof) = tree.get_proof(&absent).unwrap();
    assert!(found.is_none());
    proof
        .verify_exclusion(JmtHashScheme::V1, &tree.root(), &absent)
        .unwrap();
}

#[test]
fn the_root_does_not_depend_on_leaf_order() {
    let forward = KeyedProofTree::build((1..=5).map(leaf)).unwrap();
    let reverse = KeyedProofTree::build((1..=5).rev().map(leaf)).unwrap();
    assert_eq!(forward.root(), reverse.root());
}

#[test]
fn it_rejects_two_leaves_with_the_same_key() {
    let (key, value) = leaf(1);
    let Err(err) = KeyedProofTree::build([(key, value), (key, TreeHash::new([7; 32]))]) else {
        panic!("a duplicate key must be rejected");
    };
    assert!(matches!(err, StateTreeError::DuplicateLeafKey { .. }), "{err}");
}
