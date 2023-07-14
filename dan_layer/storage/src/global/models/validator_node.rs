//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_dan_common_types::{vn_node_hash, Epoch, NodeAddressable, ShardId};

#[derive(Debug, Clone)]
pub struct ValidatorNode<TAddr> {
    pub address: TAddr,
    pub shard_key: ShardId,
    pub epoch: Epoch,
    pub committee_bucket: Option<u32>,
}

impl<TAddr: NodeAddressable> ValidatorNode<TAddr> {
    pub fn node_hash(&self) -> FixedHash {
        vn_node_hash(&self.address, &self.shard_key)
    }
}
