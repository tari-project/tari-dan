//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, ops::Deref, sync::Arc};

use tari_jellyfish::{Node, NodeKey};

use crate::StateHashTreeDiff;

/// A [`StateHashTreeDiff`] whose new nodes can be looked up by key. Cloning shares the diff.
///
/// It encodes exactly as the [`StateHashTreeDiff`] it holds.
#[derive(Debug)]
pub struct IndexedTreeDiff<P> {
    inner: Arc<Indexed<P>>,
}

#[derive(Debug)]
struct Indexed<P> {
    diff: StateHashTreeDiff<P>,
    /// Position of each new node in `diff.new_nodes`.
    positions: HashMap<NodeKey, usize>,
}

impl<P> IndexedTreeDiff<P> {
    pub fn new(diff: StateHashTreeDiff<P>) -> Self {
        let positions = diff
            .new_nodes
            .iter()
            .enumerate()
            .map(|(position, (key, _))| (key.clone(), position))
            .collect();
        Self {
            inner: Arc::new(Indexed { diff, positions }),
        }
    }

    pub fn get_node(&self, key: &NodeKey) -> Option<&Node<P>> {
        let position = *self.inner.positions.get(key)?;
        Some(&self.inner.diff.new_nodes[position].1)
    }
}

impl<P> Clone for IndexedTreeDiff<P> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<P> Default for IndexedTreeDiff<P> {
    fn default() -> Self {
        Self::new(StateHashTreeDiff::new())
    }
}

impl<P> Deref for IndexedTreeDiff<P> {
    type Target = StateHashTreeDiff<P>;

    fn deref(&self) -> &Self::Target {
        &self.inner.diff
    }
}

impl<P> From<StateHashTreeDiff<P>> for IndexedTreeDiff<P> {
    fn from(diff: StateHashTreeDiff<P>) -> Self {
        Self::new(diff)
    }
}

impl<P: serde::Serialize> serde::Serialize for IndexedTreeDiff<P> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.inner.diff.serialize(serializer)
    }
}

impl<'de, P: serde::Deserialize<'de>> serde::Deserialize<'de> for IndexedTreeDiff<P> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        StateHashTreeDiff::deserialize(deserializer).map(Self::new)
    }
}

impl<C, P: minicbor::Encode<C>> minicbor::Encode<C> for IndexedTreeDiff<P> {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        self.inner.diff.encode(e, ctx)
    }
}

impl<'b, C, P: minicbor::Decode<'b, C>> minicbor::Decode<'b, C> for IndexedTreeDiff<P> {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        StateHashTreeDiff::decode(d, ctx).map(Self::new)
    }
}

impl<C, P: minicbor::CborLen<C>> minicbor::CborLen<C> for IndexedTreeDiff<P> {
    fn cbor_len(&self, ctx: &mut C) -> usize {
        self.inner.diff.cbor_len(ctx)
    }
}

#[cfg(test)]
mod tests {
    use tari_jellyfish::{LeafKey, NibblePath};

    use super::*;

    fn diff() -> StateHashTreeDiff<u32> {
        let mut diff = StateHashTreeDiff::new();
        for i in 0..3u8 {
            let key = NodeKey::new(u64::from(i), NibblePath::new_even(&[i; 2]).unwrap());
            let node = Node::new_leaf(LeafKey::new([i; 32].into()), [i; 32].into(), u32::from(i), u64::from(i));
            diff.new_nodes.push((key, node));
        }
        diff
    }

    #[test]
    fn finds_each_new_node_by_key() {
        let diff = diff();
        let indexed = IndexedTreeDiff::new(diff.clone());
        for (key, node) in &diff.new_nodes {
            assert_eq!(indexed.get_node(key), Some(node));
        }
        assert_eq!(indexed.get_node(&NodeKey::new_empty_path(9)), None);
    }

    #[test]
    fn encodes_as_the_diff_it_holds() {
        let diff = diff();
        let indexed = IndexedTreeDiff::new(diff.clone());
        let encoded = tari_bor::encode(&indexed).unwrap();
        assert_eq!(encoded, tari_bor::encode(&diff).unwrap());

        let decoded: IndexedTreeDiff<u32> = tari_bor::decode_exact(&encoded).unwrap();
        assert_eq!(decoded.new_nodes, diff.new_nodes);
        let (key, node) = &diff.new_nodes[1];
        assert_eq!(decoded.get_node(key), Some(node));
    }
}
