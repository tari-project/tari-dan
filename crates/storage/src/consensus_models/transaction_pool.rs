//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    clone::Clone,
    fmt::{Display, Formatter},
    marker::PhantomData,
    num::NonZeroU64,
    str::FromStr,
    time::Duration,
};

use log::*;
use serde::{Deserialize, Serialize};
use tari_consensus_types::{BlockId, Decision, LeafBlock};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    ShardGroup,
    SubstateAddress,
    SubstateLockType,
    committee::CommitteeInfo,
    displayable::Displayable,
    optional::IsNotFoundError,
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_template_lib_types::TransactionReceiptAddress;

use crate::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{
        BlockTransactionExecution,
        Evidence,
        LeaderFee,
        LockedEpoch,
        TransactionAtom,
        TransactionExecution,
        TransactionRecord,
        calculate_leader_fee,
    },
};

const LOG_TARGET: &str = "tari::ootle::storage::transaction_pool";

#[derive(Debug, Clone, Default)]
pub struct TransactionPool<TStateStore> {
    _store: PhantomData<TStateStore>,
}

impl<TStateStore: StateStore> TransactionPool<TStateStore> {
    pub fn new() -> Self {
        Self { _store: PhantomData }
    }

    pub fn exists(
        &self,
        tx: &impl StateStoreReadTransaction,
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
        initial_evidence: &Evidence,
        is_ready: bool,
        is_global: bool,
        max_epoch: Epoch,
        transaction_weight: u64,
    ) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_insert_new(
            tx_id,
            decision,
            initial_evidence,
            is_ready,
            is_global,
            max_epoch,
            transaction_weight,
        )?;
        Ok(())
    }

    pub fn insert_new_batched<'a, I: IntoIterator<Item = (&'a TransactionRecord, Decision, bool)>>(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        num_preshards: NumPreshards,
        num_committees: u32,
        transactions: I,
    ) -> Result<(), TransactionPoolError> {
        for (transaction, decision, is_ready) in transactions {
            tx.transaction_pool_insert_new(
                *transaction.id(),
                decision,
                &transaction.to_initial_evidence(num_preshards, num_committees),
                is_ready,
                transaction.transaction().is_global(),
                transaction.transaction().max_epoch(),
                transaction.transaction().calculate_transaction_weight().as_u64(),
            )?;
        }
        Ok(())
    }

    pub fn get_all(
        &self,
        tx: &impl StateStoreReadTransaction,
        limit: usize,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError> {
        let recs = tx.transaction_pool_get_all(limit)?;
        Ok(recs)
    }

    /// Fetch ready transactions for the next block, bounded by a weight budget and a hard command
    /// count cap. Records are accumulated in order until either their cumulative
    /// [`TransactionPoolRecord::proposal_weight`] would exceed `weight_budget` or `max_count` records
    /// have been collected. At least one ready record is always returned (if any exist) so that a
    /// single transaction heavier than the whole budget still makes progress.
    pub fn get_batch_for_next_block(
        &self,
        tx: &impl StateStoreReadTransaction,
        weight_budget: u64,
        max_count: usize,
        block_id: &BlockId,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError> {
        if weight_budget == 0 || max_count == 0 {
            return Ok(Vec::new());
        }
        let recs = tx.transaction_pool_get_many_ready(weight_budget, max_count, block_id)?;
        Ok(recs)
    }

    pub fn has_ready_or_pending_transaction_updates(
        &self,
        tx: &impl StateStoreReadTransaction,
        block_id: &BlockId,
    ) -> Result<bool, TransactionPoolError> {
        // Check if any pending transactions have state updates that need to be applied
        if tx.transaction_pool_has_pending_state_updates(block_id)? {
            debug!(
                target: LOG_TARGET,
                "has_ready_or_pending_transaction_updates: Pending state updates found",
            );
            return Ok(true);
        }
        debug!(
            target: LOG_TARGET,
            "has_ready_or_pending_transaction_updates: No pending state updates",
        );

        // Check if any transactions are marked as ready to propose
        let count = tx.transaction_pool_count(None, Some(true), true)?;
        if count > 0 {
            debug!(
                target: LOG_TARGET,
                "has_ready_or_pending_transaction_updates: {} transactions marked as ready",
                count,
            );
            return Ok(true);
        }
        debug!(
            target: LOG_TARGET,
            "has_ready_or_pending_transaction_updates: No transactions marked as ready",
        );

        // Check if we have transactions that have not yet been confirmed (locked). In this case we should propose
        // until this stage is locked.
        // let count = tx.transaction_pool_count(None, None, Some(None))?;
        // if count > 0 {
        //     return Ok(true);
        // }

        let count = tx.transaction_pool_count(Some(TransactionPoolStage::LocalOnly), None, true)?;
        if count > 0 {
            debug!(
                target: LOG_TARGET,
                "has_ready_or_pending_transaction_updates: {} transactions that need to be finalized (LocalOnly)",
                count,
            );
            return Ok(true);
        }

        // Check if we have multishard transactions that need to be finalized. These checks apply to transactions that
        // have been locked but not committed.
        let count = tx.transaction_pool_count(Some(TransactionPoolStage::AllAccepted), None, true)?;
        if count > 0 {
            debug!(
                target: LOG_TARGET,
                "has_ready_or_pending_transaction_updates: {} transactions that need to be finalized (AllAccepted)",
                count,
            );
            return Ok(true);
        }

        let count = tx.transaction_pool_count(Some(TransactionPoolStage::SomeAccepted), None, true)?;
        if count > 0 {
            debug!(
                target: LOG_TARGET,
                "has_ready_or_pending_transaction_updates: {} transactions that need to be finalized (SomeAccepted)",
                count,
            );
            return Ok(true);
        }

        debug!(
            target: LOG_TARGET,
            "has_ready_or_pending_transaction_updates: No transactions that need to be finalized",
        );

        Ok(false)
    }

    pub fn count(&self, tx: &impl StateStoreReadTransaction) -> Result<usize, TransactionPoolError> {
        let count = tx.transaction_pool_count(None, None, false)?;
        Ok(count)
    }

    pub fn confirm_all_transitions(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        block: &LeafBlock,
    ) -> Result<(), TransactionPoolError> {
        tx.transaction_pool_confirm_all_transitions(block)?;
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
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum TransactionPoolStage {
    /// Transaction has just come in and has never been proposed
    #[n(0)]
    New,
    /// Transaction is prepared in response to a LocalPrepare command. We have proof that all local committees have
    /// prepared the transaction
    #[n(1)]
    LocalPrepared,
    /// All (Commit), Some or None (Abort) of involved shard groups have prepared and all have pledged their local
    /// inputs The local shard group should accept the transaction
    #[n(2)]
    LocalAccepted,
    /// All involved shard groups have accepted the transaction
    #[n(3)]
    AllAccepted,
    /// Some involved shard groups have accepted the transaction, but one or more have decided to ABORT
    #[n(4)]
    SomeAccepted,
    /// Only involves local shards. This transaction can be executed and accepted without cross-shard agreement.
    #[n(5)]
    LocalOnly,
}

impl TransactionPoolStage {
    pub fn is_new(&self) -> bool {
        matches!(self, Self::New)
    }

    pub fn is_local_only(&self) -> bool {
        matches!(self, Self::LocalOnly)
    }

    pub fn is_local_prepared(&self) -> bool {
        matches!(self, Self::LocalPrepared)
    }

    pub fn is_local_accepted(&self) -> bool {
        matches!(self, Self::LocalAccepted)
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

    /// Heuristic cost (as a percentage of the transaction's static weight) of proposing the next
    /// command for a record at this stage. Phases that execute the transaction and/or carry its full
    /// footprint cost full weight; finalisation phases reuse an already-computed execution and only
    /// carry a small atom, so they are discounted. Used purely as a local block-packing heuristic.
    pub fn proposal_weight_percent(&self) -> u64 {
        match self {
            // Prepare phase. May execute (LocalOnly / output-only) or only resolve+pledge local inputs,
            // but the command carries the transaction's full footprint either way. Charged in full.
            TransactionPoolStage::New => 100,
            // Accept phase: executes the transaction with all gathered pledges.
            TransactionPoolStage::LocalPrepared => 100,
            // Finalisation phases: reuse the prior execution and apply its diff (or abort). No
            // re-execution and only a compact atom is proposed.
            TransactionPoolStage::LocalAccepted |
            TransactionPoolStage::AllAccepted |
            TransactionPoolStage::SomeAccepted |
            TransactionPoolStage::LocalOnly => 35,
        }
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
            "LocalPrepared" => Ok(TransactionPoolStage::LocalPrepared),
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

// TODO: remove
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

#[derive(Debug, Clone, Serialize, Deserialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TransactionPoolRecord {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(0)]
    transaction_id: TransactionId,
    #[n(1)]
    evidence: Evidence,
    #[n(2)]
    is_global: bool,
    #[n(3)]
    transaction_fee: u64,
    #[n(4)]
    leader_fee: Option<LeaderFee>,
    #[n(5)]
    stage: TransactionPoolStage,
    #[n(6)]
    pending_stage: Option<TransactionPoolStage>,
    #[n(7)]
    original_decision: Decision,
    #[n(8)]
    local_decision: Option<Decision>,
    #[n(9)]
    remote_decision: Option<Decision>,
    #[n(10)]
    is_ready: bool,
    /// The maximum epoch for which this transaction is valid.
    #[n(11)]
    max_epoch: Epoch,
    /// Epoch to use when executing the transaction. This updates as foreign proposals are received
    /// until the transaction is executed.
    #[n(12)]
    locked_epoch: Option<LockedEpoch>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    // time::OffsetDateTime is foreign (time crate) — bridge through serde.
    #[n(13)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    last_updated: time::OffsetDateTime,
    #[n(14)]
    last_updated_in_block: Option<BlockId>,
    /// Static transaction weight (`Transaction::calculate_transaction_weight`), cached here so block
    /// proposal can budget by weight without loading the full transaction body. Computed once when the
    /// record is created.
    #[serde(default)]
    #[cbor(default)]
    #[n(15)]
    transaction_weight: u64,
    /// The exhaust burn collected by the executor (`FeeReceipt::exhaust_burn`), set from local
    /// execution alongside `transaction_fee`.
    #[serde(default)]
    #[cbor(default)]
    #[n(16)]
    exhaust_burn: u64,
}

impl TransactionPoolRecord {
    pub fn new_from_transaction(transaction: &Transaction, initial_evidence: Evidence) -> Self {
        Self {
            transaction_id: transaction.calculate_id(),
            evidence: initial_evidence,
            is_global: transaction.is_global(),
            transaction_fee: 0,
            leader_fee: None,
            stage: TransactionPoolStage::New,
            pending_stage: None,
            original_decision: Decision::Commit,
            local_decision: None,
            remote_decision: None,
            is_ready: false,
            max_epoch: transaction.max_epoch(),
            locked_epoch: None,
            last_updated: time::OffsetDateTime::now_utc(),
            last_updated_in_block: None,
            transaction_weight: transaction.calculate_transaction_weight().as_u64(),
            exhaust_burn: 0,
        }
    }

    pub fn load(
        id: TransactionId,
        evidence: Evidence,
        is_global: bool,
        transaction_fee: u64,
        leader_fee: Option<LeaderFee>,
        stage: TransactionPoolStage,
        pending_stage: Option<TransactionPoolStage>,
        original_decision: Decision,
        local_decision: Option<Decision>,
        remote_decision: Option<Decision>,
        is_ready: bool,
        max_epoch: Epoch,
        locked_epoch: Option<LockedEpoch>,
        last_updated: time::OffsetDateTime,
        last_updated_in_block: Option<BlockId>,
        transaction_weight: u64,
        exhaust_burn: u64,
    ) -> Self {
        Self {
            transaction_id: id,
            evidence,
            is_global,
            transaction_fee,
            leader_fee,
            stage,
            pending_stage,
            original_decision,
            local_decision,
            remote_decision,
            is_ready,
            max_epoch,
            locked_epoch,
            last_updated,
            last_updated_in_block,
            transaction_weight,
            exhaust_burn,
        }
    }

    pub fn current_decision(&self) -> Decision {
        self.remote_decision()
            // Prioritize remote ABORT i.e. if accept we look at our local decision
            .filter(|d| d.is_abort())
            .unwrap_or_else(|| self.current_local_decision())
    }

    fn can_continue_to(&self, stage: TransactionPoolStage, local_shard_group: ShardGroup) -> bool {
        match stage {
            TransactionPoolStage::New => self.is_ready,
            TransactionPoolStage::LocalPrepared => match self.current_decision() {
                Decision::Commit => self.evidence.all_input_shard_groups_prepared(local_shard_group),
                Decision::Abort(_) => self.evidence.some_shard_groups_prepared(),
            },
            TransactionPoolStage::LocalAccepted => match self.current_decision() {
                Decision::Commit => self.evidence.all_shard_groups_accepted(),
                // If we have decided to abort, we can continue if any foreign shard or locally has prepared
                Decision::Abort(_) => self.evidence.some_shard_groups_prepared(),
            },
            TransactionPoolStage::AllAccepted |
            TransactionPoolStage::SomeAccepted |
            TransactionPoolStage::LocalOnly => false,
        }
    }

    pub fn is_ready_for_pending_stage(&self, local_shard_group: ShardGroup) -> bool {
        self.can_continue_to(self.current_stage(), local_shard_group)
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

    pub fn max_epoch(&self) -> Epoch {
        self.max_epoch
    }

    pub fn locked_epoch(&self) -> Option<&LockedEpoch> {
        self.locked_epoch.as_ref()
    }

    pub fn id(&self) -> &TransactionId {
        &self.transaction_id
    }

    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    pub fn evidence_mut(&mut self) -> &mut Evidence {
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

    pub fn stage(&self) -> TransactionPoolStage {
        self.stage
    }

    /// The cached static transaction weight (see `Transaction::calculate_transaction_weight`).
    pub fn transaction_weight(&self) -> u64 {
        self.transaction_weight
    }

    /// The weight this record contributes to a block when proposing its next command. It is the static
    /// transaction weight scaled by the per-phase cost of the next command (see
    /// [`TransactionPoolStage::proposal_weight_percent`]). Always at least 1 so every command consumes
    /// some of the block weight budget. This is a local proposing heuristic, not a consensus rule.
    pub fn proposal_weight(&self) -> u64 {
        let percent = self.current_stage().proposal_weight_percent();
        self.transaction_weight.saturating_mul(percent).div_ceil(100).max(1)
    }

    pub fn leader_fee(&self) -> Option<&LeaderFee> {
        if self.current_decision().is_abort() {
            return None;
        }
        self.leader_fee.as_ref()
    }

    pub fn is_ready(&self) -> bool {
        self.is_ready
    }

    pub fn to_receipt_id(&self) -> TransactionReceiptAddress {
        (*self.id()).into()
    }

    pub fn get_current_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            id: self.transaction_id,
            decision: self.current_decision(),
            evidence: self.evidence.clone(),
            transaction_fee: self.transaction_fee,
            leader_fee: self.leader_fee().cloned(),
        }
    }

    pub fn get_local_transaction_atom(&self) -> TransactionAtom {
        TransactionAtom {
            id: self.transaction_id,
            decision: self.current_local_decision(),
            evidence: self.evidence.clone(),
            transaction_fee: self.transaction_fee,
            leader_fee: self.leader_fee().cloned(),
        }
    }

    pub fn into_current_transaction_atom(self) -> TransactionAtom {
        TransactionAtom {
            id: self.transaction_id,
            decision: self.current_decision(),
            leader_fee: self.leader_fee().cloned(),
            evidence: self.evidence,
            transaction_fee: self.transaction_fee,
        }
    }

    pub fn is_global(&self) -> bool {
        self.is_global
    }

    pub fn calculate_leader_fee(&self, num_involved_shards: NonZeroU64) -> LeaderFee {
        calculate_leader_fee(self.transaction_fee, self.exhaust_burn, num_involved_shards)
    }

    pub fn set_remote_decision(&mut self, decision: Decision) -> &mut Self {
        // Only set remote_decision to ABORT, or COMMIT if it is not already ABORT
        let decision = self.remote_decision().map(|d| d.and(decision)).unwrap_or(decision);
        self.remote_decision = Some(decision);
        if decision.is_abort() {
            self.evidence.abort();
        }
        self
    }

    pub fn set_local_decision(&mut self, decision: Decision) -> &mut Self {
        self.local_decision = Some(decision);
        // Represents that no substates are locked/pledged when ABORT
        if decision.is_abort() {
            self.evidence.abort();
        }
        self
    }

    pub fn set_transaction_fee(&mut self, transaction_fee: u64) -> &mut Self {
        self.transaction_fee = transaction_fee;
        self
    }

    pub fn exhaust_burn(&self) -> u64 {
        self.exhaust_burn
    }

    pub fn set_exhaust_burn(&mut self, exhaust_burn: u64) -> &mut Self {
        self.exhaust_burn = exhaust_burn;
        self
    }

    pub fn set_leader_fee(&mut self, leader_fee: LeaderFee) -> &mut Self {
        self.leader_fee = Some(leader_fee);
        self
    }

    pub fn no_leader_fee(&mut self) -> &mut Self {
        self.leader_fee = None;
        self
    }

    pub fn set_is_ready(&mut self, is_ready: bool) -> &mut Self {
        self.is_ready = is_ready;
        self
    }

    pub fn set_pending_stage(&mut self, pending_stage: Option<TransactionPoolStage>) -> &mut Self {
        self.pending_stage = pending_stage;
        self
    }

    pub fn set_last_updated(&mut self, in_block: BlockId, timestamp: time::OffsetDateTime) -> &mut Self {
        self.last_updated_in_block = Some(in_block);
        self.last_updated = timestamp;
        self
    }

    /// Updates the locked epoch if the new epoch is less than the current locked epoch.
    /// Returns true if the locked epoch was updated, otherwise false.
    pub fn update_locked_epoch(&mut self, new_epoch: LockedEpoch) -> bool {
        if self.locked_epoch.as_ref().is_none_or(|e| e.epoch() > new_epoch.epoch()) {
            self.locked_epoch = Some(new_epoch);
            return true;
        }
        false
    }

    /// Sets the locked epoch to the given value.
    /// This is used for database loading and transaction pool record update merging only.
    /// Use `update_locked_epoch` to update the locked epoch during foreign proposal etc processing.
    pub fn set_locked_epoch(&mut self, locked_epoch: Option<LockedEpoch>) -> &mut Self {
        self.locked_epoch = locked_epoch;
        self
    }

    pub fn set_stage(&mut self, stage: TransactionPoolStage) -> &mut Self {
        self.stage = stage;
        self
    }

    pub fn update_from_execution(
        &mut self,
        num_preshards: NumPreshards,
        num_committees: u32,
        execution: &TransactionExecution,
    ) -> &mut Self {
        // Only change the local decision if we haven't already decided to ABORT
        if self.local_decision().is_none_or(|d| d.is_commit()) {
            self.set_local_decision(execution.decision());
        }

        if self.current_decision().is_commit() {
            let involved_locks = execution.resolved_inputs().iter().chain(execution.resulting_outputs());
            for lock in involved_locks {
                self.evidence_mut()
                    .insert_from_lock_intent(num_preshards, num_committees, lock);
            }
        } else {
            self.evidence.abort();
        }

        self.set_transaction_fee(execution.transaction_fee());
        self.set_exhaust_burn(execution.exhaust_burn());
        self
    }

    pub fn set_next_stage_and_readiness(
        &mut self,
        next_stage: TransactionPoolStage,
        local_shard_group: ShardGroup,
    ) -> Result<(), TransactionPoolError> {
        let is_ready = self.can_continue_to(next_stage, local_shard_group);
        self.check_pending_status_update(next_stage, is_ready)?;
        info!(
            target: LOG_TARGET,
            "📝 Setting next update for transaction {} to {}->{},is_ready={}->{},{}->{}",
            self.id(),
            self.current_stage(),
            next_stage,
            self.is_ready,
            is_ready,
            self.current_local_decision(),
            self.current_decision(),
        );
        self.pending_stage = Some(next_stage);
        self.is_ready = is_ready;
        Ok(())
    }

    pub fn set_ready(&mut self, is_ready: bool) -> &mut Self {
        self.is_ready = is_ready;
        self
    }

    /// Sets the evidence for the transaction pool record. This replaces any existing evidence.
    pub fn set_evidence(&mut self, evidence: Evidence) -> &mut Self {
        self.evidence = evidence;
        self
    }

    pub fn since_last_updated(&self) -> Duration {
        let d = time::OffsetDateTime::now_utc() - self.last_updated;
        // If d is negative (perhaps only possible with a mal-timed clock adjustment), we treat this like a saturating
        // sub (zero duration)
        d.try_into().unwrap_or_default()
    }

    pub fn last_updated_in_block(&self) -> Option<&BlockId> {
        self.last_updated_in_block.as_ref()
    }

    /// Merges the given evidence into the existing evidence of the transaction pool record.
    /// This ensures that QC evidence is preserved (neither added not removed), adding only new lock evidence.
    /// TODO: we also add inputs/outputs - this _shouldn't be_ necessary because the node should have already added
    /// this. Determine if there are any cases which need this e.g outputs need to be added initially to
    /// preserve version info before local execution.
    pub fn merge_evidence(&mut self, evidence: Evidence) -> &mut Self {
        self.evidence.merge(&evidence);
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
            ((TransactionPoolStage::New, TransactionPoolStage::LocalPrepared), _) |
            ((TransactionPoolStage::New, TransactionPoolStage::LocalOnly), false) |
            // Output-only case - we can skip straight to LocalAccepted
            ((TransactionPoolStage::New, TransactionPoolStage::LocalAccepted), _) |
            // LocalPrepared
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::LocalPrepared), _) |
            ((TransactionPoolStage::LocalPrepared, TransactionPoolStage::LocalAccepted), _) |
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
    pub fn remove_all<'a, TTx, I>(
        tx: &mut TTx,
        transaction_ids: I,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError>
    where
        TTx: StateStoreWriteTransaction,
        I: IntoIterator<Item = &'a TransactionId>,
    {
        let recs = tx.transaction_pool_remove_all(transaction_ids)?;
        let iter = recs.iter().map(|rec| rec.id());
        // Clear any related foreign pledges
        tx.foreign_substate_pledges_remove_many(iter.clone())?;
        // Clear any related lock_conflicts
        tx.lock_conflicts_remove_by_transaction_ids(iter)?;
        Ok(recs)
    }

    pub fn get<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        to_block_id: &BlockId,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, TransactionPoolError> {
        let rec = tx.transaction_pool_get_for_blocks(to_block_id, transaction_id)?;
        Ok(rec)
    }

    pub fn get_transaction<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<TransactionRecord, TransactionPoolError> {
        let transaction = TransactionRecord::get(tx, self.id())?;
        Ok(transaction)
    }

    pub fn get_pending_execution_for_block<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        from_block: &LeafBlock,
    ) -> Result<BlockTransactionExecution, TransactionPoolError> {
        let exec = BlockTransactionExecution::get_pending_for_block(tx, self.id(), from_block)?;
        Ok(exec)
    }

    pub fn involves_committee(&self, committee_info: &CommitteeInfo) -> bool {
        self.evidence.contains(&committee_info.shard_group())
    }

    pub fn committee_involves_inputs(&self, committee_info: &CommitteeInfo) -> bool {
        self.evidence
            .get(&committee_info.shard_group())
            .is_some_and(|e| !e.inputs().is_empty())
    }

    pub fn has_all_required_foreign_pledges<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        local_committee_info: &CommitteeInfo,
    ) -> Result<bool, StorageError> {
        let involved_objects = self
            .evidence()
            .all_inputs_iter()
            .map(|(_, substate_id, evidence)| (substate_id, evidence.map(|e| (e.version, e.as_lock_type()))))
            .chain(
                self.evidence()
                    .all_outputs_iter()
                    .map(|(_, substate_id, version)| (substate_id, Some((*version, SubstateLockType::Output)))),
            )
            .filter(|(substate_id, _)| !local_committee_info.includes_substate_id(substate_id));

        self.has_foreign_pledges_for_objects(tx, local_committee_info, involved_objects)
    }

    pub fn has_all_required_foreign_input_pledges<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        local_committee_info: &CommitteeInfo,
    ) -> Result<bool, StorageError> {
        let involved_objects = self
            .evidence()
            .all_inputs_iter()
            .map(|(_, substate_id, evidence)| (substate_id, evidence.map(|e| (e.version, e.as_lock_type()))))
            .filter(|(substate_id, _)| !local_committee_info.includes_substate_id(substate_id));

        self.has_foreign_pledges_for_objects(tx, local_committee_info, involved_objects)
    }

    fn has_foreign_pledges_for_objects<'a, TTx, TObj>(
        &self,
        tx: &TTx,
        local_committee_info: &CommitteeInfo,
        involved_objects: TObj,
    ) -> Result<bool, StorageError>
    where
        TTx: StateStoreReadTransaction,
        TObj: IntoIterator<Item = (&'a SubstateId, Option<(u64, SubstateLockType)>)>,
    {
        for (substate_id, data) in involved_objects {
            let Some((version, lock_type)) = data else {
                debug!(
                    target: LOG_TARGET,
                    "Transaction {} is missing a version for substate_id {}",
                    self.id(),
                    substate_id,
                );
                return Ok(false);
            };
            let address = SubstateAddress::from_substate_id(substate_id, version);
            // TODO(perf): O(n) queries
            if tx.foreign_substate_pledges_exists_for_transaction_and_address(self.id(), address)? {
                continue;
            }

            if log_enabled!(Level::Debug) {
                // Load them for debugging purposes
                let pledges = tx.foreign_substate_pledges_get_all_by_transaction_id(self.id())?;
                let remote_shard_group = address.to_shard_group(
                    local_committee_info.num_preshards(),
                    local_committee_info.num_committees(),
                );
                debug!(
                    target: LOG_TARGET,
                    "pledges: {}",
                    pledges.display(),
                );
                debug!(
                    target: LOG_TARGET,
                    "{} Transaction {} is missing a foreign {} pledge for {}:{} from {} ({} pledge(s) found)",
                    local_committee_info.shard_group(),
                    self.id(),
                    lock_type,
                    substate_id,
                    version,
                    remote_shard_group,
                    pledges.len(),
                );
            }

            return Ok(false);
        }
        Ok(true)
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

    use rand::RngExt;

    use super::*;
    use crate::consensus_models::LeaderFee;

    mod ordering {
        use super::*;

        #[test]
        fn it_is_ordered_correctly() {
            assert!(TransactionPoolStage::New < TransactionPoolStage::LocalPrepared);
            assert!(TransactionPoolStage::LocalAccepted < TransactionPoolStage::AllAccepted);
            assert!(TransactionPoolStage::LocalAccepted < TransactionPoolStage::SomeAccepted);
        }
    }

    mod proposal_weight {
        use super::*;

        fn record_with_weight_and_stage(weight: u64, stage: TransactionPoolStage) -> TransactionPoolRecord {
            TransactionPoolRecord {
                transaction_id: TransactionId::new([0; 32]),
                original_decision: Decision::Commit,
                evidence: Default::default(),
                transaction_fee: 0,
                leader_fee: None,
                stage,
                is_global: false,
                pending_stage: None,
                local_decision: None,
                remote_decision: None,
                is_ready: true,
                max_epoch: Epoch(1),
                locked_epoch: None,
                last_updated: time::OffsetDateTime::now_utc(),
                last_updated_in_block: None,
                transaction_weight: weight,
                exhaust_burn: 0,
            }
        }

        #[test]
        fn executing_phases_cost_full_weight() {
            assert_eq!(TransactionPoolStage::New.proposal_weight_percent(), 100);
            assert_eq!(TransactionPoolStage::LocalPrepared.proposal_weight_percent(), 100);

            let rec = record_with_weight_and_stage(200, TransactionPoolStage::New);
            assert_eq!(rec.proposal_weight(), 200);
        }

        #[test]
        fn finalisation_phases_are_discounted() {
            // Finalisation phases reuse a prior execution and carry only a compact atom.
            for stage in [
                TransactionPoolStage::LocalAccepted,
                TransactionPoolStage::AllAccepted,
                TransactionPoolStage::SomeAccepted,
                TransactionPoolStage::LocalOnly,
            ] {
                assert!(
                    stage.proposal_weight_percent() < 100,
                    "{stage} should be discounted relative to an executing phase"
                );
                let rec = record_with_weight_and_stage(200, stage);
                assert!(
                    rec.proposal_weight() < 200,
                    "{stage} proposal weight should be discounted"
                );
            }
        }

        #[test]
        fn proposal_weight_is_at_least_one() {
            // A zero-weight (e.g. legacy/empty) record still consumes some budget so the count cap
            // is the only thing bounding it, never an unbounded fill.
            let rec = record_with_weight_and_stage(0, TransactionPoolStage::New);
            assert_eq!(rec.proposal_weight(), 1);
        }
    }

    mod calculate_leader_fee {

        use super::*;

        fn create_record_with_fee(fee: u64, exhaust_burn: u64) -> TransactionPoolRecord {
            TransactionPoolRecord {
                transaction_id: TransactionId::new([0; 32]),
                original_decision: Decision::Commit,
                evidence: Default::default(),
                transaction_fee: fee,
                leader_fee: None,
                stage: TransactionPoolStage::New,
                is_global: false,
                pending_stage: None,
                local_decision: None,
                remote_decision: None,
                is_ready: false,
                max_epoch: Epoch(1),
                locked_epoch: None,
                last_updated: time::OffsetDateTime::now_utc(),
                last_updated_in_block: None,
                transaction_weight: 0,
                exhaust_burn,
            }
        }

        fn check_calculate_leader_fee(
            total_tx_fee: u64,
            exhaust_burn: u64,
            total_num_involved_shards: u64,
        ) -> LeaderFee {
            let tx = create_record_with_fee(total_tx_fee, exhaust_burn);
            let leader_fee = tx.calculate_leader_fee(total_num_involved_shards.try_into().unwrap());
            // Every amount withheld from the network is accounted for exactly: the fees paid out to leaders plus
            // the whole-transaction burn equal the transaction fee plus the executor-collected burn.
            assert_eq!(
                leader_fee.fee * total_num_involved_shards + leader_fee.exhaust_burn,
                total_tx_fee + exhaust_burn,
                "Fees were created or lost. total_tx_fee: {}, exhaust_burn: {}, leader_fee: {}, num_shards: {}",
                total_tx_fee,
                exhaust_burn,
                leader_fee.fee,
                total_num_involved_shards
            );

            leader_fee
        }

        #[test]
        fn it_calculates_the_correct_leader_fee() {
            let fee = check_calculate_leader_fee(100, 5, 1);
            assert_eq!(fee.fee, 100);
            assert_eq!(fee.exhaust_burn, 5);

            let fee = check_calculate_leader_fee(100, 5, 2);
            assert_eq!(fee.fee, 50);
            assert_eq!(fee.exhaust_burn, 5);

            let fee = check_calculate_leader_fee(100, 5, 3);
            assert_eq!(fee.fee, 33);
            assert_eq!(fee.exhaust_burn, 6);

            let fee = check_calculate_leader_fee(98, 0, 3);
            assert_eq!(fee.fee, 32);
            assert_eq!(fee.exhaust_burn, 2);

            let fee = check_calculate_leader_fee(98, 4, 10);
            assert_eq!(fee.fee, 9);
            assert_eq!(fee.exhaust_burn, 12);

            let fee = check_calculate_leader_fee(19802, 990, 45);
            assert_eq!(fee.fee, 440);
            assert_eq!(fee.exhaust_burn, 992);

            let fee = check_calculate_leader_fee(311, 15, 45);
            assert_eq!(fee.fee, 6);
            assert_eq!(fee.exhaust_burn, 56);
        }

        #[test]
        fn simple_fuzz() {
            let mut total_fees = 0;
            let mut total_burnt = 0;
            let mut rng = rand::rng();
            for _ in 0..1_000_000 {
                let fee = rng.random_range(100..100000u64);
                let burn = fee / 20;
                let involved = rng.random_range(1..100u64);
                let leader_fee = check_calculate_leader_fee(fee, burn, involved);
                total_fees += leader_fee.fee * involved;
                total_burnt += leader_fee.exhaust_burn;
            }

            println!(
                "total fees: {}, total burnt: {}, {}%",
                total_fees,
                total_burnt,
                // Approaches 5% (the burn is 1/20 of the fee plus sub-shard-count dust)
                (total_burnt as f64 / total_fees as f64) * 100.0
            );
        }
    }
}
