//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_dan_storage::consensus_models::BlockId;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoteMessage {
    pub block_id: BlockId,
}
