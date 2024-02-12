//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::DerefMut,
    time::Duration,
};

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::{optional::Optional, SubstateAddress};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason},
    lock::LockFlag,
};
use tari_transaction::{Transaction, TransactionId};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    consensus_models::{Decision, Evidence, ShardEvidence, TransactionAtom, TransactionRecord},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ExecutedTransaction {
    transaction: Transaction,
    result: ExecuteResult,
    resulting_outputs: Vec<SubstateAddress>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    execution_time: Duration,
    final_decision: Option<Decision>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    finalized_time: Option<Duration>,
    abort_details: Option<String>,
}

impl ExecutedTransaction {
    pub fn new(
        transaction: Transaction,
        result: ExecuteResult,
        resulting_outputs: Vec<SubstateAddress>,
        execution_time: Duration,
    ) -> Self {
        Self {
            transaction,
            result,
            execution_time,
            resulting_outputs,
            final_decision: None,
            finalized_time: None,
            abort_details: None,
        }
    }

    pub fn id(&self) -> &TransactionId {
        self.transaction.id()
    }

    pub fn decision(&self) -> Decision {
        if let Some(decision) = self.final_decision {
            return decision;
        }

        self.original_decision()
    }

    pub fn original_decision(&self) -> Decision {
        if self.result.finalize.is_accept() {
            Decision::Commit
        } else {
            Decision::Abort
        }
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

    pub fn result(&self) -> &ExecuteResult {
        &self.result
    }

    pub fn involved_shards_iter(&self) -> impl Iterator<Item = &SubstateAddress> + '_ {
        self.transaction.all_inputs_iter().chain(&self.resulting_outputs)
    }

    pub fn num_involved_shards(&self) -> usize {
        self.transaction.num_involved_shards() + self.resulting_outputs.len()
    }

    pub fn into_final_result(self) -> Option<ExecuteResult> {
        self.final_decision().map(|d| {
            if d.is_commit() {
                self.result
            } else {
                // TODO: We preserve the original result mainly for debugging purposes, but this is a little hacky
                ExecuteResult {
                    finalize: FinalizeResult::new_rejected(
                        self.result.finalize.transaction_hash,
                        RejectReason::ShardRejected(format!(
                            "Validators decided to abort: {}",
                            self.abort_details
                                .as_deref()
                                .unwrap_or("<invalid state, no abort details>")
                        )),
                    ),
                    fee_receipt: None,
                }
            }
        })
    }

    pub fn into_result(self) -> ExecuteResult {
        self.result
    }

    pub fn execution_time(&self) -> Duration {
        self.execution_time
    }

    /// Returns the outputs that resulted from execution.
    pub fn resulting_outputs(&self) -> &[SubstateAddress] {
        &self.resulting_outputs
    }

    pub fn dissolve(self) -> (Transaction, ExecuteResult) {
        (self.transaction, self.result)
    }

    pub fn to_initial_evidence(&self) -> Evidence {
        let mut deduped_evidence = HashMap::new();
        deduped_evidence.extend(self.transaction.inputs().iter().map(|input| {
            (*input, ShardEvidence {
                qc_ids: IndexSet::new(),
                lock: LockFlag::Write,
            })
        }));

        deduped_evidence.extend(self.transaction.input_refs().iter().map(|input_ref| {
            (*input_ref, ShardEvidence {
                qc_ids: IndexSet::new(),
                lock: LockFlag::Read,
            })
        }));

        deduped_evidence.extend(self.transaction.filled_inputs().iter().map(|input_ref| {
            (*input_ref, ShardEvidence {
                qc_ids: IndexSet::new(),
                lock: LockFlag::Write,
            })
        }));

        deduped_evidence.extend(self.resulting_outputs.iter().map(|output| {
            (*output, ShardEvidence {
                qc_ids: IndexSet::new(),
                lock: LockFlag::Write,
            })
        }));

        deduped_evidence.into_iter().collect()
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

    pub fn abort_details(&self) -> Option<&String> {
        self.abort_details.as_ref()
    }

    pub fn set_final_decision(&mut self, decision: Decision) -> &mut Self {
        self.final_decision = Some(decision);
        if decision.is_abort() && self.abort_details.is_none() {
            self.abort_details = Some(
                self.result
                    .finalize
                    .result
                    .reject()
                    .map(|reason| reason.to_string())
                    .unwrap_or_else(|| "Transaction execution succeeded but ABORT decision made".to_string()),
            );
        }
        self
    }

    pub fn set_abort<T: Into<String>>(&mut self, details: T) -> &mut Self {
        self.final_decision = Some(Decision::Abort);
        self.abort_details = Some(details.into());
        self
    }

    pub fn to_atom(&self) -> TransactionAtom {
        TransactionAtom {
            id: *self.id(),
            decision: self.decision(),
            evidence: self.to_initial_evidence(),
            transaction_fee: self
                .result()
                .fee_receipt
                .as_ref()
                .and_then(|f| f.total_fees_paid().as_u64_checked())
                .unwrap_or(0),
            // We calculate the leader fee later depending on the epoch of the block
            leader_fee: 0,
        }
    }
}

impl ExecutedTransaction {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_insert(self.transaction())?;
        self.update(tx)
    }

    pub fn upsert<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        if TransactionRecord::exists(tx.deref_mut(), self.transaction.id())? {
            self.update(tx)
        } else {
            self.insert(tx)
        }
    }

    pub fn update<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        TransactionRecord::from(self.clone()).update(tx)
    }

    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, tx_id: &TransactionId) -> Result<Self, StorageError> {
        let rec = tx.transactions_get(tx_id)?;
        if rec.result.is_none() {
            return Err(StorageError::NotFound {
                item: "ExecutedTransaction".to_string(),
                key: tx_id.to_string(),
            });
        }

        // This should never fail as we just checked that the transaction has been executed
        rec.try_into()
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &mut TTx,
        tx_id: &TransactionId,
    ) -> Result<bool, StorageError> {
        match tx.transactions_get(tx_id).optional()? {
            Some(rec) => Ok(rec.result.is_some()),
            None => Ok(false),
        }
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
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
        tx: &mut TTx,
        tx_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        let (recs, missing) = Self::get_any(tx, tx_ids)?;
        if !missing.is_empty() {
            return Err(StorageError::NotFound {
                item: "ExecutedTransaction".to_string(),
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
        tx: &mut TTx,
        transactions: I,
    ) -> Result<HashMap<TransactionId, HashSet<SubstateAddress>>, StorageError> {
        let transactions = Self::get_all(tx, transactions)?;
        Ok(transactions
            .into_iter()
            .map(|t| (*t.transaction.id(), t.involved_shards_iter().copied().collect()))
            .collect())
    }
}

impl TryFrom<TransactionRecord> for ExecutedTransaction {
    type Error = StorageError;

    fn try_from(value: TransactionRecord) -> Result<Self, Self::Error> {
        if value.result.is_none() {
            return Err(StorageError::QueryError {
                reason: format!("Transaction {} has not yet executed", value.transaction.id()),
            });
        }

        Ok(Self {
            transaction: value.transaction,
            result: value.result.unwrap(),
            execution_time: value.execution_time.unwrap_or_default(),
            final_decision: value.final_decision,
            finalized_time: value.finalized_time,
            resulting_outputs: value.resulting_outputs,
            abort_details: value.abort_details,
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
