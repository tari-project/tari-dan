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
/// The leaf commits to the shard's state version as well as its root, so the quorum signature over a header fixes the
/// version each shard of the group is at, and to `shard`, so a leaf, and every proof that cites it, belongs to exactly
/// one shard. Two shards with the same root and version (e.g. two empty shards) have different leaves.
pub fn shard_state_leaf(
    protocol_version: ProtocolVersion,
    shard: Shard,
    shard_root: &TreeHash,
    state_version: Version,
) -> TreeHash {
    match protocol_version {
        ProtocolVersion::V0 => {
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
    fn the_leaf_commits_to_the_shard_root_and_state_version() {
        let root = TreeHash::new([1u8; 32]);
        let shard = Shard::from_u32(1);
        let leaf = shard_state_leaf(ProtocolVersion::V0, shard, &root, 7);
        assert_ne!(leaf, root);
        assert_ne!(leaf, shard_state_leaf(ProtocolVersion::V0, shard, &root, 8));
        assert_ne!(
            leaf,
            shard_state_leaf(ProtocolVersion::V0, shard, &TreeHash::new([2u8; 32]), 7)
        );
    }

    #[test]
    fn the_leaf_commits_to_the_shard() {
        let root = TreeHash::new([1u8; 32]);
        let leaf = shard_state_leaf(ProtocolVersion::V0, Shard::from_u32(1), &root, 7);
        assert_ne!(
            leaf,
            shard_state_leaf(ProtocolVersion::V0, Shard::from_u32(2), &root, 7)
        );
        assert_ne!(leaf, shard_state_leaf(ProtocolVersion::V0, Shard::global(), &root, 7));
    }
}
