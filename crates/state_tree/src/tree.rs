//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, HashMap},
    marker::PhantomData,
};

use serde::{Deserialize, Serialize};
use tari_engine_types::ProtocolVersion;
use tari_jellyfish::{
    JellyfishMerkleTree,
    JmtHashScheme,
    LeafKey,
    Node,
    NodeKey,
    ProofValue,
    SparseMerkleProofExt,
    StaleTreeNode,
    TreeHash,
    TreeStore,
    TreeStoreReader,
    TreeStoreWriter,
    TreeUpdateBatch,
    Version,
};
use tari_ootle_common_types::{ToSubstateAddress, VersionedSubstateId, shard::Shard};
use tari_template_lib_types::Hash32;

use crate::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    StateTreePayload,
    TreeStoreBatchWriter,
    cbor,
    error::StateTreeError,
    key_mapper::{DbKeyMapper, HashIdentityKeyMapper, ShardKeyMapper, SpreadPrefixKeyMapper},
    memory_store::MemoryTreeStore,
    shard_state_leaf,
};

const LOG_TARGET: &str = "tari::ootle::state_tree";

pub type SpreadPrefixStateTree<'a, S> = StateTree<'a, S, SpreadPrefixKeyMapper>;
pub type RootStateTree<'a, S> = StateTree<'a, S, HashIdentityKeyMapper>;

pub struct StateTree<'a, S, M> {
    store: &'a mut S,
    _mapper: PhantomData<M>,
}

impl<'a, S, M> StateTree<'a, S, M> {
    pub fn new(store: &'a mut S) -> Self {
        Self {
            store,
            _mapper: PhantomData,
        }
    }
}

impl<S: TreeStoreReader<StateTreePayload>, M: DbKeyMapper<VersionedSubstateId>> StateTree<'_, S, M> {
    pub fn get_proof(
        &self,
        version: Version,
        key: &VersionedSubstateId,
    ) -> Result<(LeafKey, Option<ProofValue<StateTreePayload>>, SparseMerkleProofExt), StateTreeError> {
        let jmt = JellyfishMerkleTree::new(self.store, JmtHashScheme::V1);
        let key = M::map_to_leaf_key(key);
        let (maybe_value, proof) = jmt.get_with_proof_ext(key.as_ref(), version)?;
        Ok((key, maybe_value, proof))
    }

    pub fn get_root_hash(&self, version: Version) -> Result<TreeHash, StateTreeError> {
        let jmt = JellyfishMerkleTree::new(self.store, JmtHashScheme::V1);
        let root_hash = jmt.get_root_hash(version)?;
        Ok(root_hash)
    }

    fn calculate_substate_changes<I: IntoIterator<Item = SubstateTreeChange>>(
        &mut self,
        current_version: Option<Version>,
        next_version: Version,
        changes: I,
    ) -> Result<(TreeHash, StateHashTreeDiff<StateTreePayload>), StateTreeError> {
        let (root_hash, update_batch) =
            calculate_substate_changes::<_, M, _>(self.store, current_version, next_version, changes)?;
        Ok((root_hash, update_batch.into()))
    }
}

impl<S: TreeStore<StateTreePayload>, M: DbKeyMapper<VersionedSubstateId>> StateTree<'_, S, M> {
    /// Stores the substate changes in the state tree and returns the new root hash.
    pub fn put_substate_changes<I: IntoIterator<Item = SubstateTreeChange>>(
        &mut self,
        current_version: Option<Version>,
        next_version: Version,
        changes: I,
    ) -> Result<TreeHash, StateTreeError> {
        let (root_hash, update_batch) = self.calculate_substate_changes(current_version, next_version, changes)?;
        self.commit_diff(update_batch)?;
        Ok(root_hash)
    }

    fn commit_diff(&mut self, diff: StateHashTreeDiff<StateTreePayload>) -> Result<(), StateTreeError> {
        for (key, node) in diff.new_nodes {
            log::debug!("Inserting node: {}", key);
            self.store.insert_node(key, node)?;
        }

        for stale_tree_node in diff.stale_tree_nodes {
            log::debug!("Recording stale tree node: {}", stale_tree_node.as_node_key());
            self.store.record_stale_tree_node(stale_tree_node)?;
        }

        Ok(())
    }
}

impl<S: TreeStoreReader<StateTreePayload> + TreeStoreBatchWriter<StateTreePayload>, M: DbKeyMapper<VersionedSubstateId>>
    StateTree<'_, S, M>
{
    /// Stores the substate changes in the state tree and returns the new root hash.
    pub fn batch_put_substate_changes<I: IntoIterator<Item = SubstateTreeChange>>(
        &mut self,
        current_version: Option<Version>,
        next_version: Version,
        changes: I,
    ) -> Result<TreeHash, StateTreeError> {
        let (root_hash, update_batch) = self.calculate_substate_changes(current_version, next_version, changes)?;
        log::debug!(
            target: LOG_TARGET,
            "Batch inserting {} new nodes and recording {} stale tree nodes",
            update_batch.new_nodes.len(),
            update_batch.stale_tree_nodes.len()
        );
        self.store.batch_insert_nodes(update_batch.new_nodes)?;
        self.store
            .record_stale_tree_nodes(next_version, update_batch.stale_tree_nodes)?;

        Ok(root_hash)
    }
}

impl<S: TreeStore<()>, M: DbKeyMapper<TreeHash>> StateTree<'_, S, M> {
    pub fn put_changes<I: IntoIterator<Item = TreeHash>>(
        &mut self,
        current_version: Option<Version>,
        next_version: Version,
        changes: I,
    ) -> Result<TreeHash, StateTreeError> {
        let (root_hash, update_result) = self.compute_update_batch(current_version, next_version, changes)?;

        for (k, node) in update_result.node_batch {
            self.store.insert_node(k, node)?;
        }

        for stale_tree_node in update_result.stale_node_index_batch {
            self.store
                .record_stale_tree_node(StaleTreeNode::Node(stale_tree_node.node_key))?;
        }

        Ok(root_hash)
    }

    pub fn compute_update_batch<I: IntoIterator<Item = TreeHash>>(
        &mut self,
        current_version: Option<Version>,
        next_version: Version,
        changes: I,
    ) -> Result<(TreeHash, TreeUpdateBatch<()>), StateTreeError> {
        let jmt = JellyfishMerkleTree::<_, ()>::new(self.store, JmtHashScheme::V1);

        let changes = changes
            .into_iter()
            .map(|hash| (M::map_to_leaf_key(&hash), Some((hash, ()))));

        let (root, update) = jmt.batch_put_value_set(changes, current_version, next_version)?;
        Ok((root, update))
    }
}

/// Calculates the new root hash and tree updates for the given substate changes.
fn calculate_substate_changes<
    S: TreeStoreReader<StateTreePayload>,
    M: DbKeyMapper<VersionedSubstateId>,
    I: IntoIterator<Item = SubstateTreeChange>,
>(
    store: &mut S,
    current_version: Option<Version>,
    next_version: Version,
    changes: I,
) -> Result<(TreeHash, TreeUpdateBatch<StateTreePayload>), StateTreeError> {
    // JMT nodes are keyed by (version, nibble_path). Writing a version that is not strictly ahead of
    // the base version overwrites live nodes with keys that this same write records as stale, so the
    // stale-node GC later deletes nodes the current tree still points at. Callers that stage changes
    // and write them later are additionally guarded at the write funnel, in
    // `ShardScopedTreeStoreWriter::set_state_version`.
    if let Some(current_version) = current_version &&
        next_version <= current_version
    {
        return Err(StateTreeError::NonMonotonicVersion {
            current_version,
            next_version,
        });
    }

    let jmt = JellyfishMerkleTree::new(store, JmtHashScheme::V1);

    let changes = changes.into_iter().map(|ch| match ch {
        SubstateTreeChange::Up { id, value_hash } => (
            M::map_to_leaf_key(&id),
            Some((TreeHash::new(value_hash.into_array()), id.to_substate_address())),
        ),
        SubstateTreeChange::Down { id } => (M::map_to_leaf_key(&id), None),
    });

    let (root_hash, update_result) = jmt.batch_put_value_set(changes, current_version, next_version)?;

    Ok((root_hash, update_result))
}

pub enum SubstateTreeChange {
    Up {
        id: VersionedSubstateId,
        value_hash: Hash32,
    },
    Down {
        id: VersionedSubstateId,
    },
}

impl SubstateTreeChange {
    pub fn id(&self) -> &VersionedSubstateId {
        match self {
            Self::Up { id, .. } => id,
            Self::Down { id } => id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateHashTreeDiff<P> {
    pub new_nodes: Vec<(NodeKey, Node<P>)>,
    pub stale_tree_nodes: Vec<StaleTreeNode>,
}

/// Encoded as `[[[node_key, node], ...], [stale_tree_node, ...]]`, the array a minicbor derive gives the struct.
impl<C, P: minicbor::Encode<C>> minicbor::Encode<C> for StateHashTreeDiff<P> {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        e.array(2)?;
        e.array(self.new_nodes.len() as u64)?;
        for (key, node) in &self.new_nodes {
            e.array(2)?;
            cbor::node_key::encode(key, e, ctx)?;
            cbor::node::encode(node, e, ctx)?;
        }
        e.array(self.stale_tree_nodes.len() as u64)?;
        for stale in &self.stale_tree_nodes {
            cbor::stale_tree_node::encode(stale, e, ctx)?;
        }
        Ok(())
    }
}

impl<'b, C, P: minicbor::Decode<'b, C>> minicbor::Decode<'b, C> for StateHashTreeDiff<P> {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let definite = |len: Option<u64>| {
            len.ok_or_else(|| minicbor::decode::Error::message("StateHashTreeDiff: expected a definite-length array"))
        };
        d.array()?;
        let num_new = definite(d.array()?)?;
        let mut new_nodes = Vec::with_capacity(num_new.min(4096) as usize);
        for _ in 0..num_new {
            d.array()?;
            let key = cbor::node_key::decode(d, ctx)?;
            let node = cbor::node::decode(d, ctx)?;
            new_nodes.push((key, node));
        }
        let num_stale = definite(d.array()?)?;
        let mut stale_tree_nodes = Vec::with_capacity(num_stale.min(4096) as usize);
        for _ in 0..num_stale {
            stale_tree_nodes.push(cbor::stale_tree_node::decode(d, ctx)?);
        }
        Ok(Self {
            new_nodes,
            stale_tree_nodes,
        })
    }
}

impl<C, P: minicbor::CborLen<C>> minicbor::CborLen<C> for StateHashTreeDiff<P> {
    fn cbor_len(&self, ctx: &mut C) -> usize {
        let header = |len: usize| cbor::header_len(len as u64);
        let mut len = header(2) + header(self.new_nodes.len()) + header(self.stale_tree_nodes.len());
        for (key, node) in &self.new_nodes {
            len += header(2) + cbor::node_key::cbor_len(key, ctx) + cbor::node::cbor_len(node, ctx);
        }
        for stale in &self.stale_tree_nodes {
            len += cbor::stale_tree_node::cbor_len(stale, ctx);
        }
        len
    }
}

impl<P> StateHashTreeDiff<P> {
    pub fn new() -> Self {
        Self {
            new_nodes: Vec::new(),
            stale_tree_nodes: Vec::new(),
        }
    }
}

impl<P> From<TreeUpdateBatch<P>> for StateHashTreeDiff<P> {
    fn from(batch: TreeUpdateBatch<P>) -> Self {
        Self {
            new_nodes: batch.node_batch,
            stale_tree_nodes: batch
                .stale_node_index_batch
                .into_iter()
                .map(|node| StaleTreeNode::Node(node.node_key))
                .collect(),
        }
    }
}

pub fn compute_merkle_root_for_hashes<I: IntoIterator<Item = TreeHash>>(hashes: I) -> Result<TreeHash, StateTreeError> {
    let mut hashes = hashes.into_iter().peekable();
    if hashes.peek().is_none() {
        return Ok(SPARSE_MERKLE_PLACEHOLDER_HASH);
    }
    let mut mem_store = MemoryTreeStore::new();
    let mut root_tree = RootStateTree::new(&mut mem_store);
    let (hash, _) = root_tree.compute_update_batch(None, 1, hashes)?;
    Ok(hash)
}

/// An ephemeral tree over a set of hashes, held so that several of them can be proved against the
/// same root without rebuilding it.
///
/// Building costs O(n log n) hashes in n, the size of the set; each proof after that is one
/// traversal. Callers proving more than one hash against the same set should build this once -
/// [`compute_proof_for_hashes`] is the one-shot form and rebuilds the tree per proof.
pub struct RootProofTree {
    store: MemoryTreeStore<()>,
}

impl RootProofTree {
    pub fn build<I: IntoIterator<Item = TreeHash>>(hashes: I) -> Result<Self, StateTreeError> {
        let mut store = MemoryTreeStore::new();
        RootStateTree::new(&mut store).put_changes(None, 1, hashes)?;
        Ok(Self { store })
    }

    /// Proves that `hash_to_prove` is one of the hashes the tree was built over, or that it is not.
    /// Returns the value (if it exists) and the Merkle proof.
    pub fn get_proof(
        &self,
        hash_to_prove: TreeHash,
    ) -> Result<(Option<ProofValue<()>>, SparseMerkleProofExt), StateTreeError> {
        let jmt = JellyfishMerkleTree::new(&self.store, JmtHashScheme::V1);
        let key = HashIdentityKeyMapper::map_to_leaf_key(&hash_to_prove);
        let proof_tuple = jmt.get_with_proof_ext(key.as_ref(), 1)?;
        Ok(proof_tuple)
    }
}

/// Where a shard's state sits in its shard group's root tree: the key of its leaf, and the leaf's
/// value, or `None` when the tree holds no leaf for the shard.
///
/// - [`ProtocolVersion::V0`] and [`ProtocolVersion::V1`]: the leaf is [`shard_state_leaf`], keyed by its own hash. The
///   tree is a set of leaves, so a proof against such a root shows only that a leaf is one of the group's.
/// - [`ProtocolVersion::V2`]: the leaf is keyed by the shard, so a proof names the shard a leaf belongs to, and the
///   leaf value commits to the shard as well (see [`shard_state_leaf`]). A shard with no state - the empty-tree root at
///   state version 0 - has no leaf, and the absence of its key proves its state. This keeps the tree, and the cost of
///   building it for every block, proportional to the shards that hold state.
#[derive(Debug, Clone)]
pub struct ShardGroupLeaf {
    pub key: LeafKey,
    pub value: Option<TreeHash>,
}

impl ShardGroupLeaf {
    pub fn new(protocol_version: ProtocolVersion, shard: Shard, shard_root: &TreeHash, state_version: Version) -> Self {
        let value = shard_state_leaf(protocol_version, shard, shard_root, state_version);
        match protocol_version {
            ProtocolVersion::V0 | ProtocolVersion::V1 => Self {
                key: HashIdentityKeyMapper::map_to_leaf_key(&value),
                value: Some(value),
            },
            ProtocolVersion::V2 => {
                let has_state = *shard_root != SPARSE_MERKLE_PLACEHOLDER_HASH || state_version != 0;
                Self {
                    key: ShardKeyMapper::map_to_leaf_key(&shard),
                    value: has_state.then_some(value),
                }
            },
        }
    }
}

/// The shard-group state root tree: an ephemeral tree over the shard group's per-shard states,
/// global shard included, laid out as [`ShardGroupLeaf`] describes for the protocol version. Its
/// root is the `state_merkle_root` a block header commits.
pub struct ShardGroupRootTree {
    store: MemoryTreeStore<()>,
    root: TreeHash,
    leaves: HashMap<Shard, ShardGroupLeaf>,
}

impl ShardGroupRootTree {
    /// Builds the tree over `(shard, shard root, state version)` entries, in the canonical
    /// `[global, shard_0, ...]` order a V0 root is formed in. Fails if a shard appears more than once.
    pub fn build<I: IntoIterator<Item = (Shard, TreeHash, Version)>>(
        protocol_version: ProtocolVersion,
        shard_states: I,
    ) -> Result<Self, StateTreeError> {
        let mut store = MemoryTreeStore::<()>::new();
        let shard_states = shard_states.into_iter();
        let mut leaves = HashMap::with_capacity(shard_states.size_hint().0);
        let mut changes = Vec::with_capacity(shard_states.size_hint().0);
        for (shard, shard_root, state_version) in shard_states {
            let leaf = ShardGroupLeaf::new(protocol_version, shard, &shard_root, state_version);
            if let Some(value) = leaf.value {
                changes.push((leaf.key, Some((value, ()))));
            }
            if leaves.insert(shard, leaf).is_some() {
                return Err(StateTreeError::DuplicateShardInShardGroupTree { shard });
            }
        }
        if changes.is_empty() {
            return Ok(Self {
                store,
                root: SPARSE_MERKLE_PLACEHOLDER_HASH,
                leaves,
            });
        }
        let (root, update) = JellyfishMerkleTree::<_, ()>::new(&store, JmtHashScheme::V1).batch_put_value_set(
            changes,
            None,
            SHARD_GROUP_ROOT_VERSION,
        )?;
        for (key, node) in update.node_batch {
            store.insert_node(key, node)?;
        }
        Ok(Self { store, root, leaves })
    }

    pub fn root(&self) -> TreeHash {
        self.root
    }

    /// Proves the leaf the tree holds for `shard`, or under [`ProtocolVersion::V2`] that it holds
    /// none - which is the proof that the shard has no state.
    pub fn get_proof(&self, shard: Shard) -> Result<(Option<ProofValue<()>>, SparseMerkleProofExt), StateTreeError> {
        let leaf = self
            .leaves
            .get(&shard)
            .ok_or(StateTreeError::ShardNotInShardGroupTree { shard })?;
        let jmt = JellyfishMerkleTree::new(&self.store, JmtHashScheme::V1);
        let proof_tuple = jmt.get_with_proof_ext(leaf.key.as_ref(), SHARD_GROUP_ROOT_VERSION)?;
        Ok(proof_tuple)
    }
}

const SHARD_GROUP_ROOT_VERSION: Version = 1;

/// Computes the shard-group state root over `(shard, shard root, state version)` entries. See
/// [`ShardGroupRootTree`].
pub fn compute_shard_group_root<I: IntoIterator<Item = (Shard, TreeHash, Version)>>(
    protocol_version: ProtocolVersion,
    shard_states: I,
) -> Result<TreeHash, StateTreeError> {
    Ok(ShardGroupRootTree::build(protocol_version, shard_states)?.root())
}

/// Computes a Merkle proof for the given hash is either included in the provided the hashes, or proof of absence.
/// Returns the value (if it exists) and the Merkle proof.
pub fn compute_proof_for_hashes<I: Iterator<Item = TreeHash>>(
    hashes: I,
    hash_to_prove: TreeHash,
) -> Result<(Option<ProofValue<()>>, SparseMerkleProofExt), StateTreeError> {
    RootProofTree::build(hashes)?.get_proof(hash_to_prove)
}

/// An ephemeral tree over leaves placed at caller-chosen keys. Each leaf can be proved, and any other key shown
/// absent, against [`Self::root`].
pub struct KeyedProofTree {
    store: MemoryTreeStore<()>,
    root: TreeHash,
}

impl KeyedProofTree {
    /// Builds the tree over `(key, value hash)` leaves. Fails if two leaves share a key, since the tree holds one
    /// value per key.
    pub fn build<I: IntoIterator<Item = (LeafKey, TreeHash)>>(leaves: I) -> Result<Self, StateTreeError> {
        let mut leaves_by_key = BTreeMap::new();
        for (key, value) in leaves {
            if leaves_by_key.insert(key, value).is_some() {
                return Err(StateTreeError::DuplicateLeafKey { key: key.bytes });
            }
        }

        let mut store = MemoryTreeStore::new();
        let (root, batch) = JellyfishMerkleTree::<_, ()>::new(&store, JmtHashScheme::V1).batch_put_value_set(
            leaves_by_key.into_iter().map(|(key, value)| (key, Some((value, ())))),
            None,
            1,
        )?;
        for (key, node) in batch.node_batch {
            store.insert_node(key, node)?;
        }
        Ok(Self { store, root })
    }

    pub fn root(&self) -> TreeHash {
        self.root
    }

    /// Proves the value at `key`, or that no leaf has that key. Returns the value (if it exists) and the Merkle proof.
    pub fn get_proof(&self, key: &LeafKey) -> Result<(Option<ProofValue<()>>, SparseMerkleProofExt), StateTreeError> {
        let jmt = JellyfishMerkleTree::new(&self.store, JmtHashScheme::V1);
        Ok(jmt.get_with_proof_ext(key.as_ref(), 1)?)
    }
}
