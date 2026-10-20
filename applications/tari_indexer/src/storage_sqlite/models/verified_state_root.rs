//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup};
use tari_ootle_storage::consensus_models::VerifiedBlockTip;
use tari_sidechain::FixedHash;

use crate::storage_sqlite::schema::verified_state_roots;

#[derive(Debug, Clone, Queryable)]
#[diesel(table_name = verified_state_roots)]
pub(crate) struct VerifiedStateRootRow {
    #[allow(dead_code)]
    pub id: i32,
    // Redundant with the query filter; reads echo the caller's epoch/shard_group rather than re-parse.
    #[allow(dead_code)]
    pub epoch: i64,
    #[allow(dead_code)]
    pub shard_group: String,
    pub block_height: i64,
    pub block_hash: String,
    pub state_merkle_root: String,
    #[allow(dead_code)]
    pub validated_at: tari_ootle_storage::time::PrimitiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = verified_state_roots)]
pub(crate) struct NewVerifiedStateRoot {
    pub epoch: i64,
    pub shard_group: String,
    pub block_height: i64,
    pub block_hash: String,
    pub state_merkle_root: String,
}

/// A committee-validated state merkle root at a known committed block height within an epoch's shard
/// group chain. Mirrors [`VerifiedBlockTip`] with the block id added for diagnostics/audit.
#[derive(Debug, Clone)]
pub struct VerifiedStateRoot {
    pub epoch: Epoch,
    pub shard_group: ShardGroup,
    pub height: NodeHeight,
    pub block_hash: FixedHash,
    pub state_merkle_root: FixedHash,
}

impl VerifiedStateRoot {
    pub fn from_verified_tip(tip: &VerifiedBlockTip) -> Self {
        Self {
            epoch: tip.epoch,
            shard_group: tip.shard_group,
            height: tip.height,
            block_hash: tip.block_id,
            state_merkle_root: tip.state_merkle_root,
        }
    }
}
