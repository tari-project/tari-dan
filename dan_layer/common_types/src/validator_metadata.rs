//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io;

use digest::Digest;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_crypto::hash::blake2::Blake256;
use tari_mmr::MerkleProof;
use tari_utilities::ByteArray;

use crate::{serde_with, NodeAddressable, ShardId};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ValidatorMetadata {
    pub public_key: PublicKey,
    #[serde(with = "serde_with::hex")]
    pub vn_shard_key: ShardId,
    pub signature: Signature,
    pub merkle_proof: MerkleProof,
    pub merkle_leaf_index: u64,
}

impl ValidatorMetadata {
    pub fn new(
        public_key: PublicKey,
        vn_shard_key: ShardId,
        signature: Signature,
        merkle_proof: MerkleProof,
        merkle_leaf_index: u64,
    ) -> Self {
        Self {
            public_key,
            vn_shard_key,
            signature,
            merkle_proof,
            merkle_leaf_index,
        }
    }

    pub fn get_node_hash(&self) -> FixedHash {
        // Each node is defined as H(V_i || S_i)
        vn_mmr_node_hash(&self.public_key, &self.vn_shard_key)
    }

    // TODO: impl Borsh for merkle proof
    pub fn encode_merkle_proof(&self) -> Vec<u8> {
        bincode::serialize(&self.merkle_proof).unwrap()
    }

    // TODO: impl Borsh for merkle proof
    pub fn decode_merkle_proof(bytes: &[u8]) -> Result<MerkleProof, io::Error> {
        // Map to an io error because borsh uses that
        bincode::deserialize(bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    // TODO: once this type implements borsh we can use the consensus hashing to hash this type directly
    pub fn to_bytes(&self) -> Vec<u8> {
        [
            self.public_key.to_vec(),
            self.vn_shard_key.as_bytes().to_vec(),
            self.signature.get_public_nonce().to_vec(),
            self.signature.get_signature().to_vec(),
            self.encode_merkle_proof(),
            self.merkle_leaf_index.to_le_bytes().to_vec(),
        ]
        .concat()
    }
}

pub fn vn_mmr_node_hash<TAddr: NodeAddressable>(public_key: &TAddr, shard_id: &ShardId) -> FixedHash {
    Blake256::new()
        .chain(public_key.as_bytes())
        .chain(shard_id.as_bytes())
        .finalize()
        .into()
}
