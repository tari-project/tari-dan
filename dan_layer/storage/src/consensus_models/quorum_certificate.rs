//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, ops::DerefMut};

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_dan_common_types::{
    hashing::{
        quorum_certificate_hasher,
        MergedValidatorNodeMerkleProof,
        ValidatorNodeBalancedMerkleTree,
        ValidatorNodeBmtHasherBlake256,
        ValidatorNodeMerkleProof,
    },
    optional::Optional,
    serde_with,
    Epoch,
    NodeHeight,
};
use tari_mmr::MergedBalancedBinaryMerkleProof;

use crate::{
    consensus_models::{Block, BlockId, HighQc, LeafBlock, QuorumDecision, ValidatorSignature},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuorumCertificate<TAddr> {
    qc_id: QcId,
    block_id: BlockId,
    block_height: NodeHeight,
    epoch: Epoch,
    signatures: Vec<ValidatorSignature<TAddr>>,
    merged_proof: MergedValidatorNodeMerkleProof,
    #[serde(with = "serde_with::hex::vec")]
    leaf_hashes: Vec<FixedHash>,
    decision: QuorumDecision,
}

impl<TAddr: Serialize> QuorumCertificate<TAddr> {
    pub fn new(
        block: BlockId,
        block_height: NodeHeight,
        epoch: Epoch,
        signatures: Vec<ValidatorSignature<TAddr>>,
        merged_proof: MergedBalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>,
        mut leaf_hashes: Vec<FixedHash>,
        decision: QuorumDecision,
    ) -> Self {
        leaf_hashes.sort();
        let mut qc = Self {
            qc_id: QcId::genesis(),
            block_id: block,
            block_height,
            epoch,
            signatures,
            merged_proof,
            leaf_hashes,
            decision,
        };
        qc.qc_id = qc.calculate_id();
        qc
    }

    pub fn genesis() -> Self {
        // TODO: Should be easy to create an empty proof. Nice to have: decoupled proof.
        let bmt = ValidatorNodeBalancedMerkleTree::create(vec![]);
        let proof = ValidatorNodeMerkleProof::generate_proof(&bmt, 0).unwrap();
        let merged_proof = MergedBalancedBinaryMerkleProof::create_from_proofs(&[proof]).unwrap();
        Self::new(
            BlockId::genesis(),
            NodeHeight::zero(),
            Epoch(0),
            vec![],
            merged_proof,
            vec![],
            QuorumDecision::Accept,
        )
    }

    pub fn calculate_id(&self) -> QcId {
        quorum_certificate_hasher()
            .chain(&self.epoch)
            .chain(&self.block_id)
            .chain(&self.block_height)
            .chain(&self.signatures)
            .chain(&self.merged_proof)
            .chain(&self.leaf_hashes)
            .chain(&self.decision)
            .result()
            .into()
    }
}

impl<TAddr> QuorumCertificate<TAddr> {
    pub fn is_genesis(&self) -> bool {
        self.block_id.is_genesis()
    }

    pub fn id(&self) -> &QcId {
        &self.qc_id
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

    pub fn signatures(&self) -> &[ValidatorSignature<TAddr>] {
        &self.signatures
    }

    pub fn block_height(&self) -> NodeHeight {
        self.block_height
    }

    pub fn decision(&self) -> QuorumDecision {
        self.decision
    }

    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn as_high_qc(&self) -> HighQc {
        HighQc {
            block_id: self.block_id,
            qc_id: self.qc_id,
        }
    }

    pub fn as_leaf_block(&self) -> LeafBlock {
        LeafBlock {
            block_id: self.block_id,
            height: self.block_height,
        }
    }
}

impl<TAddr> QuorumCertificate<TAddr> {
    pub fn get<TTx: StateStoreReadTransaction<Addr = TAddr> + ?Sized>(
        tx: &mut TTx,
        qc_id: &QcId,
    ) -> Result<Self, StorageError> {
        tx.quorum_certificates_get(qc_id)
    }

    pub fn get_block<TTx: StateStoreReadTransaction + ?Sized>(
        &self,
        tx: &mut TTx,
    ) -> Result<Block<TTx::Addr>, StorageError> {
        Block::get(tx, &self.block_id)
    }

    pub fn insert<TTx: StateStoreWriteTransaction<Addr = TAddr>>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.quorum_certificates_insert(self)
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<bool, StorageError> {
        Ok(tx.quorum_certificates_get(&self.qc_id).optional()?.is_some())
    }

    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<bool, StorageError>
    where
        TTx: StateStoreWriteTransaction<Addr = TAddr> + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        if self.exists(tx.deref_mut())? {
            return Ok(true);
        }
        self.insert(tx)?;
        Ok(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QcId(#[serde(with = "serde_with::hex")] FixedHash);

impl QcId {
    pub const fn genesis() -> Self {
        Self(FixedHash::zero())
    }

    pub fn new<T: Into<FixedHash>>(hash: T) -> Self {
        Self(hash.into())
    }

    pub const fn hash(&self) -> &FixedHash {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn is_genesis(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }
}

impl AsRef<[u8]> for QcId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl From<FixedHash> for QcId {
    fn from(value: FixedHash) -> Self {
        Self(value)
    }
}

impl TryFrom<Vec<u8>> for QcId {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        FixedHash::try_from(value).map(Self)
    }
}

impl Display for QcId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}
