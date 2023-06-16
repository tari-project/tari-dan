//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, ShardId};

#[derive(Debug, Clone)]
pub struct ValidatorNode {
    pub public_key: PublicKey,
    pub shard_key: ShardId,
    pub epoch: Epoch,
    pub committee_bucket: Option<u32>,
}
