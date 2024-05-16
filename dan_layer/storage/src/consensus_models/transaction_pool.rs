//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    marker::PhantomData,
    num::NonZeroU64,
    str::FromStr,
};

use log::*;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::{
    committee::CommitteeInfo,
    optional::{IsNotFoundError, Optional},
};
use tari_transaction::TransactionId;

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

const LOG_TARGET: &str = "tari::dan::storage::transaction_pool";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, Deserialize),
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
    /// Only involves local shards
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
            TransactionPoolStage::LocalOnly |
            TransactionPoolStage::AllPrepared |
            TransactionPoolStage::SomePrepared => None,
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
            "LocalOnly" => Ok(TransactionPoolStage::LocalOnly),
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
            target: LOG_TARGET,
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

    pub fn set_atom(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        transaction: TransactionAtom,
    ) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_set_atom(transaction)?;
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
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::LocalOnly), None, None)?;
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
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
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

    pub fn is_deferred(&self) -> bool {
        self.original_decision().is_deferred()
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

    pub fn get_final_transaction_atom(&self, leader_fee: LeaderFee) -> TransactionAtom {
        TransactionAtom {
            decision: self.current_decision(),
            leader_fee: Some(leader_fee),
            ..self.transaction.clone()
        }
    }

    pub fn get_local_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            decision: self.current_local_decision(),
            ..self.transaction.clone()
        }
    }

    pub fn calculate_leader_fee(&self, num_involved_shards: NonZeroU64, exhaust_divisor: u64) -> LeaderFee {
        let transaction_fee = self.transaction.transaction_fee;
        let target_burn = transaction_fee.checked_div(exhaust_divisor).unwrap_or(0);
        let block_fee_after_burn = transaction_fee - target_burn;

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
            (leader_fee + 1) * num_involved_shards.get() <= transaction_fee
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

    pub fn add_evidence(&mut self, committee_info: &CommitteeInfo, qc_id: QcId) -> &mut Self {
        let evidence = &mut self.transaction.evidence;
        for (address, evidence_mut) in evidence.iter_mut() {
            if committee_info.includes_substate_address(address) {
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
            ((TransactionPoolStage::New, TransactionPoolStage::LocalOnly), false) |
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
                });
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
        foreign_committee_info: &CommitteeInfo,
    ) -> Result<(), TransactionPoolError> {
        self.add_evidence(foreign_committee_info, foreign_qc_id);
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct LeaderFee {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub fee: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub global_exhaust_burn: u64,
}

impl LeaderFee {
    pub fn fee(&self) -> u64 {
        self.fee
    }

    pub fn global_exhaust_burn(&self) -> u64 {
        self.global_exhaust_burn
    }
}

impl Display for LeaderFee {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Leader fee: {}, Burnt: {}", self.fee, self.global_exhaust_burn)
    }
}

#[cfg(test)]
mod tests {
    use rand::{rngs::OsRng, Rng};

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
                    leader_fee: None,
                },
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
