//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_transaction::TransactionId;

use crate::{
    consensus_models::{BlockId, Decision, Evidence, TransactionPoolStage},
    StateStoreWriteTransaction,
};

#[derive(Debug, Clone)]
pub struct TransactionPoolStatusUpdate {
    pub block_id: BlockId,
    pub transaction_id: TransactionId,
    pub stage: TransactionPoolStage,
    pub evidence: Evidence,
    pub is_ready: bool,
    pub local_decision: Decision,
}

impl TransactionPoolStatusUpdate {
    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    pub fn stage(&self) -> TransactionPoolStage {
        self.stage
    }

    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    pub fn is_ready(&self) -> bool {
        self.is_ready
    }

    pub fn local_decision(&self) -> Decision {
        self.local_decision
    }
}

impl TransactionPoolStatusUpdate {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), crate::StorageError> {
        tx.transaction_pool_add_pending_update(self)
    }
}
