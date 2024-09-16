//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_transaction::TransactionId;

use crate::{
    consensus_models::{BlockId, Decision, Evidence, LeaderFee, TransactionPoolRecord, TransactionPoolStage},
    StateStoreWriteTransaction,
};

#[derive(Debug, Clone)]
pub struct TransactionPoolStatusUpdate {
    pub transaction: TransactionPoolRecord,
    ready_now: bool,
}

impl TransactionPoolStatusUpdate {
    pub fn new(transaction: TransactionPoolRecord, ready_now: bool) -> Self {
        Self { transaction, ready_now }
    }

    pub fn transaction(&self) -> &TransactionPoolRecord {
        &self.transaction
    }

    pub fn transaction_id(&self) -> &TransactionId {
        self.transaction.transaction_id()
    }

    pub fn stage(&self) -> TransactionPoolStage {
        self.transaction.current_stage()
    }

    pub fn evidence(&self) -> &Evidence {
        self.transaction.evidence()
    }

    pub fn is_ready(&self) -> bool {
        self.transaction.is_ready()
    }

    pub fn is_ready_now(&self) -> bool {
        self.ready_now
    }

    pub fn decision(&self) -> Decision {
        self.transaction.current_decision()
    }

    pub fn remote_decision(&self) -> Option<Decision> {
        self.transaction.remote_decision()
    }

    pub fn transaction_fee(&self) -> u64 {
        self.transaction.transaction_fee()
    }

    pub fn leader_fee(&self) -> Option<&LeaderFee> {
        self.transaction.leader_fee()
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
