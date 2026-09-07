//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, hash_map::Entry};

use ootle_network::Network;
use tari_common_types::types::FixedHash;
use tari_engine_types::substate::{SubstateId, SubstateValue, hash_substate};
use tari_ootle_common_types::{Epoch, NumPreshards, ShardGroup, VersionedSubstateId, VotePower, shard::Shard};
use tari_sidechain::SidechainProofValidationError;
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    SparseMerkleProofExt,
    SpreadPrefixStateTree,
    SubstateValueProof,
    SubstateValueProofError,
    TreeHash,
    Version,
    compute_proof_for_hashes,
};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{CommittedBlockProof, CommittedBlockProofError, VerifiedBlockTip},
    state_store::ShardScopedTreeStoreReader,
};

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
    /// Per-shard roots in the canonical order the block header commits them: [global, shard_0, ...].
    ordered_roots: Vec<TreeHash>,
    /// The committed state of each shard in `ordered_roots`, keyed by shard.
    shards: HashMap<Shard, CommittedShardState>,
    /// Level-2 proofs, computed on first use of each shard. They all come out of the same tree over
    /// `ordered_roots`; only the leaf extracted from it differs.
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
            ordered_roots,
            shards,
            shard_root_proofs: HashMap::new(),
        })
    }

    /// Proves `versioned_id`'s committed value, or its absence, against the shard-group root.
    pub fn generate(&mut self, versioned_id: &VersionedSubstateId) -> Result<SubstateValueProof, StorageError> {
        let shard = versioned_id.to_shard(self.num_preshards);
        let Some(state) = self.shards.get(&shard).copied() else {
            return Err(StorageError::QueryError {
                reason: format!("generate_substate_proof: {versioned_id} is in {shard}, outside this shard group"),
            });
        };
        let version = state.version.ok_or_else(|| StorageError::QueryError {
            reason: format!("generate_substate_proof: shard {shard} has no committed state"),
        })?;

        // Level 1: leaf proof (inclusion or exclusion) for the substate within its shard.
        let mut scoped = ShardScopedTreeStoreReader::new(self.tx, shard);
        let tree = SpreadPrefixStateTree::new(&mut scoped);
        let (_leaf_key, _proof_value, leaf_proof) =
            tree.get_proof(version, versioned_id)
                .map_err(|e| StorageError::QueryError {
                    reason: format!("generate_substate_proof get_proof: {e}"),
                })?;

        // Level 2: prove the shard root is committed in the shard-group root.
        let shard_root_proof = match self.shard_root_proofs.entry(shard) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let (_, proof) =
                    compute_proof_for_hashes(self.ordered_roots.iter().copied(), state.root).map_err(|e| {
                        StorageError::QueryError {
                            reason: format!("generate_substate_proof shard root proof: {e}"),
                        }
                    })?;
                entry.insert(proof).clone()
            },
        };

        Ok(SubstateValueProof::new(state.root, shard_root_proof, leaf_proof))
    }
}

/// Generates a two-level [`SubstateValueProof`] for a single `versioned_id` against the latest
/// committed shard-group state. See [`SubstateProofGenerator`], which proves many substates against
/// the same state without repeating the per-shard-group work.
pub fn generate_substate_proof<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    shard_group: ShardGroup,
    versioned_id: &VersionedSubstateId,
    num_preshards: NumPreshards,
) -> Result<SubstateValueProof, StorageError> {
    SubstateProofGenerator::new(tx, shard_group, num_preshards)?.generate(versioned_id)
}

/// Verifies a substate value proof served by a (possibly untrusted) validator.
///
/// 1. The commit proof is validated against the shard group committee, yielding a trusted shard-group state merkle
///    root.
/// 2. The substate value proof is verified against that root - an inclusion proof for an up substate (binding the
///    returned `value` by re-deriving its leaf value hash), or an exclusion proof for a down substate (`value` is
///    `None`).
///
/// `check_vn` must return the voting power of the given validator in the committee for the commit
/// proof's epoch/shard group (zero if not a member); the caller is responsible for fetching that
/// committee (see [`CommittedBlockProof::epoch`]/[`CommittedBlockProof::shard_group`]).
#[allow(clippy::too_many_arguments)]
pub fn verify_substate_value_proof(
    commit_proof: &CommittedBlockProof,
    value_proof_bytes: &[u8],
    substate_id: &SubstateId,
    version: u32,
    value: Option<&SubstateValue>,
    network: Network,
    proof_epoch: Epoch,
    quorum_threshold: VotePower,
    check_vn: impl Fn(&RistrettoPublicKeyBytes) -> Result<VotePower, SidechainProofValidationError>,
) -> Result<VerifiedBlockTip, SubstateProofVerifyError> {
    // Anchor: a quorum-signed shard-group state merkle root.
    let verified_tip = commit_proof.validate(quorum_threshold, check_vn)?;
    verify_substate_value_proof_against_root(
        value_proof_bytes,
        substate_id,
        version,
        value,
        network,
        proof_epoch,
        verified_tip.state_merkle_root,
    )?;
    Ok(verified_tip)
}

/// Verifies a substate value proof against an *already-trusted* shard-group state merkle root,
/// skipping commit-proof (QC chain) validation.
///
/// This is the inner half of [`verify_substate_value_proof`] for callers that have independently
/// established `trusted_root` - e.g. from a commit proof that was committee-validated in an earlier
/// round and recorded in a trusted-root store. Soundness is identical to the full path: the
/// validator pins the substate value proof to the same committed block whose `state_merkle_root` we
/// trust (proof and commit proof are generated in one read transaction against the same committed
/// block), so verifying the proof against that root is exactly what [`verify_substate_value_proof`]
/// does after `validate()`. The trust decision must therefore be keyed on `trusted_root` itself: a
/// node cannot forge a substate proof that verifies against a root a quorum already signed.
pub fn verify_substate_value_proof_against_root(
    value_proof_bytes: &[u8],
    substate_id: &SubstateId,
    version: u32,
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
    #[error("commit proof invalid: {0}")]
    CommitProof(#[from] CommittedBlockProofError),
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
        reason: format!("generate_substate_proof shard {shard} root: {e}"),
    })?;
    Ok(CommittedShardState {
        root,
        version: Some(version),
    })
}
