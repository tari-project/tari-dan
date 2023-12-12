//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use blake2::{
    digest::{consts::U32, Digest},
    Blake2b,
};
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_crypto::tari_utilities::ByteArray;

use crate::ShardId;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ValidatorMetadata {
    pub public_key: PublicKey,
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
}

pub fn vn_node_hash(public_key: &PublicKey, shard_id: &ShardId) -> FixedHash {
    Blake2b::<U32>::new()
        .chain_update(public_key.as_bytes())
        .chain_update(shard_id.as_bytes())
        .finalize()
        .into()
}
