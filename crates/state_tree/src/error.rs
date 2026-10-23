//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_jellyfish::{JmtStorageError, TreeHash, Version};
use tari_ootle_common_types::{SubstateAddress, optional::IsNotFoundError, shard::Shard};

#[derive(Debug, thiserror::Error)]
pub enum StateTreeError {
    #[error("JMT Storage error: {0}")]
    JmtStorageError(#[from] JmtStorageError),
    #[error(
        "Refusing to write state tree version {next_version} on top of current version {current_version}: the next \
         version must be greater than the current version"
    )]
    NonMonotonicVersion {
        current_version: Version,
        next_version: Version,
    },
    #[error("Two leaves share the key {key}, so a keyed tree cannot hold both")]
    DuplicateLeafKey { key: TreeHash },
    #[error("{shard} is not one of the shards the shard group root tree was built over")]
    ShardNotInShardGroupTree { shard: Shard },
    #[error("{shard} appears more than once in the shard states a shard group root tree is built over")]
    DuplicateShardInShardGroupTree { shard: Shard },
    #[error(
        "Refusing to write state tree changes on top of version {current_version}: it is the last version a state \
         tree can hold"
    )]
    VersionExhausted { current_version: Version },
    #[error("A state version holds at most one transition of each kind per substate address, but two are at {address}")]
    DuplicateTransition { address: SubstateAddress },
}

impl IsNotFoundError for StateTreeError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, StateTreeError::JmtStorageError(JmtStorageError::NotFound(_)))
    }
}
