//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    marker::PhantomData,
    str::FromStr,
};

use log::*;
#[cfg(feature = "ts")]
use serde::Deserialize;
use serde::Serialize;
use tari_dan_common_types::{
    committee::CommitteeShard,
    optional::{IsNotFoundError, Optional},
};
use tari_transaction::TransactionId;
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    consensus_models::{
        Decision,
        LeafBlock,
        LockedBlock,
        QcId,
        TransactionAtom,
        TransactionPoolStatusUpdate,
        TransactionRecord,
    },
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

const _LOG_TARGET: &str = "tari::dan::storage::transaction_pool";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(TS, Deserialize),
    ts(export, export_to = "../../bindings/src/types/")
)]
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

    pub fn is_accepted(&self) -> bool {
        self.is_all_prepared() || self.is_some_prepared()
    }

    pub fn next_stage(&self) -> Option<Self> {
        match self {
            TransactionPoolStage::New => Some(TransactionPoolStage::Prepared),
            TransactionPoolStage::Prepared => Some(TransactionPoolStage::LocalPrepared),
            TransactionPoolStage::LocalPrepared => Some(TransactionPoolStage::AllPrepared),
            TransactionPoolStage::AllPrepared | TransactionPoolStage::SomePrepared => None,
        }
    }

    pub fn prev_stage(&self) -> Option<Self> {
        match self {
            TransactionPoolStage::New => None,
            TransactionPoolStage::Prepared => Some(TransactionPoolStage::New),
            TransactionPoolStage::LocalPrepared => Some(TransactionPoolStage::Prepared),
            TransactionPoolStage::AllPrepared => Some(TransactionPoolStage::LocalPrepared),
            TransactionPoolStage::SomePrepared => Some(TransactionPoolStage::LocalPrepared),
        }
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
        leaf: LeafBlock,
        id: &TransactionId,
    ) -> Result<TransactionPoolRecord, TransactionPoolError> {
        // We always want to fetch the state at the current leaf block until the leaf block
        // let leaf = LeafBlock::get(tx)?;
        let locked = LockedBlock::get(tx)?;
        debug!(
            target: _LOG_TARGET,
            "TransactionPool::get: transaction_id {}, leaf block {} and locked block {}",
            id,
            leaf,
            locked,
        );
        let rec = tx.transaction_pool_get(locked.block_id(), leaf.block_id(), id)?;
        Ok(rec)
    }

    pub fn exists(
        &self,
        tx: &mut TStateStore::ReadTransaction<'_>,
        id: &TransactionId,
    ) -> Result<bool, TransactionPoolError> {
        let exists = tx.transaction_pool_exists(id)?;
        Ok(exists)
    }

    pub fn insert(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        transaction: TransactionAtom,
    ) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_insert(transaction, TransactionPoolStage::New, true)?;
        Ok(())
    }

    pub fn get_batch_for_next_block(
        &self,
        tx: &mut TStateStore::ReadTransaction<'_>,
        max: usize,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError> {
        let recs = tx.transaction_pool_get_many_ready(max)?;
        Ok(recs)
    }

    pub fn has_uncommitted_transactions(
        &self,
        tx: &mut TStateStore::ReadTransaction<'_>,
    ) -> Result<bool, TransactionPoolError> {
        let count = tx.transaction_pool_count(None, Some(true), None)?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::Prepared), None, None)?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::LocalPrepared), Some(true), None)?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::AllPrepared), None, None)?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::SomePrepared), None, None)?;
        if count > 0 {
            return Ok(true);
        }

        // Check if we have any localprepared, is_ready=false but have foreign localprepared. If so, propose so that the
        // leaf block is processed.
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::LocalPrepared), None, Some(true))?;
        if count > 0 {
            return Ok(true);
        }

        Ok(count > 0)
    }

    pub fn count(&self, tx: &mut TStateStore::ReadTransaction<'_>) -> Result<usize, TransactionPoolError> {
        let count = tx.transaction_pool_count(None, None, None)?;
        Ok(count)
    }

    pub fn confirm_all_transitions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        locked_block: &LockedBlock,
        new_locked_block: &LockedBlock,
        tx_ids: I,
    ) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_set_all_transitions(locked_block, new_locked_block, tx_ids)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionPoolRecord {
    transaction: TransactionAtom,
    stage: TransactionPoolStage,
    pending_stage: Option<TransactionPoolStage>,
    local_decision: Option<Decision>,
    remote_decision: Option<Decision>,
    is_ready: bool,
}

impl TransactionPoolRecord {
    pub fn load(
        transaction: TransactionAtom,
        stage: TransactionPoolStage,
        pending_stage: Option<TransactionPoolStage>,
        local_decision: Option<Decision>,
        remote_decision: Option<Decision>,
        is_ready: bool,
    ) -> Self {
        Self {
            transaction,
            stage,
            pending_stage,
            local_decision,
            remote_decision,
            is_ready,
        }
    }

    pub fn current_decision(&self) -> Decision {
        self.remote_decision()
            // Prioritize remote ABORT
            .filter(|d| d.is_abort())
            .or_else(|| self.local_decision())
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

    pub fn transaction(&self) -> &TransactionAtom {
        &self.transaction
    }

    pub fn stage(&self) -> TransactionPoolStage {
        self.stage
    }

    pub fn pending_stage(&self) -> Option<TransactionPoolStage> {
        self.pending_stage
    }

    pub fn current_stage(&self) -> TransactionPoolStage {
        self.pending_stage.unwrap_or(self.stage)
    }

    pub fn is_ready(&self) -> bool {
        self.is_ready
    }

    pub fn get_final_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            decision: self.current_decision(),
            ..self.transaction.clone()
        }
    }

    pub fn get_local_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            decision: self.current_local_decision(),
            ..self.transaction.clone()
        }
    }

    pub fn set_remote_decision(&mut self, decision: Decision) -> &mut Self {
        // Only set remote_decision to ABORT, or COMMIT if it is not already ABORT
        self.remote_decision = self
            .remote_decision()
            .map(|d| match d {
                Decision::Commit => decision,
                Decision::Abort => Decision::Abort,
            })
            .or(Some(decision));
        self
    }

    pub fn set_local_decision(&mut self, decision: Decision) -> &mut Self {
        self.local_decision = Some(decision);
        self
    }

    pub fn add_evidence(&mut self, committee_shard: &CommitteeShard, qc_id: QcId) -> &mut Self {
        let evidence = &mut self.transaction.evidence;
        for (shard, evidence_mut) in evidence.iter_mut() {
            if committee_shard.includes_substate_address(shard) {
                evidence_mut.qc_ids.insert(qc_id);
            }
        }

        self
    }
}

impl TransactionPoolRecord {
    pub fn add_pending_status_update<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        block: LeafBlock,
        pending_stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), TransactionPoolError> {
        // Check that only permitted stage transactions are performed
        match ((self.current_stage(), pending_stage), is_ready) {
            ((TransactionPoolStage::New, TransactionPoolStage::Prepared), true) |
            ((TransactionPoolStage::Prepared, TransactionPoolStage::LocalPrepared), _) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::LocalPrepared), true) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::AllPrepared), false) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::SomePrepared), false) |
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::SomePrepared), false) |
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::AllPrepared), false) => {},
            _ => {
                return Err(TransactionPoolError::InvalidTransactionTransition {
                    from: self.current_stage(),
                    to: pending_stage,
                    is_ready,
                })
            },
        }

        let update = TransactionPoolStatusUpdate {
            block_id: block.block_id,
            block_height: block.height,
            transaction_id: self.transaction.id,
            stage: pending_stage,
            evidence: self.transaction.evidence.clone(),
            is_ready,
            local_decision: self.current_local_decision(),
        };

        tx.transaction_pool_add_pending_update(update)?;
        self.pending_stage = Some(pending_stage);

        Ok(())
    }

    pub fn update_remote_data<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        decision: Decision,
        foreign_qc_id: QcId,
        foreign_committee_shard: &CommitteeShard,
    ) -> Result<(), TransactionPoolError> {
        self.add_evidence(foreign_committee_shard, foreign_qc_id);
        self.set_remote_decision(decision);
        tx.transaction_pool_update(
            &self.transaction.id,
            None,
            Some(decision),
            Some(&self.transaction.evidence),
        )?;
        Ok(())
    }

    pub fn update_local_decision<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        decision: Decision,
    ) -> Result<(), TransactionPoolError> {
        if self.local_decision.map(|d| d != decision).unwrap_or(true) {
            self.set_local_decision(decision);
            tx.transaction_pool_update(&self.transaction.id, Some(decision), None, None)?;
        }
        Ok(())
    }

    pub fn remove<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_remove(&self.transaction.id)?;
        Ok(())
    }

    pub fn remove_any<'a, TTx, I>(tx: &mut TTx, transaction_ids: I) -> Result<(), TransactionPoolError>
    where
        TTx: StateStoreWriteTransaction,
        I: IntoIterator<Item = &'a TransactionId>,
    {
        // TODO(perf): n queries
        for id in transaction_ids {
            let _ = tx.transaction_pool_remove(id).optional()?;
        }
        Ok(())
    }

    pub fn get_transaction<TTx: StateStoreReadTransaction>(
        &self,
        tx: &mut TTx,
    ) -> Result<TransactionRecord, TransactionPoolError> {
        let transaction = TransactionRecord::get(tx, self.transaction_id())?;
        Ok(transaction)
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
