//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_bor::borsh::BorshSerialize;

use crate::{PayloadId, ShardId, SubstateState, TreeNodeHash};

#[derive(Debug, Clone, Deserialize, Serialize, BorshSerialize)]
pub struct ObjectPledge {
    pub shard_id: ShardId,
    pub current_state: SubstateState,
    pub pledged_to_payload: PayloadId,
}

#[derive(Debug, Clone)]
pub struct ObjectPledgeInfo {
    pub shard_id: ShardId,
    pub pledged_to_payload_id: PayloadId,
    pub completed_by_tree_node_hash: Option<TreeNodeHash>,
    pub abandoned_by_tree_node_hash: Option<TreeNodeHash>,
    pub is_active: bool,
}
