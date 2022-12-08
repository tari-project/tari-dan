//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_bor::borsh::BorshSerialize;

use crate::{PayloadId, ShardId, SubstateState};

#[derive(Debug, Clone, Deserialize, Serialize, BorshSerialize)]
pub struct ObjectPledge {
    pub shard_id: ShardId,
    pub current_state: SubstateState,
    // pub current_state_hash: FixedHash,
    pub pledged_to_payload: PayloadId,
}
