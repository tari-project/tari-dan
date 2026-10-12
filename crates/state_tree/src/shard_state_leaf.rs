//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{
    ProtocolVersion,
    hashing::{EngineHashDomainLabel, hasher32},
};
use tari_jellyfish::{TreeHash, Version};
use tari_ootle_common_types::shard::Shard;

/// The leaf a shard contributes to its shard group's state merkle root, the root a block header commits to.
///
/// - [`ProtocolVersion::V0`]: the leaf is the shard root itself.
/// - [`ProtocolVersion::V1`]: the leaf commits to the shard's state version as well as its root, so the quorum
///   signature over a header fixes the version each shard of the group is at. `shard` is ignored.
/// - [`ProtocolVersion::V2`]: the leaf also commits to `shard`, so a leaf, and every proof that cites it, belongs to
///   exactly one shard. Two shards with the same root and version (e.g. two empty shards) have different leaves.
pub fn shard_state_leaf(
    protocol_version: ProtocolVersion,
    shard: Shard,
    shard_root: &TreeHash,
    state_version: Version,
) -> TreeHash {
    match protocol_version {
        ProtocolVersion::V0 => *shard_root,
        ProtocolVersion::V1 => {
            let hash = hasher32(EngineHashDomainLabel::ShardStateLeaf)
                .chain(&shard_root.into_array())
                .chain(&state_version)
                .result();
            TreeHash::new(hash.into_array())
        },
        ProtocolVersion::V2 => {
            let hash = hasher32(EngineHashDomainLabel::ShardStateLeaf)
                .chain(&shard)
                .chain(&shard_root.into_array())
                .chain(&state_version)
                .result();
            TreeHash::new(hash.into_array())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v0_leaf_is_the_shard_root() {
        let root = TreeHash::new([1u8; 32]);
        assert_eq!(
            shard_state_leaf(ProtocolVersion::V0, Shard::from_u32(1), &root, 7),
            root
        );
    }

    #[test]
    fn v1_leaf_commits_to_the_state_version() {
        let root = TreeHash::new([1u8; 32]);
        let shard = Shard::from_u32(1);
        let leaf = shard_state_leaf(ProtocolVersion::V1, shard, &root, 7);
        assert_ne!(leaf, root);
        assert_ne!(leaf, shard_state_leaf(ProtocolVersion::V1, shard, &root, 8));
        assert_ne!(
            leaf,
            shard_state_leaf(ProtocolVersion::V1, shard, &TreeHash::new([2u8; 32]), 7)
        );
    }

    /// Roots committed under V1 are live on esmeralda, so its leaf must not move.
    #[test]
    fn v1_leaf_is_unchanged_and_ignores_the_shard() {
        let root = TreeHash::new([1u8; 32]);
        let leaf = shard_state_leaf(ProtocolVersion::V1, Shard::from_u32(1), &root, 7);
        assert_eq!(
            leaf,
            shard_state_leaf(ProtocolVersion::V1, Shard::from_u32(2), &root, 7)
        );
        assert_eq!(leaf.to_string(), V1_GOLDEN_LEAF);
    }

    const V1_GOLDEN_LEAF: &str = "0d0414fb2b7ce928f345caa78d9133627cfe7293841367f769a22d6233061364";

    #[test]
    fn v2_leaf_commits_to_the_shard() {
        let root = TreeHash::new([1u8; 32]);
        let leaf = shard_state_leaf(ProtocolVersion::V2, Shard::from_u32(1), &root, 7);
        assert_ne!(
            leaf,
            shard_state_leaf(ProtocolVersion::V2, Shard::from_u32(2), &root, 7)
        );
        assert_ne!(leaf, shard_state_leaf(ProtocolVersion::V2, Shard::global(), &root, 7));
        assert_ne!(
            leaf,
            shard_state_leaf(ProtocolVersion::V1, Shard::from_u32(1), &root, 7)
        );
        assert_ne!(
            leaf,
            shard_state_leaf(ProtocolVersion::V2, Shard::from_u32(1), &root, 8)
        );
    }
}
