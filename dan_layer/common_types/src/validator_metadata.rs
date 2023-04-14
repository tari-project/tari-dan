//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use digest::Digest;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_crypto::hash::blake2::Blake256;
use tari_engine_types::serde_with;
use tari_mmr::BalancedBinaryMerkleProof;

use crate::{NodeAddressable, ShardId};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ValidatorMetadata {
    pub public_key: PublicKey,
    #[serde(with = "serde_with::hex")]
    pub vn_shard_key: ShardId,
    pub signature: Signature,
}

impl ValidatorMetadata {
    pub fn new(public_key: PublicKey, vn_shard_key: ShardId, signature: Signature) -> Self {
        Self {
            public_key,
            vn_shard_key,
            signature,
        }
    }

    pub fn get_node_hash(&self) -> FixedHash {
        // Each node is defined as H(V_i || S_i)
        vn_bmt_node_hash(&self.public_key, &self.vn_shard_key)
    }
}

pub fn vn_bmt_node_hash<TAddr: NodeAddressable>(public_key: &TAddr, shard_id: &ShardId) -> FixedHash {
    Blake256::new()
        .chain(public_key.as_bytes())
        .chain(shard_id.as_bytes())
        .finalize()
        .into()
}
