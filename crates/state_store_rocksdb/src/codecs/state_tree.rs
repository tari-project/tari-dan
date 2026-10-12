//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io;

use anyhow::anyhow;
use tari_state_tree::{NibblePath, Node, NodeKey, StaleTreeNode, StateTreePayload, Version, cbor};

use crate::{
    codecs::{DbDecoder, DbEncoder},
    error::RocksDbStorageError,
    utils::take_fixed,
};

/// Codec for `NodeKey`
#[derive(Default)]
pub struct NodeKeyCodec;

impl NodeKeyCodec {
    fn encode_node_key_into<W: io::Write>(&self, key: &NodeKey, writer: &mut W) -> Result<(), RocksDbStorageError> {
        let version = key.version();
        let num_nibbles =
            u64::try_from(key.nibble_path().num_nibbles()).map_err(|_| RocksDbStorageError::EncodeError {
                source: anyhow!("Number of nibbles exceeds u64"),
            })?;
        let nibble_path = key.nibble_path();
        writer
            .write_all(&version.to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("NodeKeyCodec: Failed to write version: {}", e),
            })?;
        writer
            .write_all(&num_nibbles.to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("NodeKeyCodec: Failed to write num_nibbles: {}", e),
            })?;
        writer
            .write_all(nibble_path.bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("NodeKeyCodec: Failed to write nibble_path bytes: {}", e),
            })?;
        Ok(())
    }

    fn get_node_key_encoded_len(&self, key: &NodeKey) -> Result<usize, RocksDbStorageError> {
        let len = 8 + // version
            8 + // num_nibbles
            key.nibble_path().bytes().len(); // nibble_path bytes
        Ok(len)
    }
}

impl DbEncoder<NodeKey> for NodeKeyCodec {
    fn encode_len(&self, value: &NodeKey) -> Result<usize, RocksDbStorageError> {
        self.get_node_key_encoded_len(value)
    }

    fn encode_into<W: io::Write>(&self, value: &NodeKey, writer: &mut W) -> Result<(), RocksDbStorageError> {
        self.encode_node_key_into(value, writer)
    }
}

impl DbDecoder<NodeKey> for NodeKeyCodec {
    fn decode(&self, bytes: &[u8]) -> Result<(NodeKey, usize), RocksDbStorageError> {
        let version_bytes: [u8; 8] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::DecodeError {
            source: anyhow!("Invalid version bytes"),
        })?;
        let version = Version::from_be_bytes(version_bytes);

        let num_nibbles_bytes: [u8; 8] = take_fixed(&bytes[8..]).ok_or_else(|| RocksDbStorageError::DecodeError {
            source: anyhow!("Invalid num_nibbles bytes"),
        })?;
        let num_nibbles =
            usize::try_from(u64::from_be_bytes(num_nibbles_bytes)).map_err(|_| RocksDbStorageError::DecodeError {
                source: anyhow!("Number of nibbles exceeds usize"),
            })?;

        let is_even = num_nibbles % 2 == 0;
        let num_path_bytes = if is_even {
            num_nibbles / 2
        } else {
            num_nibbles.div_ceil(2)
        };
        let path_start = 16;
        let path_end = path_start + num_path_bytes;
        if bytes.len() < path_end {
            return Err(RocksDbStorageError::DecodeError {
                source: anyhow!("Invalid nibble path bytes. Could not read {} bytes", num_path_bytes),
            });
        }
        let nibble_path_bytes = &bytes[path_start..path_end];
        let nibble_path = if is_even {
            NibblePath::new_even(nibble_path_bytes)
        } else {
            NibblePath::new_odd(nibble_path_bytes)
        }
        .map_err(|e| RocksDbStorageError::DecodeError {
            source: anyhow!("Invalid nibble path: {e}"),
        })?;
        Ok((NodeKey::new(version, nibble_path), path_end))
    }
}

impl<'a> DbEncoder<&'a NodeKey> for NodeKeyCodec {
    fn encode_len(&self, value: &&'a NodeKey) -> Result<usize, RocksDbStorageError> {
        self.get_node_key_encoded_len(value)
    }

    fn encode_into<W: io::Write>(&self, value: &&'a NodeKey, writer: &mut W) -> Result<(), RocksDbStorageError> {
        self.encode_node_key_into(value, writer)
    }
}

/// [`Node`]s in `tari_state_tree::cbor`'s native encoding.
#[derive(Default)]
pub struct TreeNodeCodec;

impl DbEncoder<Node<StateTreePayload>> for TreeNodeCodec {
    fn encode_len(&self, value: &Node<StateTreePayload>) -> Result<usize, RocksDbStorageError> {
        Ok(cbor::node::cbor_len(value, &mut ()))
    }

    fn encode_into<W: io::Write>(
        &self,
        value: &Node<StateTreePayload>,
        writer: &mut W,
    ) -> Result<(), RocksDbStorageError> {
        let mut encoder = minicbor::Encoder::new(minicbor::encode::write::Writer::new(writer));
        cbor::node::encode(value, &mut encoder, &mut ()).map_err(|e| RocksDbStorageError::EncodeError {
            source: anyhow!("TreeNodeCodec: {e}"),
        })
    }
}

impl DbDecoder<Node<StateTreePayload>> for TreeNodeCodec {
    fn decode(&self, bytes: &[u8]) -> Result<(Node<StateTreePayload>, usize), RocksDbStorageError> {
        let mut decoder = minicbor::Decoder::new(bytes);
        let node = cbor::node::decode(&mut decoder, &mut ()).map_err(|e| RocksDbStorageError::DecodeError {
            source: anyhow!("TreeNodeCodec: {e}"),
        })?;
        Ok((node, decoder.position()))
    }
}

/// Lists of [`StaleTreeNode`]s in `tari_state_tree::cbor`'s native encoding.
#[derive(Default)]
pub struct StaleTreeNodesCodec;

impl StaleTreeNodesCodec {
    fn encoded_len(nodes: &[StaleTreeNode]) -> usize {
        nodes.iter().fold(minicbor::len(nodes.len() as u64), |acc, stale| {
            acc + cbor::stale_tree_node::cbor_len(stale, &mut ())
        })
    }

    fn encode_slice_into<W: io::Write>(nodes: &[StaleTreeNode], writer: &mut W) -> Result<(), RocksDbStorageError> {
        let encode_error = |e| RocksDbStorageError::EncodeError {
            source: anyhow!("StaleTreeNodesCodec: {e}"),
        };
        let mut encoder = minicbor::Encoder::new(minicbor::encode::write::Writer::new(writer));
        encoder.array(nodes.len() as u64).map_err(encode_error)?;
        for stale in nodes {
            cbor::stale_tree_node::encode(stale, &mut encoder, &mut ()).map_err(encode_error)?;
        }
        Ok(())
    }

    /// Encodes `nodes` as a stored list, without first collecting them into a `Vec`.
    pub fn encode_slice(&self, nodes: &[StaleTreeNode]) -> Result<Vec<u8>, RocksDbStorageError> {
        let mut encoded = Vec::with_capacity(Self::encoded_len(nodes));
        Self::encode_slice_into(nodes, &mut encoded)?;
        Ok(encoded)
    }
}

impl DbEncoder<Vec<StaleTreeNode>> for StaleTreeNodesCodec {
    fn encode_len(&self, value: &Vec<StaleTreeNode>) -> Result<usize, RocksDbStorageError> {
        Ok(Self::encoded_len(value))
    }

    fn encode_into<W: io::Write>(&self, value: &Vec<StaleTreeNode>, writer: &mut W) -> Result<(), RocksDbStorageError> {
        Self::encode_slice_into(value, writer)
    }
}

impl DbDecoder<Vec<StaleTreeNode>> for StaleTreeNodesCodec {
    fn decode(&self, bytes: &[u8]) -> Result<(Vec<StaleTreeNode>, usize), RocksDbStorageError> {
        let decode_error = |e| RocksDbStorageError::DecodeError {
            source: anyhow!("StaleTreeNodesCodec: {e}"),
        };
        let mut decoder = minicbor::Decoder::new(bytes);
        let len = decoder
            .array()
            .map_err(decode_error)?
            .ok_or_else(|| decode_error(minicbor::decode::Error::message("expected a definite-length array")))?;
        let mut nodes = Vec::with_capacity(len.min(4096) as usize);
        for _ in 0..len {
            nodes.push(cbor::stale_tree_node::decode(&mut decoder, &mut ()).map_err(decode_error)?);
        }
        Ok((nodes, decoder.position()))
    }
}

#[cfg(test)]
mod tests {
    use std::iter;

    use rand::RngExt;
    use tari_state_tree::{
        JellyfishMerkleTree,
        JmtHashScheme,
        LeafKey,
        StaleTreeNode,
        TreeHash,
        TreeStoreWriter,
        memory_store::MemoryTreeStore,
    };

    use super::*;

    #[test]
    fn encode_decode() {
        let version = 1;
        let nibble_path = NibblePath::new_odd(&[0x01, 0x02, 0x03, 0x04 << 4]).unwrap();
        let key = NodeKey::new(version, nibble_path);
        let codec = NodeKeyCodec;
        let encoded1 = codec.encode(&key).unwrap();
        let decoded = codec.decode_exact(&encoded1).unwrap();
        assert_eq!(key, decoded);

        let version = 2;
        let nibble_path = NibblePath::new_even(&[0x01, 0x02, 0x03, 0x04]).unwrap();
        let key = NodeKey::new(version, nibble_path);

        let encoded = codec.encode(&key).unwrap();
        assert_ne!(encoded, encoded1);
        let decoded = codec.decode_exact(&encoded).unwrap();
        assert_eq!(key, decoded);
    }

    #[test]
    fn smoke() {
        let mut store = MemoryTreeStore::new();
        let jmt = JellyfishMerkleTree::new(&store, JmtHashScheme::V1);
        let changes = iter::repeat_with(|| TreeHash::new(random_bytes()))
            .take(100)
            .map(|hash| (LeafKey::new(hash), Some((hash, ()))));

        let (_, update) = jmt.batch_put_value_set(changes, None, 1).unwrap();

        let codec = NodeKeyCodec;
        for (key, node) in update.node_batch {
            let encoded = codec.encode(&key).unwrap();
            let decoded = codec.decode_exact(&encoded).unwrap();
            assert_eq!(key, decoded);
            store.insert_node(key, node).unwrap()
        }
        for stale_tree_node in update.stale_node_index_batch {
            store
                .record_stale_tree_node(StaleTreeNode::Node(stale_tree_node.node_key))
                .unwrap();
        }

        let jmt = JellyfishMerkleTree::new(&store, JmtHashScheme::V1);
        let changes = iter::repeat_with(|| TreeHash::new(random_bytes()))
            .take(100)
            .map(|hash| (LeafKey::new(hash), Some((hash, ()))))
            .collect::<Vec<_>>();
        let (_, update) = jmt.batch_put_value_set(changes.iter().copied(), Some(1), 2).unwrap();

        for (key, node) in update.node_batch {
            let encoded = codec.encode(&key).unwrap();
            let decoded = codec.decode_exact(&encoded).unwrap();
            assert_eq!(key, decoded);
            store.insert_node(key, node).unwrap()
        }
        for stale_tree_node in update.stale_node_index_batch {
            store
                .record_stale_tree_node(StaleTreeNode::Node(stale_tree_node.node_key))
                .unwrap();
        }
        let jmt = JellyfishMerkleTree::new(&store, JmtHashScheme::V1);
        let _unused = jmt
            .batch_put_value_set(changes.into_iter().map(|(key, _)| (key, None)), Some(2), 3)
            .unwrap();
    }

    fn random_bytes<const L: usize>() -> [u8; L] {
        rand::rng().random()
    }
}
