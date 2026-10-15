//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeMap, btree_map::Entry};

use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_engine_types::limits::MAX_CBOR_NESTING_DEPTH;
use tari_jellyfish::{JmtHashScheme, LeafKey, SparseMerkleProofExt, TreeHash};
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    ShardGroup,
    ValidatorSetError,
    ValidatorSetHash,
    ValidatorSetMember,
    committee::Committee,
    hash_validator_set_committee,
    validator_set_committee_key,
};
use tari_template_lib_types::Hash32;

use crate::{KeyedProofTree, StateTreeError};

/// The scheme of every validator set tree, fixed because [`KeyedProofTree`] builds with it.
const VALIDATOR_SET_HASH_SCHEME: JmtHashScheme = JmtHashScheme::V1;

/// An epoch's validator set as a Merkle tree with one leaf per shard group: the leaf for shard group `sg` sits at
/// [`validator_set_committee_key`]`(sg)` and holds the committee's [`hash_validator_set_committee`]. The tree's root,
/// with the network, epoch and preshard count, gives the [`ValidatorSetHash`].
///
/// The tree holds only the leaf hashes; a proof takes the one committee it proves from the caller.
pub struct ValidatorSetTree {
    network: Network,
    epoch: Epoch,
    num_preshards: NumPreshards,
    leaves: BTreeMap<ShardGroup, Hash32>,
    tree: KeyedProofTree,
    hash: ValidatorSetHash,
}

impl ValidatorSetTree {
    /// Builds the tree over each shard group's committee leaf hash. Fails if a shard group appears more than once.
    pub fn build<I: IntoIterator<Item = (ShardGroup, Hash32)>>(
        network: Network,
        epoch: Epoch,
        num_preshards: NumPreshards,
        leaves: I,
    ) -> Result<Self, ValidatorSetTreeError> {
        let mut leaves_by_shard_group = BTreeMap::new();
        for (shard_group, leaf_hash) in leaves {
            let Entry::Vacant(entry) = leaves_by_shard_group.entry(shard_group) else {
                return Err(ValidatorSetTreeError::DuplicateShardGroup { shard_group, epoch });
            };
            entry.insert(leaf_hash);
        }
        let tree = KeyedProofTree::build(
            leaves_by_shard_group
                .iter()
                .map(|(shard_group, leaf_hash)| (committee_leaf_key(*shard_group), to_tree_hash(leaf_hash))),
        )?;
        let hash = ValidatorSetHash::calculate(
            network,
            epoch,
            num_preshards,
            &Hash32::from_array(tree.root().into_array()),
        );
        Ok(Self {
            network,
            epoch,
            num_preshards,
            leaves: leaves_by_shard_group,
            tree,
            hash,
        })
    }

    pub fn hash(&self) -> ValidatorSetHash {
        self.hash
    }

    pub fn committee_root(&self) -> TreeHash {
        self.tree.root()
    }

    /// Proves `committee` as `shard_group`'s committee against [`Self::hash`]. Fails unless `committee` hashes to the
    /// leaf the tree holds for `shard_group`.
    pub fn committee_proof<TAddr>(
        &self,
        shard_group: &ShardGroup,
        committee: &Committee<TAddr>,
    ) -> Result<ValidatorSetCommitteeProof, ValidatorSetTreeError> {
        let committed_leaf = self
            .leaves
            .get(shard_group)
            .ok_or(ValidatorSetTreeError::ShardGroupNotInSet {
                shard_group: *shard_group,
                epoch: self.epoch,
            })?;
        if committee.calculate_validator_set_leaf_hash(shard_group)? != *committed_leaf {
            return Err(ValidatorSetTreeError::CommitteeDoesNotMatchLeaf {
                shard_group: *shard_group,
                epoch: self.epoch,
            });
        }
        let (_, proof) = self.tree.get_proof(&committee_leaf_key(*shard_group))?;
        Ok(ValidatorSetCommitteeProof {
            network: self.network,
            epoch: self.epoch,
            num_preshards: self.num_preshards,
            committee_root: self.tree.root(),
            shard_group: *shard_group,
            members: committee
                .into_iter()
                .map(|member| ValidatorSetMember {
                    public_key: member.public_key,
                    vote_power: member.vote_power,
                })
                .collect(),
            proof,
        })
    }
}

fn committee_leaf_key(shard_group: ShardGroup) -> LeafKey {
    LeafKey::new(to_tree_hash(&validator_set_committee_key(&shard_group)))
}

fn to_tree_hash(hash: &Hash32) -> TreeHash {
    TreeHash::new(hash.into_array())
}

#[derive(Debug, thiserror::Error)]
pub enum ValidatorSetTreeError {
    #[error(transparent)]
    StateTree(#[from] StateTreeError),
    #[error(transparent)]
    Committee(#[from] ValidatorSetError),
    #[error("Shard group {shard_group} appears more than once in the epoch {epoch} validator set leaves")]
    DuplicateShardGroup { shard_group: ShardGroup, epoch: Epoch },
    #[error("Shard group {shard_group} has no committee in the epoch {epoch} validator set")]
    ShardGroupNotInSet { shard_group: ShardGroup, epoch: Epoch },
    #[error(
        "The committee loaded for shard group {shard_group} does not hash to the leaf the epoch {epoch} validator set \
         holds for it"
    )]
    CommitteeDoesNotMatchLeaf { shard_group: ShardGroup, epoch: Epoch },
}

/// One shard group's committee and the Merkle proof that it is that shard group's committee in the validator set a
/// [`ValidatorSetHash`] commits to.
///
/// Every field is untrusted until [`Self::verify`] succeeds against a hash from an authenticated source, such as a
/// quorum-certified end-of-epoch header. Verification authenticates every field but `proof`, so after it each of them
/// is the committed value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorSetCommitteeProof {
    pub network: Network,
    pub epoch: Epoch,
    pub num_preshards: NumPreshards,
    pub committee_root: TreeHash,
    pub shard_group: ShardGroup,
    /// Ascending by public key.
    pub members: Vec<ValidatorSetMember>,
    pub proof: SparseMerkleProofExt,
}

impl ValidatorSetCommitteeProof {
    /// Decodes a proof that [`Self::to_bytes`] encoded.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ValidatorSetProofError> {
        tari_bor::serde_codec::from_slice_with_max_depth(bytes, MAX_CBOR_NESTING_DEPTH)
            .map_err(|e| ValidatorSetProofError::Decode(e.to_string()))
    }

    /// The proof as CBOR, the form it is served in.
    pub fn to_bytes(&self) -> Vec<u8> {
        tari_bor::serde_codec::to_vec(self).expect("ValidatorSetCommitteeProof serialization is infallible")
    }

    /// The validator set hash this proof proves the committee against. Untrusted until [`Self::verify`] succeeds
    /// against a trusted hash.
    pub fn validator_set_hash(&self) -> ValidatorSetHash {
        ValidatorSetHash::calculate(
            self.network,
            self.epoch,
            self.num_preshards,
            &Hash32::from_array(self.committee_root.into_array()),
        )
    }

    /// Returns the members once they are proved to be `shard_group`'s committee in the set `expected` commits to.
    pub fn verify(&self, expected: &ValidatorSetHash) -> Result<&[ValidatorSetMember], ValidatorSetProofError> {
        let computed = self.validator_set_hash();
        if computed != *expected {
            return Err(ValidatorSetProofError::SetHashMismatch {
                expected: *expected,
                computed,
            });
        }

        let leaf_hash = hash_validator_set_committee(
            &self.shard_group,
            self.members
                .iter()
                .map(|member| (&member.public_key, member.vote_power)),
        )?;
        self.proof
            .verify_inclusion(
                VALIDATOR_SET_HASH_SCHEME,
                &self.committee_root,
                &committee_leaf_key(self.shard_group),
                &to_tree_hash(&leaf_hash),
            )
            .map_err(|e| ValidatorSetProofError::CommitteeNotInSet {
                shard_group: self.shard_group,
                epoch: self.epoch,
                details: e.to_string(),
            })?;
        Ok(&self.members)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidatorSetProofError {
    #[error("Committee proof failed to decode: {0}")]
    Decode(String),
    #[error(
        "Committee proof names a validator set with hash {computed}, but the trusted validator set hash is {expected}"
    )]
    SetHashMismatch {
        expected: ValidatorSetHash,
        computed: ValidatorSetHash,
    },
    #[error("Committee proof carries a malformed committee: {0}")]
    MalformedCommittee(#[from] ValidatorSetError),
    #[error(
        "Committee proof does not show the given members as shard group {shard_group}'s committee in the epoch \
         {epoch} validator set: {details}"
    )]
    CommitteeNotInSet {
        shard_group: ShardGroup,
        epoch: Epoch,
        details: String,
    },
}
