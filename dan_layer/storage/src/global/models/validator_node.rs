//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common::configuration::Network;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{shard::Shard, vn_node_hash, Epoch, NodeAddressable, SubstateAddress};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorNode<TAddr> {
    pub address: TAddr,
    pub public_key: PublicKey,
    pub shard_key: SubstateAddress,
    pub registered_at_base_height: u64,
    pub start_epoch: Epoch,
    pub end_epoch: Epoch,
    pub fee_claim_public_key: PublicKey,
    pub sidechain_id: Option<PublicKey>,
}

impl<TAddr: NodeAddressable> ValidatorNode<TAddr> {
    pub fn get_node_hash(&self, network: Network) -> FixedHash {
        vn_node_hash(network, &self.public_key, &self.shard_key)
    }
}

impl<TAddr> PartialEq for ValidatorNode<TAddr> {
    fn eq(&self, other: &Self) -> bool {
        self.shard_key == other.shard_key
    }
}

impl<TAddr> Eq for ValidatorNode<TAddr> {}

impl<TAddr> std::hash::Hash for ValidatorNode<TAddr> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.shard_key.hash(state);
    }
}
