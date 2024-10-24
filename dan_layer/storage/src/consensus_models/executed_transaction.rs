//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::Deref,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{optional::Optional, SubstateAddress, ToSubstateAddress, VersionedSubstateId};
use tari_engine_types::commit_result::{ExecuteResult, RejectReason};
use tari_transaction::{Transaction, TransactionId};

use crate::{
    consensus_models::{
        AbortReason,
        BlockId,
        BlockTransactionExecution,
        Decision,
        TransactionExecution,
        TransactionRecord,
        VersionedSubstateIdLockIntent,
    },
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct ExecutedTransaction {
    transaction: Transaction,
    result: ExecuteResult,
    resulting_outputs: Vec<VersionedSubstateIdLockIntent>,
    resolved_inputs: Vec<VersionedSubstateIdLockIntent>,
    final_decision: Option<Decision>,
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number} | null"))]
    finalized_time: Option<Duration>,
    abort_reason: Option<RejectReason>,
}

impl ExecutedTransaction {
    pub fn new(
        transaction: Transaction,
        result: ExecuteResult,
        resolved_inputs: Vec<VersionedSubstateIdLockIntent>,
    ) -> Self {
        let outputs = result
            .finalize
            .result
            .accept()
            .map(|diff| {
                diff.up_iter()
                    .map(|(addr, substate)| {
                        VersionedSubstateIdLockIntent::output(VersionedSubstateId::new(
                            addr.clone(),
                            substate.version(),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            transaction,
            resolved_inputs,
            result,
            resulting_outputs: outputs,
            final_decision: None,
            finalized_time: None,
            abort_reason: None,
        }
    }

    pub fn id(&self) -> &TransactionId {
        self.transaction.id()
    }

    pub fn decision(&self) -> Decision {
        if let Some(decision) = self.final_decision {
            return decision;
        }

        if let Some(reject_reason) = &self.abort_reason {
            return Decision::Abort(AbortReason::from(reject_reason));
        }

        self.original_decision()
    }

    pub fn original_decision(&self) -> Decision {
        Decision::from(&self.result.finalize.result)
    }

    pub fn transaction(&self) -> &Transaction {
        &self.transaction
    }

    pub fn transaction_mut(&mut self) -> &mut Transaction {
        &mut self.transaction
    }

    pub fn into_transaction(self) -> Transaction {
        self.transaction
    }

    pub fn into_execution(self) -> TransactionExecution {
        TransactionRecord::from(self)
            .into_execution()
            .expect("invariant: ExecutedTransaction is executed")
    }

    pub fn result(&self) -> &ExecuteResult {
        &self.result
    }

    pub fn all_inputs_iter(&self) -> impl Iterator<Item = &VersionedSubstateId> + '_ {
        self.resolved_inputs.iter().map(|input| input.versioned_substate_id())
    }

    pub fn involved_addresses_iter(&self) -> impl Iterator<Item = SubstateAddress> + '_ {
        self.resolved_inputs
            .iter()
            .map(|input| input.to_substate_address())
            .chain(self.resulting_outputs.iter().map(|output| output.to_substate_address()))
    }

    pub fn num_inputs_and_outputs(&self) -> usize {
        self.transaction.num_unique_inputs() + self.resulting_outputs.len()
    }

    pub fn into_result(self) -> ExecuteResult {
        self.result
    }

    pub fn execution_time(&self) -> Duration {
        self.result.execution_time
    }

    /// Returns the outputs that resulted from execution.
    pub fn resulting_outputs(&self) -> &[VersionedSubstateIdLockIntent] {
        &self.resulting_outputs
    }

    pub fn resolved_inputs(&self) -> &[VersionedSubstateIdLockIntent] {
        &self.resolved_inputs
    }

    pub fn dissolve(
        self,
    ) -> (
        Transaction,
        ExecuteResult,
        Vec<VersionedSubstateIdLockIntent>,
        Vec<VersionedSubstateIdLockIntent>,
    ) {
        (
            self.transaction,
            self.result,
            self.resolved_inputs,
            self.resulting_outputs,
        )
    }

    pub fn transaction_fee(&self) -> u64 {
        self.result
            .finalize
            .fee_receipt
            .total_fees_paid()
            .as_u64_checked()
            .expect("invariant: engine calculated negative fees")
    }

    pub fn is_finalized(&self) -> bool {
        self.final_decision.is_some()
    }

    pub fn final_decision(&self) -> Option<Decision> {
        self.final_decision
    }

    pub fn finalized_time(&self) -> Option<Duration> {
        self.finalized_time
    }

    pub fn abort_reason(&self) -> Option<&RejectReason> {
        self.abort_reason.as_ref()
    }

    pub fn set_abort_reason(&mut self, reason: RejectReason) -> &mut Self {
        self.abort_reason = Some(reason);
        self
    }
}

impl ExecutedTransaction {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        TransactionRecord::from(self.clone()).insert(tx)
    }

    pub fn update<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        TransactionRecord::from(self.clone()).update(tx)
    }

    pub fn upsert<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        if Self::exists(&**tx, self.id())? {
            self.update(tx)
        } else {
            self.insert(tx)
        }
    }

    pub fn get<TTx: StateStoreReadTransaction>(tx: &TTx, tx_id: &TransactionId) -> Result<Self, StorageError> {
        let rec = tx.transactions_get(tx_id)?;
        if rec.execution_result.is_none() {
            return Err(StorageError::NotFound {
                item: "ExecutedTransaction",
                key: tx_id.to_string(),
            });
        }

        // This should never fail as we just checked that the transaction has been executed
        rec.try_into()
    }

    pub fn get_result<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        tx_id: &TransactionId,
    ) -> Result<ExecuteResult, StorageError> {
        // TODO(perf): consider optimising
        let rec = tx.transactions_get(tx_id)?;
        let Some(result) = rec.execution_result else {
            return Err(StorageError::NotFound {
                item: "ExecutedTransaction result",
                key: tx_id.to_string(),
            });
        };

        Ok(result)
    }

    pub fn get_pending_execution_for_block<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        block_id: &BlockId,
        tx_id: &TransactionId,
    ) -> Result<BlockTransactionExecution, StorageError> {
        if let Some(execution) = BlockTransactionExecution::get_by_block(tx, tx_id, block_id).optional()? {
            return Ok(execution);
        }

        // Since the mempool only executes versioned inputs it will update the local record with the final result.
        // If there is no pending transaction result, we check if the final transaction execution has been set.
        let exec = Self::get(tx, tx_id)?;
        if exec.is_finalized() {
            return Err(StorageError::QueryError {
                reason: format!("Transaction {} has already been finalized", tx_id),
            });
        }

        Ok(exec.into_execution().for_block(*block_id))
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        tx_id: &TransactionId,
    ) -> Result<bool, StorageError> {
        match tx.transactions_get(tx_id).optional()? {
            Some(rec) => Ok(rec.execution_result.is_some()),
            None => Ok(false),
        }
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &TTx,
        tx_ids: I,
    ) -> Result<(Vec<Self>, HashSet<&'a TransactionId>), StorageError> {
        let mut tx_ids = tx_ids.into_iter().collect::<HashSet<_>>();
        let recs = tx.transactions_get_any(tx_ids.iter().copied())?;
        for found in &recs {
            tx_ids.remove(found.transaction.id());
        }

        let recs = recs.into_iter().map(|rec| rec.try_into()).collect::<Result<_, _>>()?;
        Ok((recs, tx_ids))
    }

    pub fn get_all<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &TTx,
        tx_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        let (recs, missing) = Self::get_any(tx, tx_ids)?;
        if !missing.is_empty() {
            return Err(StorageError::NotFound {
                item: "ExecutedTransaction",
                key: missing
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        Ok(recs)
    }

    pub fn get_involved_shards<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &TTx,
        transactions: I,
    ) -> Result<HashMap<TransactionId, HashSet<SubstateAddress>>, StorageError> {
        let transactions = Self::get_all(tx, transactions)?;
        Ok(transactions
            .into_iter()
            .map(|t| (*t.transaction.id(), t.involved_addresses_iter().collect()))
            .collect())
    }
}

impl TryFrom<TransactionRecord> for ExecutedTransaction {
    type Error = StorageError;

    fn try_from(value: TransactionRecord) -> Result<Self, Self::Error> {
        if !value.is_executed() {
            return Err(StorageError::QueryError {
                reason: format!(
                    "ExecutedTransaction::try_from: Transaction {} has not yet executed",
                    value.transaction.id()
                ),
            });
        }

        let resolved_inputs = value.resolved_inputs.ok_or_else(|| StorageError::DataInconsistency {
            details: format!("Executed transaction {} has no resolved inputs", value.transaction.id()),
        })?;

        let resulting_outputs = value.resulting_outputs.ok_or_else(|| StorageError::DataInconsistency {
            details: format!(
                "Executed transaction {} has no resulting outputs",
                value.transaction.id()
            ),
        })?;

        Ok(Self {
            transaction: value.transaction,
            result: value.execution_result.unwrap(),
            resolved_inputs,
            final_decision: value.final_decision,
            finalized_time: value.finalized_time,
            resulting_outputs,
            abort_reason: value.abort_reason,
        })
    }
}

impl PartialEq for ExecutedTransaction {
    fn eq(&self, other: &Self) -> bool {
        self.transaction.id() == other.transaction.id()
    }
}

impl Eq for ExecutedTransaction {}

impl Hash for ExecutedTransaction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.transaction.id().hash(state);
    }
}
