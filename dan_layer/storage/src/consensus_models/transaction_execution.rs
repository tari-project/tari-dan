//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_engine_types::commit_result::ExecuteResult;
use tari_transaction::{TransactionId, VersionedSubstateId};

use crate::{
    consensus_models::{BlockId, Decision, Evidence, VersionedSubstateIdLockIntent},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone)]
pub struct TransactionExecution {
    pub block_id: BlockId,
    pub transaction_id: TransactionId,
    pub result: ExecuteResult,
    pub resolved_inputs: Vec<VersionedSubstateIdLockIntent>,
    pub resulting_outputs: Vec<VersionedSubstateId>,
    pub execution_time: Duration,
}

impl TransactionExecution {
    pub fn new(
        block_id: BlockId,
        transaction_id: TransactionId,
        result: ExecuteResult,
        resolved_inputs: Vec<VersionedSubstateIdLockIntent>,
        resulting_outputs: Vec<VersionedSubstateId>,
        execution_time: Duration,
    ) -> Self {
        Self {
            block_id,
            transaction_id,
            result,
            resolved_inputs,
            resulting_outputs,
            execution_time,
        }
    }

    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn decision(&self) -> Decision {
        Decision::from(&self.result.finalize.result)
    }

    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    pub fn result(&self) -> &ExecuteResult {
        &self.result
    }

    pub fn resolved_inputs(&self) -> &[VersionedSubstateIdLockIntent] {
        &self.resolved_inputs
    }

    pub fn resulting_outputs(&self) -> &Vec<VersionedSubstateId> {
        &self.resulting_outputs
    }

    pub fn execution_time(&self) -> Duration {
        self.execution_time
    }

    pub fn to_initial_evidence(&self) -> Evidence {
        Evidence::from_inputs_and_outputs(self.transaction_id, &self.resolved_inputs, &self.resulting_outputs)
    }

    pub fn transaction_fee(&self) -> u64 {
        self.result
            .finalize
            .fee_receipt
            .total_fees_paid()
            .as_u64_checked()
            .unwrap()
    }
}

impl TransactionExecution {
    pub fn insert_if_required<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transaction_executions_insert_or_ignore(self)
    }

    /// Fetches any pending execution that happened before the given block until the commit block (parent of locked
    /// block)
    pub fn get_pending_for_block<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        transaction_id: &TransactionId,
        from_block_id: &BlockId,
    ) -> Result<Self, StorageError> {
        tx.transaction_executions_get_pending_for_block(transaction_id, from_block_id)
    }

    /// Fetches any pending execution that happened in the given block
    pub fn get_by_block<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        transaction_id: &TransactionId,
        block_id: &BlockId,
    ) -> Result<Self, StorageError> {
        tx.transaction_executions_get(transaction_id, block_id)
    }
}
