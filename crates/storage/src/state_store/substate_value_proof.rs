//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, hash_map::Entry};

use ootle_network::Network;
use tari_common_types::types::FixedHash;
use tari_engine_types::substate::{SubstateId, SubstateValue, hash_substate};
use tari_ootle_common_types::{Epoch, NumPreshards, ShardGroup, VersionedSubstateId, shard::Shard};
use tari_state_tree::{
    RootProofTree,
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    SparseMerkleProofExt,
    SpreadPrefixStateTree,
    SubstateValueProof,
    SubstateValueProofError,
    TreeHash,
    Version,
};

use crate::{StateStoreReadTransaction, StorageError, state_store::ShardScopedTreeStoreReader};

/// Generates two-level [`SubstateValueProof`]s against one committed shard-group state.
///
/// A proof has three parts, and only the last of them varies per substate:
/// - the shard group's per-shard roots, in the canonical order the block header commits them,
/// - level 2: the proof that a shard's root is committed in the shard-group root - one per shard,
/// - level 1: the JMT leaf proof (inclusion or exclusion) for the substate within its shard.
///
/// The first two are read and computed once and reused, so proving N substates costs N leaf
/// traversals rather than N scans of the whole shard group.
///
/// Every proof is rooted at the shard-group `state_merkle_root` - the same root the latest committed
/// block header commits - so a caller that independently trusts that root (e.g. via a verified
/// committed block proof) can verify each substate's committed value or its absence without trusting
/// the node that produced the proofs. One commit proof therefore authenticates every proof a single
/// generator produces, provided both are generated in the same read transaction.
///
/// Whether a leaf proof is an inclusion or exclusion proof depends on whether the substate is
/// currently up in its shard; the caller chooses `verify_inclusion`/`verify_exclusion` accordingly.
pub struct SubstateProofGenerator<'a, TTx> {
    tx: &'a TTx,
    num_preshards: NumPreshards,
    /// The tree over the shard group's per-shard roots, in the canonical order the block header
    /// commits them: [global, shard_0, ...]. Every level-2 proof is a leaf of this one tree, so it is
    /// built once however many substates are proved - which is what keeps the cost of a batch
    /// independent of the size of the shard group.
    root_tree: RootProofTree,
    /// The committed state of each shard in the root tree, keyed by shard.
    shards: HashMap<Shard, CommittedShardState>,
    /// Level-2 proofs, extracted from `root_tree` on first use of each shard.
    shard_root_proofs: HashMap<Shard, SparseMerkleProofExt>,
}

#[derive(Debug, Clone, Copy)]
struct CommittedShardState {
    root: TreeHash,
    /// `None` if the shard has no committed state yet, in which case `root` is the empty-tree
    /// placeholder and no substate in the shard can be proved.
    version: Option<Version>,
}

impl<'a, TTx: StateStoreReadTransaction> SubstateProofGenerator<'a, TTx> {
    /// Reads the committed root of every shard in `shard_group`, plus the global shard.
    pub fn new(tx: &'a TTx, shard_group: ShardGroup, num_preshards: NumPreshards) -> Result<Self, StorageError> {
        let mut ordered_roots = Vec::with_capacity(shard_group.len() + 1);
        let mut shards = HashMap::with_capacity(shard_group.len() + 1);
        for shard in shard_group.shard_iter_with_global() {
            let state = committed_shard_state(tx, shard)?;
            ordered_roots.push(state.root);
            shards.insert(shard, state);
        }

        Ok(Self {
            tx,
            num_preshards,
            root_tree: RootProofTree::build(ordered_roots).map_err(|e| StorageError::QueryError {
                reason: format!("SubstateProofGenerator shard group root tree: {e}"),
            })?,
            shards,
            shard_root_proofs: HashMap::new(),
        })
    }

    /// Proves `versioned_id`'s committed value, or its absence, against the shard-group root.
    ///
    /// `Ok(None)` means this state cannot prove anything about the substate, either way: its shard
    /// lies outside the shard group, or that shard has no committed state to root a proof at. A
    /// caller proving many substates can drop that one and keep the rest; an error means the read
    /// itself failed and nothing it produced can be trusted.
    pub fn generate(&mut self, versioned_id: &VersionedSubstateId) -> Result<Option<SubstateValueProof>, StorageError> {
        let shard = versioned_id.to_shard(self.num_preshards);
        let Some(state) = self.shards.get(&shard).copied() else {
            return Ok(None);
        };
        let Some(version) = state.version else {
            return Ok(None);
        };

        // Level 1: leaf proof (inclusion or exclusion) for the substate within its shard.
        let mut scoped = ShardScopedTreeStoreReader::new(self.tx, shard);
        let tree = SpreadPrefixStateTree::new(&mut scoped);
        let (_leaf_key, _proof_value, leaf_proof) =
            tree.get_proof(version, versioned_id)
                .map_err(|e| StorageError::QueryError {
                    reason: format!("SubstateProofGenerator get_proof: {e}"),
                })?;

        // Level 2: prove the shard root is committed in the shard-group root.
        let shard_root_proof = match self.shard_root_proofs.entry(shard) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let (_, proof) = self
                    .root_tree
                    .get_proof(state.root)
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("SubstateProofGenerator shard root proof: {e}"),
                    })?;
                entry.insert(proof).clone()
            },
        };

        Ok(Some(SubstateValueProof::new(state.root, shard_root_proof, leaf_proof)))
    }
}

/// Verifies a substate value proof against an *already-trusted* shard-group state merkle root,
/// skipping commit-proof (QC chain) validation.
///
/// `trusted_root` must have been established independently - from a commit proof validated against
/// the shard group committee (`CommittedBlockProof::validate`), either for this read or in an earlier
/// round and recorded in a trusted-root store. The validator pins the substate value proof to the
/// same committed block whose `state_merkle_root` is trusted (proof and commit proof are generated in
/// one read transaction against the same committed block), so verifying against that root is the
/// whole of the check. The trust decision must therefore be keyed on `trusted_root` itself: a node
/// cannot forge a substate proof that verifies against a root a quorum already signed.
pub fn verify_substate_value_proof_against_root(
    value_proof_bytes: &[u8],
    substate_id: &SubstateId,
    version: u64,
    value: Option<&SubstateValue>,
    network: Network,
    proof_epoch: Epoch,
    trusted_root: FixedHash,
) -> Result<(), SubstateProofVerifyError> {
    let group_root = TreeHash::new(trusted_root.into_array());

    let value_proof: SubstateValueProof = tari_bor::serde_codec::from_slice(value_proof_bytes)
        .map_err(|e| SubstateProofVerifyError::Decode(e.to_string()))?;

    let versioned_id = VersionedSubstateId::new(substate_id.clone(), version);
    match value {
        Some(value) => {
            // Bind the returned value to the committed leaf by re-deriving its value hash, so a
            // validator cannot swap the value while presenting a proof for the real committed leaf.
            let value_hash = TreeHash::new(hash_substate(network, value, version, proof_epoch).into_array());
            value_proof.verify_inclusion(&group_root, &versioned_id, &value_hash)?;
        },
        None => {
            value_proof.verify_exclusion(&group_root, &versioned_id)?;
        },
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum SubstateProofVerifyError {
    #[error("failed to decode substate value proof: {0}")]
    Decode(String),
    #[error("substate value proof invalid: {0}")]
    ValueProof(#[from] SubstateValueProofError),
}

/// The committed JMT root and state-tree version of `shard`. A shard with no committed state has the
/// empty-tree placeholder for a root and no version.
fn committed_shard_state<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    shard: Shard,
) -> Result<CommittedShardState, StorageError> {
    let Some(version) = tx.state_tree_versions_get_latest(shard)? else {
        return Ok(CommittedShardState {
            root: SPARSE_MERKLE_PLACEHOLDER_HASH,
            version: None,
        });
    };
    let mut scoped = ShardScopedTreeStoreReader::new(tx, shard);
    let tree = SpreadPrefixStateTree::new(&mut scoped);
    let root = tree.get_root_hash(version).map_err(|e| StorageError::QueryError {
        reason: format!("SubstateProofGenerator shard {shard} root: {e}"),
    })?;
    Ok(CommittedShardState {
        root,
        version: Some(version),
    })
}
