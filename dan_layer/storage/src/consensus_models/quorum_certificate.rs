//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, ops::Deref};

use log::*;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_dan_common_types::{
    hashing::quorum_certificate_hasher,
    optional::Optional,
    serde_with,
    shard::Shard,
    Epoch,
    NodeHeight,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    consensus_models::{Block, BlockId, HighQc, LastVoted, LeafBlock, QuorumDecision, ValidatorSignature},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

const LOG_TARGET: &str = "tari::dan::storage::quorum_certificate";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct QuorumCertificate {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    qc_id: QcId,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    block_id: BlockId,
    block_height: NodeHeight,
    epoch: Epoch,
    shard: Shard,
    signatures: Vec<ValidatorSignature>,
    #[serde(with = "serde_with::hex::vec")]
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    leaf_hashes: Vec<FixedHash>,
    decision: QuorumDecision,
}

impl QuorumCertificate {
    pub fn new(
        block: BlockId,
        block_height: NodeHeight,
        epoch: Epoch,
        shard: Shard,
        signatures: Vec<ValidatorSignature>,
        mut leaf_hashes: Vec<FixedHash>,
        decision: QuorumDecision,
    ) -> Self {
        leaf_hashes.sort();
        let mut qc = Self {
            qc_id: QcId::genesis(),
            block_id: block,
            block_height,
            epoch,
            shard,
            signatures,
            leaf_hashes,
            decision,
        };
        qc.qc_id = qc.calculate_id();
        qc
    }

    pub fn genesis() -> Self {
        Self::new(
            BlockId::genesis(),
            NodeHeight::zero(),
            Epoch(0),
            Shard::from(0),
            vec![],
            vec![],
            QuorumDecision::Accept,
        )
    }

    pub fn calculate_id(&self) -> QcId {
        quorum_certificate_hasher()
            .chain(&self.epoch)
            .chain(&self.shard)
            .chain(&self.block_id)
            .chain(&self.block_height)
            .chain(&self.signatures)
            .chain(&self.leaf_hashes)
            .chain(&self.decision)
            .result()
            .into()
    }

    pub fn is_valid(&self) -> bool {
        true
    }
}

impl QuorumCertificate {
    pub fn is_genesis(&self) -> bool {
        self.block_id.is_genesis()
    }

    pub fn id(&self) -> &QcId {
        &self.qc_id
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn shard(&self) -> Shard {
        self.shard
    }

    pub fn leaf_hashes(&self) -> &[FixedHash] {
        &self.leaf_hashes
    }

    pub fn signatures(&self) -> &[ValidatorSignature] {
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
            block_height: self.block_height,
            epoch: self.epoch,
            qc_id: self.qc_id,
        }
    }

    pub fn as_leaf_block(&self) -> LeafBlock {
        LeafBlock {
            block_id: self.block_id,
            height: self.block_height,
            epoch: self.epoch,
        }
    }

    pub fn as_last_voted(&self) -> LastVoted {
        LastVoted {
            block_id: self.block_id,
            height: self.block_height,
            epoch: self.epoch,
        }
    }
}

impl QuorumCertificate {
    pub fn get<TTx: StateStoreReadTransaction + ?Sized>(tx: &TTx, qc_id: &QcId) -> Result<Self, StorageError> {
        tx.quorum_certificates_get(qc_id)
    }

    pub fn get_all<'a, TTx: StateStoreReadTransaction + ?Sized, I: IntoIterator<Item = &'a QcId>>(
        tx: &TTx,
        qc_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        tx.quorum_certificates_get_all(qc_ids)
    }

    pub fn get_block<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<Block, StorageError> {
        Block::get(tx, &self.block_id)
    }

    pub fn get_by_block_id<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        block_id: &BlockId,
    ) -> Result<Self, StorageError> {
        tx.quorum_certificates_get_by_block_id(block_id)
    }

    pub fn insert<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.quorum_certificates_insert(self)
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<bool, StorageError> {
        Ok(tx.quorum_certificates_get(&self.qc_id).optional()?.is_some())
    }

    pub fn update_high_qc<TTx>(&self, tx: &mut TTx) -> Result<HighQc, StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref + ?Sized,
        TTx::Target: StateStoreReadTransaction,
    {
        let mut high_qc = HighQc::get(&**tx)?;

        if high_qc.block_height() < self.block_height() {
            debug!(
                target: LOG_TARGET,
                "🔥 UPDATE_HIGH_QC ({}, previous high QC: {} {})",
                self,
                high_qc.block_id(),
                high_qc.block_height(),
            );

            self.save(tx)?;
            // This will fail if the block doesnt exist
            self.as_leaf_block().set(tx)?;
            high_qc = self.as_high_qc();
            high_qc.set(tx)?;
        }

        Ok(high_qc)
    }

    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<bool, StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref + ?Sized,
        TTx::Target: StateStoreReadTransaction,
    {
        if self.exists(&**tx)? {
            return Ok(true);
        }
        self.insert(tx)?;
        Ok(false)
    }
}

impl Display for QuorumCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Qc(block: {} {}, qc_id: {}, epoch: {}, {} signatures)",
            self.block_id,
            self.block_height,
            self.qc_id,
            self.epoch,
            self.signatures.len()
        )
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
