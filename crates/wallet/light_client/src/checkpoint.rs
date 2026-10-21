//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::{Epoch, ShardGroup, StateVersion, shard::Shard};
use tari_ootle_storage::consensus_models::EpochCheckpoint;

/// An end-of-epoch checkpoint that a verified committee of its epoch and shard group signed. Its state merkle root
/// commits to the state version of every shard in the group, so each version it reports is one the network committed.
///
/// The light client builds one only through
/// [`VerifiedCommittee::verify_checkpoint`](crate::VerifiedCommittee::verify_checkpoint).
#[derive(Debug, Clone)]
pub struct VerifiedCheckpoint {
    shard_group: ShardGroup,
    checkpoint: EpochCheckpoint,
}

impl VerifiedCheckpoint {
    pub(crate) fn new(shard_group: ShardGroup, checkpoint: EpochCheckpoint) -> Self {
        Self {
            shard_group,
            checkpoint,
        }
    }

    /// The epoch the checkpoint ends.
    pub fn epoch(&self) -> Epoch {
        self.checkpoint.epoch()
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.shard_group
    }

    /// The state version `shard` reached at the end of the epoch, or `None` for a shard outside the checkpoint's
    /// shard group.
    pub fn shard_state_version(&self, shard: Shard) -> Option<StateVersion> {
        self.shard_group
            .contains(&shard)
            .then(|| StateVersion::from(self.checkpoint.get_shard_state_version(shard)))
    }
}
