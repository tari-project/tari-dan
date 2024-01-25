//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{shard::Shard, vn_node_hash, Epoch, NodeAddressable, SubstateAddress};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ValidatorNode<TAddr> {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub address: TAddr,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    pub shard_key: SubstateAddress,
    pub epoch: Epoch,
    pub committee_shard: Option<Shard>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub fee_claim_public_key: PublicKey,
}

impl<TAddr: NodeAddressable> ValidatorNode<TAddr> {
    pub fn node_hash(&self) -> FixedHash {
        vn_node_hash(&self.public_key, &self.shard_key)
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
