//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    clone::Clone,
    fmt::{Display, Formatter},
    marker::PhantomData,
    num::NonZeroU64,
    str::FromStr,
};

use log::*;
use serde::Serialize;
use tari_dan_common_types::{
    committee::CommitteeInfo,
    optional::{IsNotFoundError, Optional},
    ShardGroup,
};
use tari_transaction::TransactionId;

use crate::{
    consensus_models::{
        BlockId,
        BlockTransactionExecution,
        Decision,
        Evidence,
        LeaderFee,
        LeafBlock,
        LockedBlock,
        QcId,
        SubstatePledges,
        TransactionAtom,
        TransactionExecution,
        TransactionPoolStatusUpdate,
        TransactionRecord,
    },
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

const LOG_TARGET: &str = "tari::dan::storage::transaction_pool";

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
        tx: &TStateStore::ReadTransaction<'_>,
        leaf: LeafBlock,
        id: &TransactionId,
    ) -> Result<TransactionPoolRecord, TransactionPoolError> {
        // We always want to fetch the state at the current leaf block until the leaf block
        let locked = LockedBlock::get(tx)?;
        debug!(
            target: LOG_TARGET,
            "TransactionPool::get: transaction_id {}, leaf block {} and locked block {}",
            id,
            leaf,
            locked,
        );
        let rec = tx.transaction_pool_get_for_blocks(locked.block_id(), leaf.block_id(), id)?;
        Ok(rec)
    }

    pub fn exists(
        &self,
        tx: &TStateStore::ReadTransaction<'_>,
        id: &TransactionId,
    ) -> Result<bool, TransactionPoolError> {
        let exists = tx.transaction_pool_exists(id)?;
        Ok(exists)
    }

    pub fn insert_new(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        tx_id: TransactionId,
        decision: Decision,
        is_ready: bool,
    ) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_insert_new(tx_id, decision, is_ready)?;
        Ok(())
    }

    pub fn get_batch_for_next_block(
        &self,
        tx: &TStateStore::ReadTransaction<'_>,
        max: usize,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError> {
        let recs = tx.transaction_pool_get_many_ready(max)?;
        Ok(recs)
    }

    pub fn has_uncommitted_transactions(
        &self,
        tx: &TStateStore::ReadTransaction<'_>,
    ) -> Result<bool, TransactionPoolError> {
        let count = tx.transaction_pool_count(None, Some(true), None)?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::LocalOnly), None, None)?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::Prepared), None, None)?;
        if count > 0 {
            return Ok(true);
        }
        // Check if we have local prepared that has not yet been confirmed (locked). In this case we should propose
        // until this stage is locked.
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::LocalPrepared), None, Some(None))?;
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
        // Check if we have local accepted that is still confirmed(locked) to be prepared. In this case we should
        // propose until this stage is locked.
        let count = tx.transaction_pool_count(
            Some(TransactionPoolStage::LocalAccepted),
            None,
            Some(Some(TransactionPoolConfirmedStage::ConfirmedPrepared)),
        )?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::AllAccepted), None, None)?;
        if count > 0 {
            return Ok(true);
        }
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::SomeAccepted), None, None)?;
        if count > 0 {
            return Ok(true);
        }

        Ok(count > 0)
    }

    pub fn count(&self, tx: &TStateStore::ReadTransaction<'_>) -> Result<usize, TransactionPoolError> {
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
        tx.transaction_pool_confirm_all_transitions(locked_block, new_locked_block, tx_ids)?;
        Ok(())
    }

    pub fn remove_all<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        tx_ids: I,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError> {
        TransactionPoolRecord::remove_all(tx, tx_ids)
    }
}

// Ord: ensure that the enum variants are ordered in the order of their progression
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
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
    /// All involved shard groups have prepared and all have pledged their local inputs
    AllPrepared,
    /// Some involved shard groups have prepared but one or more did not successfully pledge their local inputs
    SomePrepared,
    /// The local shard group has accepted the transaction
    LocalAccepted,
    /// All involved shard groups have accepted the transaction
    AllAccepted,
    /// Some involved shard groups have accepted the transaction, but one or more have decided to ABORT
    SomeAccepted,
    /// Only involves local shards. This transaction can be executed and accepted without cross-shard agreement.
    LocalOnly,
}

impl TransactionPoolStage {
    pub fn is_new(&self) -> bool {
        matches!(self, Self::New)
    }

    pub fn is_local_only(&self) -> bool {
        matches!(self, Self::LocalOnly)
    }

    pub fn is_prepared(&self) -> bool {
        matches!(self, Self::Prepared)
    }

    pub fn is_local_prepared(&self) -> bool {
        matches!(self, Self::LocalPrepared)
    }

    pub fn is_local_accepted(&self) -> bool {
        matches!(self, Self::LocalAccepted)
    }

    pub fn is_some_prepared(&self) -> bool {
        matches!(self, Self::SomePrepared)
    }

    pub fn is_all_prepared(&self) -> bool {
        matches!(self, Self::AllPrepared)
    }

    pub fn is_all_accepted(&self) -> bool {
        matches!(self, Self::AllAccepted)
    }

    pub fn is_some_accepted(&self) -> bool {
        matches!(self, Self::SomeAccepted)
    }

    pub fn is_finalising(&self) -> bool {
        self.is_local_only() || self.is_all_accepted() || self.is_some_accepted()
    }
}

impl Display for TransactionPoolStage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl FromStr for TransactionPoolStage {
    type Err = TransactionPoolStageFromStrErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "New" => Ok(TransactionPoolStage::New),
            "Prepared" => Ok(TransactionPoolStage::Prepared),
            "LocalPrepared" => Ok(TransactionPoolStage::LocalPrepared),
            "SomePrepared" => Ok(TransactionPoolStage::SomePrepared),
            "AllPrepared" => Ok(TransactionPoolStage::AllPrepared),
            "LocalAccepted" => Ok(TransactionPoolStage::LocalAccepted),
            "AllAccepted" => Ok(TransactionPoolStage::AllAccepted),
            "SomeAccepted" => Ok(TransactionPoolStage::SomeAccepted),
            "LocalOnly" => Ok(TransactionPoolStage::LocalOnly),
            s => Err(TransactionPoolStageFromStrErr(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Invalid TransactionPoolStage string '{0}'")]
pub struct TransactionPoolStageFromStrErr(String);

#[derive(Debug, Clone)]
pub enum TransactionPoolConfirmedStage {
    ConfirmedPrepared,
    ConfirmedAccepted,
}

impl Display for TransactionPoolConfirmedStage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionPoolConfirmedStage::ConfirmedPrepared => write!(f, "ConfirmedPrepared"),
            TransactionPoolConfirmedStage::ConfirmedAccepted => write!(f, "ConfirmedAccepted"),
        }
    }
}

impl FromStr for TransactionPoolConfirmedStage {
    type Err = TransactionPoolConfirmedStageFromStrErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ConfirmedPrepared" => Ok(TransactionPoolConfirmedStage::ConfirmedPrepared),
            "ConfirmedAccepted" => Ok(TransactionPoolConfirmedStage::ConfirmedAccepted),
            s => Err(TransactionPoolConfirmedStageFromStrErr(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Invalid TransactionPoolConfirmedStage string '{0}'")]
pub struct TransactionPoolConfirmedStageFromStrErr(String);

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct TransactionPoolRecord {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    transaction_id: TransactionId,
    evidence: Evidence,
    remote_evidence: Option<Evidence>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    transaction_fee: u64,
    leader_fee: Option<LeaderFee>,
    stage: TransactionPoolStage,
    pending_stage: Option<TransactionPoolStage>,
    original_decision: Decision,
    local_decision: Option<Decision>,
    remote_decision: Option<Decision>,
    is_ready: bool,
}

impl TransactionPoolRecord {
    pub fn load(
        id: TransactionId,
        evidence: Evidence,
        remote_evidence: Option<Evidence>,
        transaction_fee: Option<u64>,
        leader_fee: Option<LeaderFee>,
        stage: TransactionPoolStage,
        pending_stage: Option<TransactionPoolStage>,
        original_decision: Decision,
        local_decision: Option<Decision>,
        remote_decision: Option<Decision>,
        is_ready: bool,
    ) -> Self {
        Self {
            transaction_id: id,
            evidence,
            remote_evidence,
            transaction_fee: transaction_fee.unwrap_or(0),
            leader_fee,
            stage,
            pending_stage,
            original_decision,
            local_decision,
            remote_decision,
            is_ready,
        }
    }

    pub fn current_decision(&self) -> Decision {
        self.remote_decision()
            // Prioritize remote ABORT i.e. if accept we look at our local decision
            .filter(|d| d.is_abort())
            .unwrap_or_else(|| self.current_local_decision())
    }

    pub fn current_local_decision(&self) -> Decision {
        self.local_decision().unwrap_or(self.original_decision())
    }

    pub fn original_decision(&self) -> Decision {
        self.original_decision
    }

    pub fn local_decision(&self) -> Option<Decision> {
        self.local_decision
    }

    pub fn remote_decision(&self) -> Option<Decision> {
        self.remote_decision
    }

    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    fn evidence_mut(&mut self) -> &mut Evidence {
        &mut self.evidence
    }

    pub fn transaction_fee(&self) -> u64 {
        self.transaction_fee
    }

    /// Returns the committed stage of the transaction. This is the stage that has been confirmed by the local shard.
    pub fn committed_stage(&self) -> TransactionPoolStage {
        self.stage
    }

    /// Returns the pending stage of the transaction. This is the stage that the transaction is current but has not been
    /// confirmed by the local shard.
    pub fn pending_stage(&self) -> Option<TransactionPoolStage> {
        self.pending_stage
    }

    pub fn current_stage(&self) -> TransactionPoolStage {
        self.pending_stage.unwrap_or(self.stage)
    }

    pub fn leader_fee(&self) -> Option<&LeaderFee> {
        self.leader_fee.as_ref()
    }

    pub fn is_ready(&self) -> bool {
        self.is_ready
    }

    pub fn get_current_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            id: self.transaction_id,
            decision: self.current_decision(),
            evidence: self.evidence.clone(),
            transaction_fee: self.transaction_fee,
            leader_fee: self.leader_fee.clone(),
        }
    }

    pub fn get_local_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            id: self.transaction_id,
            decision: self.current_local_decision(),
            evidence: self.evidence.clone(),
            transaction_fee: self.transaction_fee,
            leader_fee: None,
        }
    }

    pub fn into_current_transaction_atom(self) -> TransactionAtom {
        TransactionAtom {
            id: self.transaction_id,
            decision: self.current_decision(),
            evidence: self.evidence,
            transaction_fee: self.transaction_fee,
            leader_fee: self.leader_fee,
        }
    }

    pub fn calculate_leader_fee(&self, num_involved_shards: NonZeroU64, exhaust_divisor: u64) -> LeaderFee {
        let target_burn = self.transaction_fee.checked_div(exhaust_divisor).unwrap_or(0);
        let block_fee_after_burn = self.transaction_fee - target_burn;

        let mut leader_fee = block_fee_after_burn / num_involved_shards;
        // The extra amount that is burnt from dividing the number of shards involved
        let excess_remainder_burn = block_fee_after_burn % num_involved_shards;

        // Adjust the leader fee to account for the remainder
        // If the remainder accounts for an extra burn of greater than half the number of involved shards, we
        // give each validator an extra 1 in fees if enough fees are available, burning less than the exhaust target.
        // Otherwise, we burn a little more than/equal to the exhaust target.
        let actual_burn = if excess_remainder_burn > 0 &&
            // If the div floor burn accounts for 1 less fee for more than half of number of shards, and ...
            excess_remainder_burn >= num_involved_shards.get() / 2 &&
            // ... if there are enough fees to pay out an additional 1 to all shards
            (leader_fee + 1) * num_involved_shards.get() <= self.transaction_fee
        {
            // Pay each leader 1 more
            leader_fee += 1;

            // We burn a little less due to the remainder
            target_burn.saturating_sub(num_involved_shards.get() - excess_remainder_burn)
        } else {
            // We burn a little more due to the remainder
            target_burn + excess_remainder_burn
        };

        LeaderFee {
            fee: leader_fee,
            global_exhaust_burn: actual_burn,
        }
    }

    pub fn set_remote_decision(&mut self, decision: Decision) -> &mut Self {
        // Only set remote_decision to ABORT, or COMMIT if it is not already ABORT
        self.remote_decision = self.remote_decision().map(|d| d.and(decision)).or(Some(decision));
        self
    }

    pub fn set_local_decision(&mut self, decision: Decision) -> &mut Self {
        self.local_decision = Some(decision);
        self
    }

    pub fn set_transaction_fee(&mut self, transaction_fee: u64) -> &mut Self {
        self.transaction_fee = transaction_fee;
        self
    }

    pub fn set_leader_fee(&mut self, leader_fee: LeaderFee) -> &mut Self {
        self.leader_fee = Some(leader_fee);
        self
    }

    pub fn update_from_execution(&mut self, execution: &TransactionExecution) -> &mut Self {
        let involved_locks = execution
            .resolved_inputs()
            .iter()
            .chain(execution.resulting_outputs())
            .map(|id| (id.to_substate_address(), id.lock_type()));

        self.evidence_mut().update(involved_locks);
        // Only change the local decision if we haven't already decided to ABORT
        if self.local_decision().map_or(true, |d| d.is_commit()) {
            self.set_local_decision(execution.decision());
        }
        self.set_transaction_fee(execution.transaction_fee());
        self
    }

    pub fn set_evidence(&mut self, evidence: Evidence) -> &mut Self {
        self.evidence = evidence;
        self
    }

    pub fn add_qc_evidence(&mut self, committee_info: &CommitteeInfo, qc_id: QcId) -> &mut Self {
        self.evidence.add_qc_evidence(committee_info, qc_id);
        self
    }

    pub fn check_pending_status_update(
        &self,
        pending_stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), TransactionPoolError> {
        // Check that only permitted stage transactions are performed
        match ((self.current_stage(), pending_stage), is_ready) {
            ((TransactionPoolStage::New, TransactionPoolStage::New), true) |
            ((TransactionPoolStage::New, TransactionPoolStage::Prepared), true) |
            ((TransactionPoolStage::New, TransactionPoolStage::LocalOnly), false) |
            // Prepared
            ((TransactionPoolStage::Prepared, TransactionPoolStage::LocalPrepared), _) |
            // LocalPrepared
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::LocalPrepared), _) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::AllPrepared), _) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::SomePrepared), _) |
            // AllPrepared
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::AllPrepared), false) |
            ((TransactionPoolStage::AllPrepared, TransactionPoolStage::LocalAccepted), _) |
            // SomePrepared
            ((TransactionPoolStage::SomePrepared, TransactionPoolStage::SomePrepared), false) |
            ((TransactionPoolStage::SomePrepared, TransactionPoolStage::LocalAccepted), _) |
            // LocalAccepted
            ((TransactionPoolStage::LocalAccepted, TransactionPoolStage::LocalAccepted), _) |
            ((TransactionPoolStage::LocalAccepted, TransactionPoolStage::AllAccepted), false) |
            ((TransactionPoolStage::LocalAccepted, TransactionPoolStage::SomeAccepted), false) |
            // Accepted
            ((TransactionPoolStage::AllAccepted, TransactionPoolStage::AllAccepted), false) => {}
            _ => {
                return Err(TransactionPoolError::InvalidTransactionTransition {
                    from: self.current_stage(),
                    to: pending_stage,
                    is_ready,
                });
            }
        }

        Ok(())
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
        self.check_pending_status_update(pending_stage, is_ready)?;

        let update = TransactionPoolStatusUpdate {
            block_id: block.block_id,
            transaction_id: self.transaction_id,
            stage: pending_stage,
            evidence: self.evidence.clone(),
            is_ready,
            local_decision: self.current_decision(),
        };

        update.insert(tx)?;
        self.pending_stage = Some(pending_stage);

        Ok(())
    }

    pub fn update_remote_data<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        decision: Decision,
        foreign_qc_id: QcId,
        foreign_committee_info: &CommitteeInfo,
        remote_evidence: Evidence,
    ) -> Result<(), TransactionPoolError> {
        match self.remote_evidence.as_mut() {
            Some(evidence) => {
                evidence.merge(remote_evidence);
            },
            None => {
                self.remote_evidence = Some(remote_evidence);
            },
        }

        tx.transaction_pool_update(
            &self.transaction_id,
            None,
            None,
            None,
            Some(decision),
            self.remote_evidence.as_ref(),
        )?;
        // TODO: we should not blindly use unknown foreign QCs
        self.evidence
            .merge(self.remote_evidence.as_ref().expect("set above").clone());
        self.add_qc_evidence(foreign_committee_info, foreign_qc_id);
        self.set_remote_decision(decision);
        Ok(())
    }

    #[allow(clippy::mutable_key_type)]
    pub fn add_foreign_pledges<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        shard_group: ShardGroup,
        foreign_pledges: SubstatePledges,
    ) -> Result<(), TransactionPoolError> {
        tx.foreign_substate_pledges_save(self.transaction_id, shard_group, foreign_pledges)?;
        Ok(())
    }

    pub fn update_local_data<TTx: StateStoreWriteTransaction>(
        &mut self,
        tx: &mut TTx,
        is_ready: bool,
    ) -> Result<(), TransactionPoolError> {
        if self
            .local_decision
            .map(|d| d != self.current_decision())
            .unwrap_or(true)
        {
            self.set_local_decision(self.current_decision());
            tx.transaction_pool_update(
                &self.transaction_id,
                Some(is_ready),
                Some(self.current_decision()),
                Some(&self.evidence),
                None,
                None,
            )?;
        }
        Ok(())
    }

    pub fn remove<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_remove(&self.transaction_id)?;
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

    pub fn remove_all<'a, TTx, I>(
        tx: &mut TTx,
        transaction_ids: I,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError>
    where
        TTx: StateStoreWriteTransaction,
        I: IntoIterator<Item = &'a TransactionId>,
    {
        let recs = tx.transaction_pool_remove_all(transaction_ids)?;
        // Clear any related foreign pledges
        tx.foreign_substate_pledges_remove_many(recs.iter().map(|rec| rec.transaction_id()))?;
        Ok(recs)
    }

    pub fn get_transaction<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<TransactionRecord, TransactionPoolError> {
        let transaction = TransactionRecord::get(tx, self.transaction_id())?;
        Ok(transaction)
    }

    pub fn get_execution_for_block<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        from_block_id: &BlockId,
    ) -> Result<BlockTransactionExecution, TransactionPoolError> {
        let exec = BlockTransactionExecution::get_pending_for_block(tx, self.transaction_id(), from_block_id)?;
        Ok(exec)
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
    #[error("Transaction already updated: {transaction_id} in block {block_id}")]
    TransactionAlreadyUpdated {
        transaction_id: TransactionId,
        block_id: BlockId,
    },
    #[error("Transaction already executed: {transaction_id} in block {block_id}")]
    TransactionAlreadyExecuted {
        transaction_id: TransactionId,
        block_id: BlockId,
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
    use rand::{rngs::OsRng, Rng};

    use super::*;
    use crate::consensus_models::LeaderFee;

    mod ordering {
        use super::*;

        #[test]
        fn it_is_ordered_correctly() {
            assert!(TransactionPoolStage::New < TransactionPoolStage::Prepared);
            assert!(TransactionPoolStage::Prepared < TransactionPoolStage::LocalPrepared);
            assert!(TransactionPoolStage::LocalPrepared < TransactionPoolStage::AllPrepared);
            assert!(TransactionPoolStage::LocalPrepared < TransactionPoolStage::SomePrepared);
            assert!(TransactionPoolStage::AllPrepared < TransactionPoolStage::LocalAccepted);
            assert!(TransactionPoolStage::SomePrepared < TransactionPoolStage::LocalAccepted);
            assert!(TransactionPoolStage::LocalAccepted < TransactionPoolStage::AllAccepted);
            assert!(TransactionPoolStage::LocalAccepted < TransactionPoolStage::SomeAccepted);
        }
    }

    mod calculate_leader_fee {
        use super::*;

        fn create_record_with_fee(fee: u64) -> TransactionPoolRecord {
            TransactionPoolRecord {
                transaction_id: TransactionId::new([0; 32]),
                original_decision: Decision::Commit,
                evidence: Default::default(),
                remote_evidence: None,
                transaction_fee: fee,
                leader_fee: None,
                stage: TransactionPoolStage::New,
                pending_stage: None,
                local_decision: None,
                remote_decision: None,
                is_ready: false,
            }
        }

        fn check_calculate_leader_fee(
            total_tx_fee: u64,
            total_num_involved_shards: u64,
            exhaust_divisor: u64,
        ) -> LeaderFee {
            let tx = create_record_with_fee(total_tx_fee);
            let leader_fee = tx.calculate_leader_fee(total_num_involved_shards.try_into().unwrap(), exhaust_divisor);
            // Total payable fee + burn is always equal to the total block fee
            assert_eq!(
                leader_fee.fee * total_num_involved_shards + leader_fee.global_exhaust_burn,
                total_tx_fee,
                "Fees were created or lost in the calculation. Expected: {}, Actual: {}",
                total_tx_fee,
                leader_fee.fee * total_num_involved_shards + leader_fee.global_exhaust_burn
            );

            let deviation_from_target_burn =
                leader_fee.global_exhaust_burn as f32 - (total_tx_fee.checked_div(exhaust_divisor).unwrap_or(0) as f32);
            assert!(
                deviation_from_target_burn.abs() <= total_num_involved_shards as f32,
                "Deviation from target burn is too high: {} (target: {}, actual: {}, num_shards: {}, divisor: {})",
                deviation_from_target_burn,
                total_tx_fee.checked_div(exhaust_divisor).unwrap_or(0),
                leader_fee.global_exhaust_burn,
                total_num_involved_shards,
                exhaust_divisor
            );

            leader_fee
        }

        #[test]
        fn it_calculates_the_correct_leader_fee() {
            let fee = check_calculate_leader_fee(100, 1, 20);
            assert_eq!(fee.fee, 95);
            assert_eq!(fee.global_exhaust_burn, 5);

            let fee = check_calculate_leader_fee(100, 1, 10);
            assert_eq!(fee.fee, 90);
            assert_eq!(fee.global_exhaust_burn, 10);

            let fee = check_calculate_leader_fee(100, 2, 0);
            assert_eq!(fee.fee, 50);
            assert_eq!(fee.global_exhaust_burn, 0);

            let fee = check_calculate_leader_fee(100, 2, 10);
            assert_eq!(fee.fee, 45);
            assert_eq!(fee.global_exhaust_burn, 10);

            let fee = check_calculate_leader_fee(100, 3, 0);
            assert_eq!(fee.fee, 33);
            // Even with no exhaust, we still burn 1 due to integer div floor
            assert_eq!(fee.global_exhaust_burn, 1);

            let fee = check_calculate_leader_fee(100, 3, 10);
            assert_eq!(fee.fee, 30);
            assert_eq!(fee.global_exhaust_burn, 10);

            let fee = check_calculate_leader_fee(98, 3, 10);
            assert_eq!(fee.fee, 30);
            assert_eq!(fee.global_exhaust_burn, 8);

            let fee = check_calculate_leader_fee(98, 3, 21);
            assert_eq!(fee.fee, 32);
            // target burn is 4, but the remainder burn is 5, so we give 1 more to the leaders and burn 2
            assert_eq!(fee.global_exhaust_burn, 2);

            // Target burn is 8, and the remainder burn is 8, so we burn 8
            let fee = check_calculate_leader_fee(98, 10, 10);
            assert_eq!(fee.fee, 9);
            assert_eq!(fee.global_exhaust_burn, 8);

            let fee = check_calculate_leader_fee(19802, 45, 20);
            assert_eq!(fee.fee, 418);
            assert_eq!(fee.global_exhaust_burn, 992);

            // High burn amount due to not enough fees to pay out all involved shards to compensate
            let fee = check_calculate_leader_fee(311, 45, 20);
            assert_eq!(fee.fee, 6);
            assert_eq!(fee.global_exhaust_burn, 41);
        }

        #[test]
        fn simple_fuzz() {
            let mut total_fees = 0;
            let mut total_burnt = 0;
            for _ in 0..1_000_000 {
                let fee = OsRng.gen_range(100..100000u64);
                let involved = OsRng.gen_range(1..100u64);
                let fee = check_calculate_leader_fee(fee, involved, 20);
                total_fees += fee.fee * involved;
                total_burnt += fee.global_exhaust_burn;
            }

            println!(
                "total fees: {}, total burnt: {}, {}%",
                total_fees,
                total_burnt,
                // Should approach 5%, tends to be ~5.25%
                (total_burnt as f64 / total_fees as f64) * 100.0
            );
        }
    }
}
