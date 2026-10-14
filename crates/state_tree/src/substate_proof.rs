//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_engine_types::ProtocolVersion;
use tari_jellyfish::{JmtHashScheme, SparseMerkleProofExt, TreeHash, Version};
use tari_ootle_common_types::{NumPreshards, VersionedSubstateId};

use crate::{
    ShardGroupLeaf,
    key_mapper::{DbKeyMapper, SpreadPrefixKeyMapper},
};

/// A two-level Merkle proof binding a single substate to a shard-group state Merkle root (the
/// `state_merkle_root` committed in an L2 block header):
///   1. the substate leaf is included in (or excluded from) its shard's JMT, rooted at `shard_root`;
///   2. the shard's state is committed in the shard-group root tree, rooted at the block header's `state_merkle_root`
///      (see [`ShardGroupLeaf`] for how each protocol version lays that tree out).
///
/// Verifying both levels against a *trusted* group root proves the substate's committed value (via
/// [`SubstateValueProof::verify_inclusion`]) or its absence (via
/// [`SubstateValueProof::verify_exclusion`]) without trusting the node that produced the proof. From
/// [`ProtocolVersion::V2`] the verifier derives the substate's shard itself and checks the shard's own
/// leaf, so the proof can only cite that shard's state. The caller is responsible for obtaining a
/// trusted `group_root` (e.g. from a verified committed block proof), for checking that the
/// substate's shard is one of the shards that root's group covers, and, for inclusion, for binding
/// `value_hash` to the returned substate value. Under V2 a shard outside the group has no leaf, the
/// same as an empty shard, so without the group check an exclusion proof against another group's
/// root verifies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstateValueProof {
    /// The substate's shard JMT root.
    pub shard_root: TreeHash,
    /// Proof of the shard's leaf in the shard-group root tree, or under [`ProtocolVersion::V2`] of
    /// its absence when the shard has no state.
    pub shard_root_proof: SparseMerkleProofExt,
    /// Proof for the substate leaf within its shard JMT (inclusion or exclusion).
    pub leaf_proof: SparseMerkleProofExt,
    /// The shard's state version at `shard_root`, which the shard's leaf commits to from
    /// [`ProtocolVersion::V1`].
    #[serde(default)]
    pub shard_state_version: Version,
}

impl SubstateValueProof {
    pub fn new(
        shard_root: TreeHash,
        shard_state_version: Version,
        shard_root_proof: SparseMerkleProofExt,
        leaf_proof: SparseMerkleProofExt,
    ) -> Self {
        Self {
            shard_root,
            shard_root_proof,
            leaf_proof,
            shard_state_version,
        }
    }

    /// Verifies that `versioned_id` is committed with leaf value hash `value_hash` under the trusted
    /// shard-group `group_root`, committed under `protocol_version`.
    pub fn verify_inclusion(
        &self,
        scheme: JmtHashScheme,
        protocol_version: ProtocolVersion,
        group_root: &TreeHash,
        num_preshards: NumPreshards,
        versioned_id: &VersionedSubstateId,
        value_hash: &TreeHash,
    ) -> Result<(), SubstateValueProofError> {
        self.verify_shard_root(scheme, protocol_version, group_root, num_preshards, versioned_id)?;
        let leaf_key = SpreadPrefixKeyMapper::map_to_leaf_key(versioned_id);
        self.leaf_proof
            .verify_inclusion(scheme, &self.shard_root, &leaf_key, value_hash)
            .map_err(|e| SubstateValueProofError::LeafProof(e.to_string()))
    }

    /// Verifies that `versioned_id` is absent (destroyed or never created) under the trusted
    /// shard-group `group_root`, committed under `protocol_version`.
    pub fn verify_exclusion(
        &self,
        scheme: JmtHashScheme,
        protocol_version: ProtocolVersion,
        group_root: &TreeHash,
        num_preshards: NumPreshards,
        versioned_id: &VersionedSubstateId,
    ) -> Result<(), SubstateValueProofError> {
        self.verify_shard_root(scheme, protocol_version, group_root, num_preshards, versioned_id)?;
        let leaf_key = SpreadPrefixKeyMapper::map_to_leaf_key(versioned_id);
        // `shard_root` is authenticated for this shard by the level-2 proof, so the empty-tree root here means the
        // shard holds no substates at all (every one it held was destroyed, or none was ever created).
        self.leaf_proof
            .verify_exclusion_or_empty_tree(scheme, &self.shard_root, &leaf_key)
            .map_err(|e| SubstateValueProofError::LeafProof(e.to_string()))
    }

    /// Level 2: prove the shard-group root tree holds `shard_root` at `shard_state_version` for the
    /// substate's shard.
    ///
    /// A substate is absent from every shard but its own, so an exclusion proof is only meaningful
    /// against its own shard's root. Under [`ProtocolVersion::V2`] the shard is derived from the
    /// substate being proved, never taken from the prover, and a shard with no state is proved by
    /// the absence of its leaf.
    fn verify_shard_root(
        &self,
        scheme: JmtHashScheme,
        protocol_version: ProtocolVersion,
        group_root: &TreeHash,
        num_preshards: NumPreshards,
        versioned_id: &VersionedSubstateId,
    ) -> Result<(), SubstateValueProofError> {
        let shard = versioned_id.to_shard(num_preshards);
        let leaf = ShardGroupLeaf::new(protocol_version, shard, &self.shard_root, self.shard_state_version);
        let result = match leaf.value {
            Some(value) => self
                .shard_root_proof
                .verify_inclusion(scheme, group_root, &leaf.key, &value),
            // `group_root` is trusted for this shard group, so the empty-tree root means no shard of the group holds
            // state.
            None => self
                .shard_root_proof
                .verify_exclusion_or_empty_tree(scheme, group_root, &leaf.key),
        };
        result.map_err(|e| SubstateValueProofError::ShardRootProof(e.to_string()))
    }
}

/// Proves a substate version was committed and is now absent: a two-root proof of destruction.
///
/// An exclusion proof alone shows only that `(id, version)` is absent at one root, which a version that was never
/// created satisfies as well as one that was destroyed. Pairing it with an inclusion proof at an earlier trusted root
/// shows the version existed first. The verifier obtains both roots from quorum-signed commit proofs and checks that
/// the inclusion root is strictly earlier than the exclusion root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstateDownProof {
    /// Level-1 inclusion of `(id, version)` at shard state version `u`, plus level-2 for the shard root at `u`, under
    /// the root `up_commit_proof` commits.
    pub up: SubstateValueProof,
    /// The leaf value hash at `u`. The value itself may have been pruned since it went down.
    pub up_value_hash: TreeHash,
    /// CBOR-encoded `CommittedBlockProof` whose header commits the shard-group root `up` is proved against.
    pub up_commit_proof: Vec<u8>,
    /// Level-1 exclusion of `(id, version)` at the latest shard state version, plus level-2, under the root of the
    /// commit proof the proof is served with.
    pub down: SubstateValueProof,
}

#[derive(Debug, thiserror::Error)]
pub enum SubstateValueProofError {
    #[error("shard-root-in-group-root proof is invalid: {0}")]
    ShardRootProof(String),
    #[error("substate-leaf-in-shard-root proof is invalid: {0}")]
    LeafProof(String),
}
