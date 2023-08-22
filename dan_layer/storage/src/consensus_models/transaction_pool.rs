//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    marker::PhantomData,
    num::NonZeroU64,
    str::FromStr,
};

use tari_dan_common_types::{
    committee::CommitteeShard,
    optional::{IsNotFoundError, Optional},
};
use tari_transaction::TransactionId;

use crate::{
    consensus_models::{Decision, QcId, TransactionAtom},
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionPoolStage {
    /// Transaction has just come in and has never been proposed
    New,
    /// Transaction is prepared in response to a Prepare command, but we do not yet have confirmation that the rest of
    /// the local committee has prepared.
    Prepared,
    /// We have proof that all local committees have prepared the transaction
    LocalPrepared,
    /// All foreign shards have prepared and have an identical decision
    AllPrepared,
    /// All foreign shards have prepared but one or more has decided to ABORT
    SomePrepared,
}

impl TransactionPoolStage {
    pub fn is_new(&self) -> bool {
        matches!(self, Self::New)
    }

    pub fn is_prepared(&self) -> bool {
        matches!(self, Self::Prepared)
    }

    pub fn is_local_prepared(&self) -> bool {
        matches!(self, Self::LocalPrepared)
    }

    pub fn is_some_prepared(&self) -> bool {
        matches!(self, Self::SomePrepared)
    }

    pub fn is_all_prepared(&self) -> bool {
        matches!(self, Self::AllPrepared)
    }
}

impl Display for TransactionPoolStage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl FromStr for TransactionPoolStage {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "New" => Ok(TransactionPoolStage::New),
            "Prepared" => Ok(TransactionPoolStage::Prepared),
            "LocalPrepared" => Ok(TransactionPoolStage::LocalPrepared),
            "SomePrepared" => Ok(TransactionPoolStage::SomePrepared),
            "AllPrepared" => Ok(TransactionPoolStage::AllPrepared),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransactionPool<TStateStore> {
    _store: PhantomData<TStateStore>,
}

impl<TStateStore: StateStore> TransactionPool<TStateStore> {
    pub fn new() -> Self {
        Self { _store: PhantomData }
    }

    pub fn get(
        &self,
        tx: &mut TStateStore::ReadTransaction<'_>,
        id: &TransactionId,
    ) -> Result<TransactionPoolRecord, TransactionPoolError> {
        let rec = tx.transaction_pool_get(id)?;
        Ok(rec)
    }

    pub fn exists(
        &self,
        tx: &mut TStateStore::ReadTransaction<'_>,
        id: &TransactionId,
    ) -> Result<bool, TransactionPoolError> {
        // TODO: optimise
        let rec = tx.transaction_pool_get(id).optional()?;
        Ok(rec.is_some())
    }

    pub fn insert(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        transaction: TransactionAtom,
    ) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_insert(transaction, TransactionPoolStage::New, true)?;
        Ok(())
    }

    pub fn get_batch(
        &self,
        tx: &mut TStateStore::ReadTransaction<'_>,
        max: usize,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError> {
        let mut recs = tx.transaction_pool_get_many_ready(max)?;
        // We require the records to be canonically sorted by transaction ID
        // TODO(perf): might be able to delegate this to the storage layer
        recs.sort_by(|a, b| a.transaction.id.cmp(&b.transaction.id));
        Ok(recs)
    }

    pub fn has_uncommitted_transactions(
        &self,
        tx: &mut TStateStore::ReadTransaction<'_>,
    ) -> Result<bool, TransactionPoolError> {
        let count = tx.transaction_pool_count(None, Some(true))?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::AllPrepared), None)?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::SomePrepared), None)?;
        if count > 0 {
            return Ok(true);
        }
        Ok(count > 0)
    }

    pub fn count(&self, tx: &mut TStateStore::ReadTransaction<'_>) -> Result<usize, TransactionPoolError> {
        let count = tx.transaction_pool_count(None, None)?;
        Ok(count)
    }
}

#[derive(Debug, Clone)]
pub struct TransactionPoolRecord {
    pub transaction: TransactionAtom,
    pub stage: TransactionPoolStage,
    pub local_decision: Option<Decision>,
    pub remote_decision: Option<Decision>,
    pub is_ready: bool,
}

impl TransactionPoolRecord {
    pub fn current_decision(&self) -> Decision {
        self.local_decision()
            .or_else(|| self.remote_decision())
            .unwrap_or(self.original_decision())
    }

    pub fn current_local_decision(&self) -> Decision {
        self.local_decision().unwrap_or(self.original_decision())
    }

    pub fn original_decision(&self) -> Decision {
        self.transaction.decision
    }

    pub fn local_decision(&self) -> Option<Decision> {
        self.local_decision
    }

    pub fn remote_decision(&self) -> Option<Decision> {
        self.remote_decision
    }

    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction.id
    }

    pub fn stage(&self) -> TransactionPoolStage {
        self.stage
    }

    pub fn get_final_transaction_atom(&self, leader_fee: u64) -> TransactionAtom {
        TransactionAtom {
            decision: self.current_decision(),
            leader_fee,
            ..self.transaction.clone()
        }
    }

    pub fn get_local_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            decision: self.current_local_decision(),
            ..self.transaction.clone()
        }
    }

    pub fn calculate_leader_fee(&self, involved: NonZeroU64, exhaust_divisor: u64) -> u64 {
        // TODO: We essentially burn a random amount depending on the shards involved in the transaction. This means it
        //       is hard to tell how much is actually in circulation unless we track this in the Resource. Right
        //       now we'll set exhaust to 0, which is just transaction_fee / involved.
        let transaction_fee = self.transaction.transaction_fee;
        let due_fee = transaction_fee / involved.get();
        // The extra amount that is burnt
        let due_rem = transaction_fee % involved.get();

        // How much we want to burn due to exhaust per involved shard
        let target_exhaust_burn = if exhaust_divisor > 0 {
            let base_fee = transaction_fee.checked_div(exhaust_divisor).unwrap_or(transaction_fee);
            base_fee / involved.get()
        } else {
            0
        };

        // Adjust the amount to burn taking into account the remainder that we burn
        let adjusted_burn = target_exhaust_burn.saturating_sub(due_rem);

        due_fee - adjusted_burn
    }

    pub fn set_remote_decision(&mut self, decision: Decision) -> &mut Self {
        self.remote_decision = Some(decision);
        self
    }

    pub fn set_local_decision(&mut self, decision: Decision) -> &mut Self {
        self.local_decision = Some(decision);
        self
    }
}

impl TransactionPoolRecord {
    pub fn transition<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        next_stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), TransactionPoolError> {
        // Check that only permitted stage transactions are performed
        match ((self.stage, next_stage), is_ready) {
            ((TransactionPoolStage::New, TransactionPoolStage::Prepared), true) |
            ((TransactionPoolStage::Prepared, TransactionPoolStage::LocalPrepared), _) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::LocalPrepared), true) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::AllPrepared), false) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::SomePrepared), false) |
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::SomePrepared), false) => {},
            _ => {
                return Err(TransactionPoolError::InvalidTransactionTransition {
                    from: self.stage,
                    to: next_stage,
                    is_ready,
                })
            },
        }

        tx.transaction_pool_update(&self.transaction.id, None, Some(next_stage), None, None, Some(is_ready))?;
        self.stage = next_stage;

        Ok(())
    }

    pub fn update_remote_decision<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        decision: Decision,
    ) -> Result<(), TransactionPoolError> {
        self.set_remote_decision(decision);
        tx.transaction_pool_update(&self.transaction.id, None, None, None, Some(decision), None)?;
        Ok(())
    }

    pub fn update_local_decision<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        decision: Decision,
    ) -> Result<(), TransactionPoolError> {
        self.set_local_decision(decision);
        tx.transaction_pool_update(&self.transaction.id, None, None, Some(decision), None, None)?;
        Ok(())
    }

    pub fn update_evidence<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        committee_shard: &CommitteeShard,
        qc_id: QcId,
    ) -> Result<(), TransactionPoolError> {
        let evidence = &mut self.transaction.evidence;
        for (shard, qcs_mut) in evidence.iter_mut() {
            if committee_shard.includes_shard(shard) {
                qcs_mut.push(qc_id);
            }
        }
        tx.transaction_pool_update(&self.transaction.id, Some(evidence), None, None, None, None)?;

        Ok(())
    }

    pub fn remove<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_remove(&self.transaction.id)?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionPoolError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Invalid transaction transition from {from:?} to {to:?} with is_ready={is_ready}")]
    InvalidTransactionTransition {
        from: TransactionPoolStage,
        to: TransactionPoolStage,
        is_ready: bool,
    },
}

impl IsNotFoundError for TransactionPoolError {
    fn is_not_found_error(&self) -> bool {
        match self {
            TransactionPoolError::StorageError(e) => e.is_not_found_error(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod calculate_leader_fee {
        use super::*;

        fn create_record_with_fee(fee: u64) -> TransactionPoolRecord {
            TransactionPoolRecord {
                transaction: TransactionAtom {
                    id: TransactionId::new([0; 32]),
                    decision: Decision::Commit,
                    evidence: Default::default(),
                    transaction_fee: fee,
                    leader_fee: 0,
                },
                stage: TransactionPoolStage::New,
                local_decision: None,
                remote_decision: None,
                is_ready: false,
            }
        }

        #[test]
        fn it_calculates_the_correct_fee_due() {
            let record = create_record_with_fee(100);

            let fee = record.calculate_leader_fee(1.try_into().unwrap(), 0);
            assert_eq!(fee, 100);

            let fee = record.calculate_leader_fee(1.try_into().unwrap(), 10);
            assert_eq!(fee, 90);

            let fee = record.calculate_leader_fee(2.try_into().unwrap(), 0);
            assert_eq!(fee, 50);

            let fee = record.calculate_leader_fee(2.try_into().unwrap(), 10);
            assert_eq!(fee, 45);

            let fee = record.calculate_leader_fee(3.try_into().unwrap(), 0);
            assert_eq!(fee, 33);

            let fee = record.calculate_leader_fee(3.try_into().unwrap(), 10);
            assert_eq!(fee, 31);

            let record = create_record_with_fee(98);

            let fee = record.calculate_leader_fee(3.try_into().unwrap(), 10);
            assert_eq!(fee, 31);

            let fee = record.calculate_leader_fee(10.try_into().unwrap(), 10);
            assert_eq!(fee, 9);
        }
    }
}
