//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{
    hashing::{quorum_certificate_hasher, ValidatorNodeBmtHasherBlake256},
    Epoch,
    NodeHeight,
};
use tari_mmr::MergedBalancedBinaryMerkleProof;

use crate::{
    consensus_models::{Block, BlockId, HighQc, ValidatorSignature},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuorumCertificate {
    block_id: BlockId,
    block_height: NodeHeight,
    epoch: Epoch,
    view_number: u64,
    signatures: Vec<ValidatorSignature>,
    merged_proof: MergedBalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>,
    leaf_hashes: Vec<FixedHash>,
}

impl QuorumCertificate {
    pub fn new(
        block: BlockId,
        block_height: NodeHeight,
        epoch: Epoch,
        view_number: u64,
        signatures: Vec<ValidatorSignature>,
        merged_proof: MergedBalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>,
        mut leaf_hashes: Vec<FixedHash>,
    ) -> Self {
        leaf_hashes.sort();
        Self {
            block_id: block,
            block_height,
            epoch,
            view_number,
            signatures,
            merged_proof,
            leaf_hashes,
        }
    }

    pub fn genesis(epoch: Epoch) -> Self {
        Self {
            block_id: BlockId::genesis(),
            block_height: NodeHeight::zero(),
            epoch,
            view_number: 0,
            signatures: vec![],
            merged_proof: MergedBalancedBinaryMerkleProof::create_from_proofs(vec![]).unwrap(),
            leaf_hashes: vec![],
        }
    }

    pub fn is_genesis(&self) -> bool {
        self.block_id.is_genesis()
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn merged_proof(&self) -> &MergedBalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256> {
        &self.merged_proof
    }

    pub fn leaf_hashes(&self) -> &[FixedHash] {
        &self.leaf_hashes
    }

    pub fn signatures(&self) -> &[ValidatorSignature] {
        &self.signatures
    }

    pub fn view_number(&self) -> u64 {
        self.view_number
    }

    pub fn block_height(&self) -> NodeHeight {
        self.block_height
    }

    pub fn to_hash(&self) -> FixedHash {
        quorum_certificate_hasher()
            .chain(&self.epoch)
            .chain(&self.block_id)
            .chain(&self.view_number)
            .chain(&self.signatures)
            .chain(&self.merged_proof)
            .chain(&self.leaf_hashes)
            .result()
    }

    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn as_high_qc(&self) -> HighQc {
        HighQc {
            epoch: self.epoch,
            block_id: self.block_id,
            height: self.block_height,
        }
    }
}

impl QuorumCertificate {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, block_id: &BlockId) -> Result<Self, StorageError> {
        tx.quorum_certificates_get(block_id)
    }

    pub fn get_block<TTx: StateStoreReadTransaction>(&self, tx: &mut TTx) -> Result<Block, StorageError> {
        Block::get(tx, &self.block_id)
    }

    pub fn set_as_high_qc<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        self.as_high_qc().save(tx)
    }
}
