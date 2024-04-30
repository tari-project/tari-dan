//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::DerefMut,
    time::Duration,
};

use indexmap::IndexSet;
use serde::Deserialize;
use tari_dan_common_types::SubstateAddress;
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason},
    lock::LockFlag,
};
use tari_transaction::{Transaction, TransactionId, VersionedSubstateId};

use crate::{
    consensus_models::{
        Decision,
        Evidence,
        ExecutedTransaction,
        ShardEvidence,
        TransactionAtom,
        VersionedSubstateIdLockIntent,
    },
    Ordering,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Deserialize)]
pub struct TransactionRecord {
    pub transaction: Transaction,
    pub result: Option<ExecuteResult>,
    pub execution_time: Option<Duration>,
    pub resulting_outputs: Vec<VersionedSubstateId>,
    pub resolved_inputs: Option<IndexSet<VersionedSubstateIdLockIntent>>,
    pub final_decision: Option<Decision>,
    pub finalized_time: Option<Duration>,
    pub abort_details: Option<String>,
}

impl TransactionRecord {
    pub fn new(transaction: Transaction) -> Self {
        Self {
            transaction,
            result: None,
            resolved_inputs: None,
            execution_time: None,
            final_decision: None,
            finalized_time: None,
            resulting_outputs: Vec::new(),
            abort_details: None,
        }
    }

    pub fn load(
        transaction: Transaction,
        result: Option<ExecuteResult>,
        resolved_inputs: Option<IndexSet<VersionedSubstateIdLockIntent>>,
        execution_time: Option<Duration>,
        final_decision: Option<Decision>,
        finalized_time: Option<Duration>,
        resulting_outputs: Vec<VersionedSubstateId>,
        abort_details: Option<String>,
    ) -> Self {
        Self {
            transaction,
            resolved_inputs,
            result,
            execution_time,
            final_decision,
            finalized_time,
            resulting_outputs,
            abort_details,
        }
    }

    pub fn id(&self) -> &TransactionId {
        self.transaction.id()
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

    pub fn result(&self) -> Option<&ExecuteResult> {
        self.result.as_ref()
    }

    pub fn has_executed(&self) -> bool {
        self.result.is_some()
    }

    pub fn resulting_outputs(&self) -> &[VersionedSubstateId] {
        &self.resulting_outputs
    }

    pub fn resolved_inputs(&self) -> Option<&IndexSet<VersionedSubstateIdLockIntent>> {
        self.resolved_inputs.as_ref()
    }

    pub fn final_decision(&self) -> Option<Decision> {
        self.final_decision
    }

    pub fn execution_time(&self) -> Option<Duration> {
        self.execution_time
    }

    pub fn finalized_time(&self) -> Option<Duration> {
        self.finalized_time
    }

    pub fn is_finalized(&self) -> bool {
        self.final_decision.is_some()
    }

    pub fn abort_details(&self) -> Option<&String> {
        self.abort_details.as_ref()
    }

    pub fn set_abort<T: Into<String>>(&mut self, details: T) -> &mut Self {
        self.final_decision = Some(Decision::Abort);
        self.abort_details = Some(details.into());
        self
    }

    pub fn into_final_result(self) -> Option<ExecuteResult> {
        // TODO: This is hacky, result should be broken up into execution result, validation (mempool) result, finality
        //       result. These results are independent of each other.
        self.final_decision().and_then(|d| {
            if d.is_commit() {
                self.result
            } else {
                Some(ExecuteResult {
                    finalize: FinalizeResult::new_rejected(
                        self.transaction.id().into_array().into(),
                        RejectReason::ShardRejected(format!(
                            "Validators decided to abort: {}",
                            self.abort_details
                                .as_deref()
                                .unwrap_or("<invalid state, no abort details>")
                        )),
                    ),
                })
            }
        })
    }
}

impl TransactionRecord {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_insert(self)
    }

    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        if !Self::exists(tx.deref_mut(), self.transaction.id())? {
            self.insert(tx)?;
        }
        Ok(())
    }

    pub fn save_all<'a, TTx, I>(tx: &mut TTx, transactions: I) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
        I: IntoIterator<Item = &'a TransactionRecord>,
    {
        tx.transactions_save_all(transactions)
    }

    pub fn update<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_update(self)
    }

    pub fn upsert<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        if TransactionRecord::exists(tx.deref_mut(), self.id())? {
            self.update(tx)
        } else {
            self.insert(tx)
        }
    }

    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, tx_id: &TransactionId) -> Result<Self, StorageError> {
        tx.transactions_get(tx_id)
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &mut TTx,
        tx_id: &TransactionId,
    ) -> Result<bool, StorageError> {
        tx.transactions_exists(tx_id)
    }

    pub fn exists_any<'a, TTx: StateStoreReadTransaction + ?Sized, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        tx_ids: I,
    ) -> Result<bool, StorageError> {
        for tx_id in tx_ids {
            if tx.transactions_exists(tx_id)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        tx_ids: I,
    ) -> Result<(Vec<Self>, HashSet<TransactionId>), StorageError> {
        let mut tx_ids = tx_ids.into_iter().copied().collect::<HashSet<_>>();
        let recs = tx.transactions_get_any(tx_ids.iter())?;
        for rec in &recs {
            tx_ids.remove(rec.transaction.id());
        }

        Ok((recs, tx_ids))
    }

    pub fn get_paginated<TTx: StateStoreReadTransaction>(
        tx: &mut TTx,
        limit: u64,
        offset: u64,
        ordering: Option<Ordering>,
    ) -> Result<Vec<Self>, StorageError> {
        tx.transactions_get_paginated(limit, offset, ordering)
    }

    pub fn to_atom(&self) -> TransactionAtom {
        if let Some(result) = self.result() {
            let decision = if result.finalize.result.is_accept() {
                Decision::Commit
            } else {
                Decision::Abort
            };

            TransactionAtom {
                id: *self.id(),
                decision,
                evidence: self.to_initial_evidence(),
                transaction_fee: result
                    .finalize
                    .fee_receipt
                    .total_fees_paid()
                    .as_u64_checked()
                    .unwrap_or(0),
                // We calculate the leader fee later depending on the epoch of the block
                leader_fee: None,
            }
        } else {
            // Deferred
            TransactionAtom {
                id: *self.transaction.id(),
                decision: Decision::Deferred,
                evidence: Default::default(),
                transaction_fee: 0,
                leader_fee: None,
            }
        }
    }

    fn to_initial_evidence(&self) -> Evidence {
        let mut deduped_evidence = HashMap::new();
        deduped_evidence.extend(self.resolved_inputs.iter().flatten().map(|input| {
            (input.to_substate_address(), ShardEvidence {
                qc_ids: IndexSet::new(),
                lock: input.lock_flag().as_lock_flag(),
            })
        }));

        let tx_reciept_address = SubstateAddress::for_transaction_receipt(self.id().into_receipt_address());
        deduped_evidence.extend(
            self.resulting_outputs
                    .iter()
                    .map(|output| output.to_substate_address())
                    // Exclude transaction receipt address from evidence since all involved shards will commit it
                    .filter(|output| *output != tx_reciept_address)
                    .map(|output| {
                        (output, ShardEvidence {
                            qc_ids: IndexSet::new(),
                            lock: LockFlag::Write,
                        })
                    }),
        );

        deduped_evidence.into_iter().collect()
    }
}

impl From<ExecutedTransaction> for TransactionRecord {
    fn from(tx: ExecutedTransaction) -> Self {
        let execution_time = tx.execution_time();
        let final_decision = tx.final_decision();
        let finalized_time = tx.finalized_time();
        let abort_details = tx.abort_details().cloned();
        let resulting_outputs = tx.resulting_outputs().to_vec();
        let resolved_inputs = tx.resolved_inputs().cloned();
        let (transaction, result) = tx.dissolve();

        Self {
            transaction,
            result: Some(result),
            execution_time: Some(execution_time),
            resolved_inputs,
            final_decision,
            finalized_time,
            resulting_outputs,
            abort_details,
        }
    }
}
