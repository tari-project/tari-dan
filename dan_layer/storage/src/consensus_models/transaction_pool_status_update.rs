//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_transaction::TransactionId;

use crate::{
    consensus_models::{BlockId, Decision, Evidence, TransactionPoolRecord, TransactionPoolStage},
    StateStoreWriteTransaction,
};

#[derive(Debug, Clone)]
pub struct TransactionPoolStatusUpdate {
    pub transaction: TransactionPoolRecord,
}

impl TransactionPoolStatusUpdate {
    pub fn transaction_id(&self) -> &TransactionId {
        self.transaction.transaction_id()
    }

    pub fn stage(&self) -> TransactionPoolStage {
        self.transaction.current_stage()
    }

    pub fn evidence(&self) -> &Evidence {
        self.transaction.evidence()
    }

    pub fn evidence_mut(&mut self) -> &mut Evidence {
        self.transaction.evidence_mut()
    }

    pub fn is_ready(&self) -> bool {
        self.transaction.is_ready()
    }

    pub fn decision(&self) -> Decision {
        self.transaction.current_decision()
    }

    pub fn remote_decision(&self) -> Option<Decision> {
        self.transaction.remote_decision()
    }

    pub fn apply_evidence(&self, tx_rec_mut: &mut TransactionPoolRecord) {
        tx_rec_mut.set_evidence(self.evidence().clone());
    }
}

impl TransactionPoolStatusUpdate {
    pub fn insert_for_block<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<(), crate::StorageError> {
        tx.transaction_pool_add_pending_update(block_id, self)
    }
}
