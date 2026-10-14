//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_state_tree::{
    Child,
    InternalNode,
    JellyfishMerkleTree,
    JmtHashScheme,
    LeafKey,
    LeafNode,
    Nibble,
    NibblePath,
    Node,
    NodeKey,
    NodeType,
    StaleTreeNode,
    StateHashTreeDiff,
    TreeHash,
    memory_store::MemoryTreeStore,
};

fn tree_diff() -> StateHashTreeDiff<u64> {
    let store = MemoryTreeStore::<u64>::new();
    let jmt = JellyfishMerkleTree::new(&store, JmtHashScheme::V1);
    let changes = (0..500u64).map(|i| {
        let mut key = [0u8; 32];
        key[..8].copy_from_slice(&i.wrapping_mul(0x9E37_79B9_7F4A_7C15).to_be_bytes());
        (LeafKey::new(TreeHash::new(key)), Some((TreeHash::new([7; 32]), i)))
    });
    let (_, batch) = jmt.batch_put_value_set(changes, None, 1).unwrap();
    let mut diff = StateHashTreeDiff::from(batch);
    diff.new_nodes.push((NodeKey::new_empty_path(2), Node::Null));
    let odd_path = NibblePath::new_odd(&[0xa0]).unwrap();
    diff.stale_tree_nodes
        .push(StaleTreeNode::Node(NodeKey::new(3, odd_path)));
    diff.stale_tree_nodes
        .push(StaleTreeNode::Subtree(NodeKey::new(4, odd_path)));
    diff
}

fn hex<T: minicbor::Encode<()>>(value: T) -> String {
    minicbor::to_vec(value)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn a_tree_diff_round_trips_at_its_encoded_length() {
    let diff = tree_diff();
    assert!(diff.new_nodes.iter().any(|(_, n)| matches!(n, Node::Internal(_))));
    assert!(diff.new_nodes.iter().any(|(_, n)| matches!(n, Node::Leaf(_))));
    let encoded = tari_bor::encode(&diff).unwrap();
    assert_eq!(minicbor::len(&diff), encoded.len());
    let decoded: StateHashTreeDiff<u64> = tari_bor::decode_exact(&encoded).unwrap();
    assert_eq!(decoded.new_nodes, diff.new_nodes);
    assert_eq!(decoded.stale_tree_nodes, diff.stale_tree_nodes);
}

/// Stored tree nodes and stale node lists are in this encoding, so it must not change without a migration.
#[test]
fn stored_tree_nodes_keep_their_encoding() {
    let children = [
        (
            0xf,
            Child::try_new(TreeHash::new([3; 32]), 70_000, NodeType::Leaf).unwrap(),
        ),
        (0x0, Child::try_new(TreeHash::new([1; 32]), 5, NodeType::Leaf).unwrap()),
        (
            0x3,
            Child::try_new(TreeHash::new([2; 32]), 300, NodeType::Internal { leaf_count: 7 }).unwrap(),
        ),
    ]
    .into_iter()
    .map(|(nibble, child)| (Nibble::try_from(nibble).unwrap(), child))
    .collect();
    let internal = Node::<u64>::Internal(InternalNode::try_new(children).unwrap());
    let leaf = Node::Leaf(LeafNode::new(
        LeafKey::new(TreeHash::new([4; 32])),
        TreeHash::new([5; 32]),
        1234u64,
        42,
    ));
    let even = NodeKey::new(9, NibblePath::new_even(&[0x12, 0x34]).unwrap());
    let odd = NodeKey::new(9, NibblePath::new_odd(&[0x50]).unwrap());
    assert_eq!(
        hex(&internal),
        "8200a3008358200101010101010101010101010101010101010101010101010101010101010101058100038358200202020202020202\
         02020202020202020202020202020202020202020202020219012c8202070f8358200303030303030303030303030303030303030303\
         0303030303030303030303031a000111708100"
    );
    assert_eq!(
        hex(&leaf),
        "8201845820040404040404040404040404040404040404040404040404040404040404040458200505050505050505050505050505\
         0505050505050505050505050505050505051904d2182a"
    );
    assert_eq!(hex(Node::<u64>::Null), "8102");
    assert_eq!(hex(&even), "82098204421234");
    assert_eq!(hex(&odd), "820982014150");
    assert_eq!(hex(StaleTreeNode::Node(even)), "820082098204421234");
    assert_eq!(hex(StaleTreeNode::Subtree(odd)), "8201820982014150");
}
