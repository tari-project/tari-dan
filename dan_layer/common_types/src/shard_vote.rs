//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_bor::borsh::BorshSerialize;

use crate::{object_pledge::ObjectPledge, ShardId, TreeNodeHash};

#[derive(Debug, Clone, Deserialize, Serialize, BorshSerialize)]
pub struct ShardVote {
    pub shard_id: ShardId,
    pub node_hash: TreeNodeHash,
    pub pledge: Option<ObjectPledge>,
}
