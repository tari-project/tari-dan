//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use tari_transaction::TransactionId;

use crate::{
    consensus_models::{Block, BlockPledge, QuorumCertificate},
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone)]
pub struct ForeignParkedProposal {
    pub block: Block,
    pub block_pledge: BlockPledge,
    pub justify_qc: QuorumCertificate,
}

impl ForeignParkedProposal {
    pub fn new(block: Block, justify_qc: QuorumCertificate, block_pledge: BlockPledge) -> Self {
        Self {
            block,
            block_pledge,
            justify_qc,
        }
    }

    pub fn block(&self) -> &Block {
        &self.block
    }

    pub fn block_pledge(&self) -> &BlockPledge {
        &self.block_pledge
    }

    pub fn justify_qc(&self) -> &QuorumCertificate {
        &self.justify_qc
    }
}

impl ForeignParkedProposal {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.foreign_parked_blocks_insert(self)
    }

    pub fn add_missing_transactions<'a, TTx: StateStoreWriteTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        &self,
        tx: &mut TTx,
        transaction_ids: I,
    ) -> Result<(), StorageError> {
        tx.foreign_parked_blocks_insert_missing_transactions(self.block.id(), transaction_ids)
    }

    pub fn remove_by_transaction_id<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        transaction_id: &TransactionId,
    ) -> Result<Vec<Self>, StorageError> {
        tx.foreign_parked_blocks_remove_all_by_transaction(transaction_id)
    }
}

impl Display for ForeignParkedProposal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ForeignParkedBlock: block={}, qcs=", self.block)?;
        for (_tx_id, pledges) in self.block_pledge().iter() {
            write!(f, "{_tx_id}:[")?;
            for pledge in pledges {
                write!(f, "{pledge}, ")?;
            }
            write!(f, "],")?;
        }
        write!(f, "justify_qc={}", self.justify_qc)?;
        Ok(())
    }
}
