//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Native minicbor encoding of the state tree's node types, in the format `tari_jellyfish`'s `minicbor` feature
//! defines (tari-project/tari#8101), until a `tari_jellyfish` release carries it. The modules follow minicbor's
//! `#[cbor(with = "...")]` convention.
//!
//! | Type | Encoding |
//! |------|----------|
//! | [`TreeHash`] | `bytes(32)` |
//! | [`NibblePath`] | `[num_nibbles, bytes]` |
//! | [`NodeKey`] | `[version, nibble_path]` |
//! | [`NodeType`] | `[0]` leaf, `[1]` null, `[2, leaf_count]` internal |
//! | [`Child`] | `[hash, version, node_type]` |
//! | [`InternalNode`] | `{nibble => child}` in ascending nibble order |
//! | [`LeafNode`] | `[leaf_key, value_hash, payload, version]` |
//! | [`Node`] | `[0, internal]`, `[1, leaf]`, `[2]` null |
//! | [`StaleTreeNode`] | `[0, node_key]` node, `[1, node_key]` subtree |
//!
//! These types are read only from the node's own database, so decoding checks no more than it needs to avoid a panic.
//! Never decode them from untrusted input.

use minicbor::{
    CborLen,
    Decode,
    Decoder,
    Encode,
    Encoder,
    decode,
    encode::{self, Write},
};
use tari_jellyfish::{Child, InternalNode, LeafKey, LeafNode, Nibble, NibblePath, Node, NodeKey, NodeType, TreeHash};

/// The encoded length of an array or map header, which is that of a `uint` of the same value.
pub(crate) fn header_len(len: u64) -> usize {
    CborLen::<()>::cbor_len(&len, &mut ())
}

fn sum<const N: usize>(parts: [usize; N]) -> usize {
    parts.into_iter().fold(0, usize::saturating_add)
}

fn unknown_variant(index: u32, pos: usize) -> decode::Error {
    decode::Error::unknown_variant(i64::from(index)).at(pos)
}

/// Reads a flat enum's array header and returns its variant index.
fn variant(d: &mut Decoder<'_>) -> Result<u32, decode::Error> {
    d.array()?;
    d.u32()
}

fn encode_tree_hash<W: Write>(hash: &TreeHash, e: &mut Encoder<W>) -> Result<(), encode::Error<W::Error>> {
    e.bytes(hash.as_slice())?;
    Ok(())
}

fn decode_tree_hash(d: &mut Decoder<'_>) -> Result<TreeHash, decode::Error> {
    let pos = d.position();
    TreeHash::try_from_bytes(d.bytes()?).map_err(|_| decode::Error::message("TreeHash must be 32 bytes").at(pos))
}

const TREE_HASH_LEN: usize = 34;

fn encode_nibble_path<C, W: Write>(
    path: &NibblePath,
    e: &mut Encoder<W>,
    ctx: &mut C,
) -> Result<(), encode::Error<W::Error>> {
    e.array(2)?;
    path.num_nibbles().encode(e, ctx)?;
    e.bytes(path.bytes())?;
    Ok(())
}

fn decode_nibble_path<C>(d: &mut Decoder<'_>, ctx: &mut C) -> Result<NibblePath, decode::Error> {
    d.array()?;
    let pos = d.position();
    let num_nibbles = usize::decode(d, ctx)?;
    let bytes = d.bytes()?;
    let num_bytes_nibbles = bytes.len().saturating_mul(2);
    let path = if num_nibbles == num_bytes_nibbles {
        NibblePath::new_even(bytes)
    } else if num_nibbles.checked_add(1) == Some(num_bytes_nibbles) && bytes.last().is_some_and(|b| b & 0x0F == 0) {
        NibblePath::new_odd(bytes)
    } else {
        return Err(decode::Error::message("NibblePath: nibble count does not match its bytes").at(pos));
    };
    path.map_err(|e| decode::Error::message(e.to_string()).at(pos))
}

fn nibble_path_len<C>(path: &NibblePath, ctx: &mut C) -> usize {
    let num_bytes = path.bytes().len();
    sum([
        header_len(2),
        path.num_nibbles().cbor_len(ctx),
        num_bytes.cbor_len(ctx),
        num_bytes,
    ])
}

pub mod node_key {
    use super::*;

    pub fn encode<C, W: Write>(key: &NodeKey, e: &mut Encoder<W>, ctx: &mut C) -> Result<(), encode::Error<W::Error>> {
        e.array(2)?;
        key.version().encode(e, ctx)?;
        encode_nibble_path(key.nibble_path(), e, ctx)
    }

    pub fn decode<C>(d: &mut Decoder<'_>, ctx: &mut C) -> Result<NodeKey, decode::Error> {
        d.array()?;
        let version = d.u64()?;
        let nibble_path = decode_nibble_path(d, ctx)?;
        Ok(NodeKey::new(version, nibble_path))
    }

    pub fn cbor_len<C>(key: &NodeKey, ctx: &mut C) -> usize {
        sum([
            header_len(2),
            key.version().cbor_len(ctx),
            nibble_path_len(key.nibble_path(), ctx),
        ])
    }
}

fn encode_node_type<C, W: Write>(
    node_type: &NodeType,
    e: &mut Encoder<W>,
    ctx: &mut C,
) -> Result<(), encode::Error<W::Error>> {
    match node_type {
        NodeType::Leaf => {
            e.array(1)?.u32(0)?;
        },
        NodeType::Null => {
            e.array(1)?.u32(1)?;
        },
        NodeType::Internal { leaf_count } => {
            e.array(2)?.u32(2)?;
            leaf_count.encode(e, ctx)?;
        },
    }
    Ok(())
}

fn decode_node_type<C>(d: &mut Decoder<'_>, ctx: &mut C) -> Result<NodeType, decode::Error> {
    let pos = d.position();
    match variant(d)? {
        0 => Ok(NodeType::Leaf),
        1 => Ok(NodeType::Null),
        2 => Ok(NodeType::Internal {
            leaf_count: usize::decode(d, ctx)?,
        }),
        i => Err(unknown_variant(i, pos)),
    }
}

fn node_type_len<C>(node_type: &NodeType, ctx: &mut C) -> usize {
    match node_type {
        // array header and a one-byte index
        NodeType::Leaf | NodeType::Null => 2,
        NodeType::Internal { leaf_count } => sum([2, leaf_count.cbor_len(ctx)]),
    }
}

fn encode_child<C, W: Write>(child: &Child, e: &mut Encoder<W>, ctx: &mut C) -> Result<(), encode::Error<W::Error>> {
    e.array(3)?;
    encode_tree_hash(&child.hash, e)?;
    child.version.encode(e, ctx)?;
    encode_node_type(&child.node_type, e, ctx)
}

fn decode_child<C>(d: &mut Decoder<'_>, ctx: &mut C) -> Result<Child, decode::Error> {
    d.array()?;
    let pos = d.position();
    let hash = decode_tree_hash(d)?;
    let version = d.u64()?;
    let node_type = decode_node_type(d, ctx)?;
    Child::try_new(hash, version, node_type).map_err(|e| decode::Error::message(e.to_string()).at(pos))
}

fn child_len<C>(child: &Child, ctx: &mut C) -> usize {
    sum([
        header_len(3),
        TREE_HASH_LEN,
        child.version.cbor_len(ctx),
        node_type_len(&child.node_type, ctx),
    ])
}

fn encode_internal_node<C, W: Write>(
    node: &InternalNode,
    e: &mut Encoder<W>,
    ctx: &mut C,
) -> Result<(), encode::Error<W::Error>> {
    // `children_sorted` yields children in ascending nibble order, so this map is too.
    e.map(node.children_sorted().count() as u64)?;
    for (nibble, child) in node.children_sorted() {
        e.u8(u8::from(nibble))?;
        encode_child(child, e, ctx)?;
    }
    Ok(())
}

fn decode_internal_node<C>(d: &mut Decoder<'_>, ctx: &mut C) -> Result<InternalNode, decode::Error> {
    let pos = d.position();
    let len = d
        .map()?
        .ok_or_else(|| decode::Error::message("InternalNode: expected a definite-length map").at(pos))?;
    let mut children = Vec::with_capacity(16);
    for _ in 0..len {
        let nibble_pos = d.position();
        let nibble =
            Nibble::try_from(d.u8()? & 0x0F).map_err(|e| decode::Error::message(e.to_string()).at(nibble_pos))?;
        children.push((nibble, decode_child(d, ctx)?));
    }
    InternalNode::try_new(children.into_iter().collect()).map_err(|e| decode::Error::message(e.to_string()).at(pos))
}

fn internal_node_len<C>(node: &InternalNode, ctx: &mut C) -> usize {
    node.children_sorted().fold(
        header_len(node.children_sorted().count() as u64),
        |acc, (nibble, child)| sum([acc, u8::from(nibble).cbor_len(ctx), child_len(child, ctx)]),
    )
}

fn encode_leaf_node<C, P: Encode<C>, W: Write>(
    leaf: &LeafNode<P>,
    e: &mut Encoder<W>,
    ctx: &mut C,
) -> Result<(), encode::Error<W::Error>> {
    e.array(4)?;
    encode_tree_hash(&leaf.leaf_key().bytes, e)?;
    encode_tree_hash(&leaf.value_hash(), e)?;
    leaf.payload().encode(e, ctx)?;
    leaf.version().encode(e, ctx)
}

fn decode_leaf_node<'b, C, P: Decode<'b, C>>(d: &mut Decoder<'b>, ctx: &mut C) -> Result<LeafNode<P>, decode::Error> {
    d.array()?;
    let leaf_key = LeafKey::new(decode_tree_hash(d)?);
    let value_hash = decode_tree_hash(d)?;
    let payload = P::decode(d, ctx)?;
    let version = d.u64()?;
    Ok(LeafNode::new(leaf_key, value_hash, payload, version))
}

fn leaf_node_len<C, P: CborLen<C>>(leaf: &LeafNode<P>, ctx: &mut C) -> usize {
    sum([
        header_len(4),
        TREE_HASH_LEN,
        TREE_HASH_LEN,
        leaf.payload().cbor_len(ctx),
        leaf.version().cbor_len(ctx),
    ])
}

pub mod node {
    use super::*;

    pub fn encode<C, P: Encode<C>, W: Write>(
        node: &Node<P>,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        match node {
            Node::Internal(internal) => {
                e.array(2)?.u32(0)?;
                encode_internal_node(internal, e, ctx)
            },
            Node::Leaf(leaf) => {
                e.array(2)?.u32(1)?;
                encode_leaf_node(leaf, e, ctx)
            },
            Node::Null => {
                e.array(1)?.u32(2)?;
                Ok(())
            },
        }
    }

    pub fn decode<'b, C, P: Decode<'b, C>>(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Node<P>, decode::Error> {
        let pos = d.position();
        match variant(d)? {
            0 => Ok(Node::Internal(decode_internal_node(d, ctx)?)),
            1 => Ok(Node::Leaf(decode_leaf_node(d, ctx)?)),
            2 => Ok(Node::Null),
            i => Err(unknown_variant(i, pos)),
        }
    }

    pub fn cbor_len<C, P: CborLen<C>>(node: &Node<P>, ctx: &mut C) -> usize {
        match node {
            // array header and a one-byte index
            Node::Internal(internal) => sum([2, internal_node_len(internal, ctx)]),
            Node::Leaf(leaf) => sum([2, leaf_node_len(leaf, ctx)]),
            Node::Null => 2,
        }
    }
}

pub mod stale_tree_node {
    use tari_jellyfish::StaleTreeNode;

    use super::*;

    pub fn encode<C, W: Write>(
        stale: &StaleTreeNode,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        let index = match stale {
            StaleTreeNode::Node(_) => 0,
            StaleTreeNode::Subtree(_) => 1,
        };
        e.array(2)?.u32(index)?;
        node_key::encode(stale.as_node_key(), e, ctx)
    }

    pub fn decode<C>(d: &mut Decoder<'_>, ctx: &mut C) -> Result<StaleTreeNode, decode::Error> {
        let pos = d.position();
        match variant(d)? {
            0 => Ok(StaleTreeNode::Node(node_key::decode(d, ctx)?)),
            1 => Ok(StaleTreeNode::Subtree(node_key::decode(d, ctx)?)),
            i => Err(unknown_variant(i, pos)),
        }
    }

    pub fn cbor_len<C>(stale: &StaleTreeNode, ctx: &mut C) -> usize {
        sum([2, node_key::cbor_len(stale.as_node_key(), ctx)])
    }
}

#[cfg(test)]
mod tests {
    use tari_jellyfish::{JellyfishMerkleTree, JmtHashScheme, StaleTreeNode};

    use super::*;
    use crate::{StateHashTreeDiff, memory_store::MemoryTreeStore};

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
            .push(StaleTreeNode::Node(NodeKey::new(3, odd_path.clone())));
        diff.stale_tree_nodes
            .push(StaleTreeNode::Subtree(NodeKey::new(4, odd_path)));
        diff
    }

    fn encode_node(node: &Node<u64>) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        node::encode(node, &mut e, &mut ()).unwrap();
        e.into_writer()
    }

    #[test]
    fn every_node_round_trips_at_its_encoded_length() {
        let diff = tree_diff();
        assert!(diff.new_nodes.iter().any(|(_, n)| matches!(n, Node::Internal(_))));
        assert!(diff.new_nodes.iter().any(|(_, n)| matches!(n, Node::Leaf(_))));
        for (key, node) in &diff.new_nodes {
            let encoded = encode_node(node);
            assert_eq!(node::cbor_len(node, &mut ()), encoded.len());
            assert_eq!(
                node::decode::<_, u64>(&mut Decoder::new(&encoded), &mut ()).unwrap(),
                *node
            );

            let mut e = Encoder::new(Vec::new());
            node_key::encode(key, &mut e, &mut ()).unwrap();
            let encoded = e.into_writer();
            assert_eq!(node_key::cbor_len(key, &mut ()), encoded.len());
            assert_eq!(node_key::decode(&mut Decoder::new(&encoded), &mut ()).unwrap(), *key);
        }
    }

    #[test]
    fn a_tree_diff_round_trips_at_its_encoded_length() {
        let diff = tree_diff();
        let encoded = tari_bor::encode(&diff).unwrap();
        assert_eq!(minicbor::len(&diff), encoded.len());
        let decoded: StateHashTreeDiff<u64> = tari_bor::decode_exact(&encoded).unwrap();
        assert_eq!(decoded.new_nodes, diff.new_nodes);
        assert_eq!(decoded.stale_tree_nodes, diff.stale_tree_nodes);
    }

    #[test]
    fn a_full_internal_node_encodes_compactly() {
        let diff = tree_diff();
        let full = diff
            .new_nodes
            .iter()
            .find_map(|(_, n)| match n {
                Node::Internal(internal) if internal.children_sorted().count() == 16 => Some(n),
                _ => None,
            })
            .expect("500 leaves fill the root");
        assert!(encode_node(full).len() < 720, "{} bytes", encode_node(full).len());
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Bytes `tari_jellyfish`'s `minicbor` feature (tari-project/tari#8101) encodes the same values to.
    #[test]
    fn encodes_as_tari_jellyfish_does() {
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
        let encode_key = |key: &NodeKey| {
            let mut e = Encoder::new(Vec::new());
            node_key::encode(key, &mut e, &mut ()).unwrap();
            hex(&e.into_writer())
        };
        let encode_stale = |stale: &StaleTreeNode| {
            let mut e = Encoder::new(Vec::new());
            stale_tree_node::encode(stale, &mut e, &mut ()).unwrap();
            hex(&e.into_writer())
        };

        assert_eq!(
            hex(&encode_node(&internal)),
            "8200a3008358200101010101010101010101010101010101010101010101010101010101010101058100038358200202020202020202\
             02020202020202020202020202020202020202020202020219012c8202070f8358200303030303030303030303030303030303030303\
             0303030303030303030303031a000111708100"
        );
        assert_eq!(
            hex(&encode_node(&leaf)),
            "8201845820040404040404040404040404040404040404040404040404040404040404040458200505050505050505050505050505\
             0505050505050505050505050505050505051904d2182a"
        );
        assert_eq!(hex(&encode_node(&Node::Null)), "8102");
        assert_eq!(encode_key(&even), "82098204421234");
        assert_eq!(encode_key(&odd), "820982014150");
        assert_eq!(encode_stale(&StaleTreeNode::Node(even)), "820082098204421234");
        assert_eq!(encode_stale(&StaleTreeNode::Subtree(odd)), "8201820982014150");
    }

    #[test]
    fn a_nibble_path_whose_count_disagrees_with_its_bytes_is_rejected() {
        let mut e = Encoder::new(Vec::new());
        e.array(2).unwrap().u64(3).unwrap().bytes(&[0xab]).unwrap();
        let encoded = e.into_writer();
        assert!(decode_nibble_path(&mut Decoder::new(&encoded), &mut ()).is_err());
    }
}
