//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use digest::Digest;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_crypto::hash::blake2::Blake256;
use tari_dan_common_types::ShardId;
use tari_utilities::ByteArray;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ValidatorSignature {
    pub public_key: PublicKey,
    pub shard_id: ShardId,
    pub signature: Signature,
}

impl ValidatorSignature {
    pub fn new(public_key: PublicKey, shard_id: ShardId, signature: Signature) -> Self {
        Self {
            public_key,
            shard_id,
            signature,
        }
    }

    pub fn verify(&self) -> bool {
        let challenge = self.create_challenge();
        self.signature.verify_challenge(&self.public_key, &*challenge)
    }

    pub fn create_challenge(&self) -> FixedHash {
        Blake256::new()
            .chain(self.public_key.as_bytes())
            .chain(self.shard_id.as_bytes())
            .finalize()
            .into()
    }
}
