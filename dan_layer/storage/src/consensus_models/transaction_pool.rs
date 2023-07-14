//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::BTreeSet,
    fmt::{Display, Formatter},
    marker::PhantomData,
    str::FromStr,
};

use tari_dan_common_types::{
    committee::CommitteeShard,
    optional::{IsNotFoundError, Optional},
};
use tari_transaction::TransactionId;

use crate::{
    consensus_models::{Command, QcId, TransactionAtom},
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Copy)]
pub enum TransactionPoolStage {
    New,
    LocalPrepared,
    SomePrepared,
    AllPrepared,
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
    ) -> Result<BTreeSet<Command>, TransactionPoolError> {
        let ready = tx.transaction_pool_get_many_ready(max)?;
        let commands = ready
            .into_iter()
            .map(|t| match t.stage {
                TransactionPoolStage::New => Command::Prepare(t.transaction),
                TransactionPoolStage::LocalPrepared => Command::LocalPrepared(t.transaction),
                TransactionPoolStage::AllPrepared => Command::Accept(t.transaction),
                // TODO: We move to abort - add some test cases and figure out the best way to handle this
                TransactionPoolStage::SomePrepared => Command::Accept(t.transaction),
            })
            .collect();

        Ok(commands)
    }

    pub fn has_transactions(&self, tx: &mut TStateStore::ReadTransaction<'_>) -> Result<bool, TransactionPoolError> {
        let count = self.count(tx)?;
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
    pub is_ready: bool,
}

impl TransactionPoolRecord {
    pub fn transition<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        next_stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), TransactionPoolError> {
        match ((self.stage, next_stage), is_ready) {
            ((TransactionPoolStage::New, TransactionPoolStage::LocalPrepared), false) => {
                tx.transaction_pool_update(&self.transaction.id, None, Some(next_stage), Some(false))?;
            },
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::LocalPrepared), true) => {
                tx.transaction_pool_update(&self.transaction.id, None, Some(next_stage), Some(true))?;
            },
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::AllPrepared), true) => {
                tx.transaction_pool_update(&self.transaction.id, None, Some(next_stage), Some(true))?;
            },
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::AllPrepared), true) => {
                tx.transaction_pool_update(&self.transaction.id, None, Some(next_stage), Some(true))?;
            },
            _ => {
                return Err(TransactionPoolError::InvalidTransactionTransition {
                    from: self.stage,
                    to: next_stage,
                    is_ready,
                })
            },
        }

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
        tx.transaction_pool_update(&self.transaction.id, Some(evidence), None, None)?;

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
