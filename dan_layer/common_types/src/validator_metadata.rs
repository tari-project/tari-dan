//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{io, io::Write};

use borsh::BorshSerialize;
use digest::Digest;
use serde::Deserialize;
use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_crypto::hash::blake2::Blake256;
use tari_mmr::BalancedBinaryMerkleProof;

use crate::{serde_with, NodeAddressable, ShardId, ValidatorNodeBmtHasherBlake256};

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct ValidatorMetadata {
    pub public_key: PublicKey,
    #[serde(with = "serde_with::hex")]
    pub vn_shard_key: ShardId,
    pub signature: Signature,
    pub merkle_proof: BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>,
    pub merkle_leaf_index: u64,
}

impl ValidatorMetadata {
    pub fn new(
        public_key: PublicKey,
        vn_shard_key: ShardId,
        signature: Signature,
        merkle_proof: BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>,
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
        vn_bmt_node_hash(&self.public_key, &self.vn_shard_key)
    }

    // TODO: impl Borsh for merkle proof
    pub fn encode_merkle_proof(&self) -> Vec<u8> {
        bincode::serialize(&self.merkle_proof).unwrap()
    }

    // TODO: impl Borsh for merkle proof
    pub fn decode_merkle_proof(
        bytes: &[u8],
    ) -> Result<BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>, io::Error> {
        // Map to an io error because borsh uses that
        bincode::deserialize(bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

pub fn vn_bmt_node_hash<TAddr: NodeAddressable>(public_key: &TAddr, shard_id: &ShardId) -> FixedHash {
    Blake256::new()
        .chain(public_key.as_bytes())
        .chain(shard_id.as_bytes())
        .finalize()
        .into()
}

impl BorshSerialize for ValidatorMetadata {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.public_key.serialize(writer)?;
        self.vn_shard_key.serialize(writer)?;
        self.signature.serialize(writer)?;
        // TODO: MerkleProof should implement borsh
        // self.merkle_proof.serialize(writer)
        self.merkle_leaf_index.serialize(writer)?;
        Ok(())
    }
}
