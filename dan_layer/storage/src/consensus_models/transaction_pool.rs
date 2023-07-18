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
    consensus_models::{Command, Decision, QcId, TransactionAtom},
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
    /// The transaction been finalized but not yet executed, this allows proceeding blocks to form a 3-chain.
    Complete,
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

    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
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
            "Complete" => Ok(TransactionPoolStage::Complete),
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
            .filter_map(|t| match t.stage {
                TransactionPoolStage::New => Some(Command::Prepare(t.transaction)),
                TransactionPoolStage::Prepared => Some(Command::LocalPrepared(t.get_transaction_atom())),
                TransactionPoolStage::LocalPrepared => Some(Command::LocalPrepared(t.get_transaction_atom())),
                TransactionPoolStage::AllPrepared => Some(Command::Accept(t.get_transaction_atom())),
                TransactionPoolStage::SomePrepared => Some(Command::Accept(t.get_transaction_atom())),
                // Technically unreachable because is_ready should be false
                TransactionPoolStage::Complete => None,
            })
            .collect();

        Ok(commands)
    }

    pub fn has_uncommitted_transactions(
        &self,
        tx: &mut TStateStore::ReadTransaction<'_>,
    ) -> Result<bool, TransactionPoolError> {
        let count = tx.transaction_pool_count(None, Some(true))?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::Complete), None)?;
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
    pub changed_decision: Option<Decision>,
    pub is_ready: bool,
}

impl TransactionPoolRecord {
    pub fn final_decision(&self) -> Decision {
        self.changed_decision().unwrap_or(self.original_decision())
    }

    pub fn original_decision(&self) -> Decision {
        self.transaction.decision
    }

    pub fn changed_decision(&self) -> Option<Decision> {
        self.changed_decision
    }

    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction.id
    }

    pub fn stage(&self) -> TransactionPoolStage {
        self.stage
    }

    pub fn get_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            decision: self.final_decision(),
            ..self.transaction.clone()
        }
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
            ((TransactionPoolStage::Prepared, TransactionPoolStage::LocalPrepared), false) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::AllPrepared), _) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::SomePrepared), _) |
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::AllPrepared), _) |
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::SomePrepared), _) |
            ((TransactionPoolStage::SomePrepared, TransactionPoolStage::SomePrepared), _) |
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::Complete), false) |
            ((TransactionPoolStage::SomePrepared, TransactionPoolStage::Complete), false) => {},
            _ => {
                return Err(TransactionPoolError::InvalidTransactionTransition {
                    from: self.stage,
                    to: next_stage,
                    is_ready,
                })
            },
        }

        tx.transaction_pool_update(&self.transaction.id, None, Some(next_stage), None, Some(is_ready))?;
        self.stage = next_stage;

        Ok(())
    }

    pub fn update_decision<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        decision: Decision,
    ) -> Result<(), TransactionPoolError> {
        if self.original_decision() == decision {
            return Ok(());
        }

        self.changed_decision = Some(decision);
        tx.transaction_pool_update(&self.transaction.id, None, None, Some(decision), None)?;
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
        tx.transaction_pool_update(&self.transaction.id, Some(evidence), None, None, None)?;

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
