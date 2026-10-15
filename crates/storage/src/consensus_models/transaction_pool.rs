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
use rand::{Rng, RngExt, seq::SliceRandom};
use serde::{Deserialize, Serialize};
use tari_consensus_types::{BlockId, Decision, LeafBlock};
use tari_engine_types::{SubstateVersion, substate::SubstateId};
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
    PendingChain,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{
        BlockTransactionExecution,
        Evidence,
        FOREIGN_PROPOSAL_EPOCH_WINDOW,
        LeaderFee,
        LocalOnlyAtom,
        LockedEpoch,
        MultiShardAtom,
        TransactionExecution,
        TransactionRecord,
        calculate_leader_fee,
    },
};

const LOG_TARGET: &str = "tari::ootle::storage::transaction_pool";

/// Epochs after a transaction's `max_epoch` that a shard group holding it in `LocalPrepared` keeps waiting for the
/// other involved shard groups. A foreign proposal made in `max_epoch` is accepted until the local epoch moves past
/// [`FOREIGN_PROPOSAL_EPOCH_WINDOW`] epochs after it.
pub const LOCAL_PREPARED_GRACE_EPOCHS: u64 = FOREIGN_PROPOSAL_EPOCH_WINDOW;

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

    #[expect(clippy::too_many_arguments)]
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

    /// Fetch transactions ready to propose in a block in `epoch`, bounded by a weight budget and a hard command
    /// count cap. See [`select_proposal_batch`] for how records are chosen.
    ///
    /// The candidates are the ready records that follow a random transaction id, wrapping around. At most
    /// `max_candidates` ready records are sampled and at most `max_reads` pool records are read, ready or not, so the
    /// cost of a call is bounded however large the pool is. When the pool holds more than that window, each call
    /// samples a different, uniformly placed part of it.
    #[expect(clippy::too_many_arguments)]
    pub fn get_batch_for_next_block(
        &self,
        tx: &impl StateStoreReadTransaction,
        weight_budget: u64,
        max_count: usize,
        max_candidates: usize,
        max_reads: usize,
        heavy_turn: bool,
        block_id: &BlockId,
        epoch: Epoch,
    ) -> Result<Vec<TransactionPoolRecord>, TransactionPoolError> {
        if weight_budget == 0 || max_count == 0 || max_candidates == 0 || max_reads == 0 {
            return Ok(Vec::new());
        }
        let mut rng = rand::rng();
        let start_id = TransactionId::new(rng.random());
        let mut sample = ProposalSample::new(weight_budget, max_count, heavy_turn);
        tx.transaction_pool_for_each_ready(block_id, epoch, &start_id, max_candidates, max_reads, |rec| {
            sample.offer(rec, &mut rng)
        })?;
        Ok(sample.into_batch(&mut rng))
    }

    pub fn has_ready_or_pending_transaction_updates(
        &self,
        tx: &impl StateStoreReadTransaction,
        block_id: &BlockId,
        epoch: Epoch,
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

        // A transaction is worth proposing for if it is ready, or if it has been locked but not yet finalized and
        // needs further proposals until it is.
        let mut reason = None;
        let found = tx.transaction_pool_any(true, |rec| {
            if rec.is_ready_to_propose_in(epoch) {
                reason = Some("ready to propose");
                return true;
            }
            let needs_finalizing = matches!(
                rec.current_stage(),
                TransactionPoolStage::LocalOnly |
                    TransactionPoolStage::AllAccepted |
                    TransactionPoolStage::SomeAccepted
            );
            if needs_finalizing {
                reason = Some("needs to be finalized");
            }
            needs_finalizing
        })?;

        if let Some(reason) = reason {
            debug!(
                target: LOG_TARGET,
                "has_ready_or_pending_transaction_updates: a transaction {reason}",
            );
        } else {
            debug!(
                target: LOG_TARGET,
                "has_ready_or_pending_transaction_updates: No ready transactions or transactions that need to be finalized",
            );
        }

        Ok(found)
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

/// Choose the ready records to propose in the next block from `records`. See [`ProposalSample`].
pub fn select_proposal_batch<R: Rng + ?Sized>(
    records: impl IntoIterator<Item = TransactionPoolRecord>,
    weight_budget: u64,
    max_count: usize,
    heavy_turn: bool,
    rng: &mut R,
) -> Vec<TransactionPoolRecord> {
    let mut sample = ProposalSample::new(weight_budget, max_count, heavy_turn);
    for rec in records {
        sample.offer(rec, rng);
    }
    sample.into_batch(rng)
}

/// A bounded uniform random sample of the ready records offered to it, from which the next block's batch is
/// chosen.
///
/// Every offered record has the same chance of being kept whatever its position, so a submitter cannot choose
/// or predict where their transaction sorts, and memory stays bounded by `max_count` however many records are
/// ready. Records that fit `weight_budget` share a reservoir of `max_count`, the most a batch can hold. Records
/// that only fit alone share a single slot.
///
/// When packing, records are considered in random order. Each one whose
/// [`TransactionPoolRecord::proposal_weight`] fits the remaining `weight_budget` is taken; one that does not fit
/// is skipped so the lighter records behind it still pack. Packing stops at `max_count` records.
///
/// A heavy record weighs more than half the `weight_budget`, so a random draw rarely reaches it before lighter
/// records fill the budget. On a `heavy_turn` one heavy record, if any is ready, is taken first. A record heavier
/// than the whole `weight_budget` is oversized and can only be proposed alone: it is chosen on a heavy turn, or
/// when no ready record fits the budget. The caller sets `heavy_turn` on a fixed fraction of blocks, so heavy
/// transactions still commit while the pool never drains, and oversized ones cannot claim more than that fraction
/// of blocks from transactions that fit.
///
/// The batch is sorted by transaction id. The proposer prepares transactions in batch order and replicas process
/// a block's commands in transaction-id order, so two transactions contending for a lock must reach it in the
/// same order on both sides. A caller that stops preparing part way through the batch (at a deadline or execution
/// budget) therefore defers the highest ids; which records enter the batch is random, the order within it is not.
pub struct ProposalSample {
    weight_budget: u64,
    max_count: usize,
    heavy_turn: bool,
    fitting: Vec<TransactionPoolRecord>,
    num_fitting_offered: u64,
    /// Holds an oversized record, or on a heavy turn any heavy record. At most one record over half the budget
    /// fits in a batch, so one slot is all a batch can use.
    alone: Option<TransactionPoolRecord>,
    num_alone_offered: u64,
}

impl ProposalSample {
    pub fn new(weight_budget: u64, max_count: usize, heavy_turn: bool) -> Self {
        Self {
            weight_budget,
            max_count,
            heavy_turn,
            fitting: Vec::new(),
            num_fitting_offered: 0,
            alone: None,
            num_alone_offered: 0,
        }
    }

    pub fn offer<R: Rng + ?Sized>(&mut self, rec: TransactionPoolRecord, rng: &mut R) {
        if self.weight_budget == 0 || self.max_count == 0 {
            return;
        }
        let weight = rec.proposal_weight();
        let is_oversized = weight > self.weight_budget;
        let is_heavy = weight > self.weight_budget / 2;
        if is_oversized || (self.heavy_turn && is_heavy) {
            self.num_alone_offered += 1;
            if rng.random_range(0..self.num_alone_offered) == 0 {
                self.alone = Some(rec);
            }
            return;
        }

        self.num_fitting_offered += 1;
        if self.fitting.len() < self.max_count {
            self.fitting.push(rec);
            return;
        }
        let slot = rng.random_range(0..self.num_fitting_offered);
        if let Some(kept) = usize::try_from(slot).ok().and_then(|slot| self.fitting.get_mut(slot)) {
            *kept = rec;
        }
    }

    pub fn into_batch<R: Rng + ?Sized>(self, rng: &mut R) -> Vec<TransactionPoolRecord> {
        let Self {
            weight_budget,
            max_count,
            heavy_turn,
            mut fitting,
            mut alone,
            ..
        } = self;

        let mut batch = Vec::new();
        let mut accumulated_weight = 0u64;
        if heavy_turn && let Some(rec) = alone.take() {
            if rec.proposal_weight() > weight_budget {
                return vec![rec];
            }
            accumulated_weight = rec.proposal_weight();
            batch.push(rec);
        }

        fitting.shuffle(rng);
        for rec in fitting {
            if batch.len() >= max_count || accumulated_weight == weight_budget {
                break;
            }
            let next_weight = accumulated_weight.saturating_add(rec.proposal_weight());
            if next_weight > weight_budget {
                continue;
            }
            accumulated_weight = next_weight;
            batch.push(rec);
        }

        if batch.is_empty() {
            batch.extend(alone);
        }
        batch.sort_unstable_by(|a, b| a.id().cmp(b.id()));
        batch
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

    #[expect(clippy::too_many_arguments)]
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

    /// True if, in `epoch`, this transaction has waited in `LocalPrepared` with a COMMIT decision for longer than its
    /// validity window plus [`LOCAL_PREPARED_GRACE_EPOCHS`], and the local shard group must abort it.
    ///
    /// CONSENSUS RULE: the leader proposes, and every replica requires, `LocalAccept(ABORT)` for such a transaction.
    /// Aborting here is safe because the local shard group has not sent its LocalAccept yet, and no shard group can
    /// finalize COMMIT without it. A transaction in `LocalAccepted` is never expired by this rule: a foreign shard
    /// group may already have finalized COMMIT on the strength of the local LocalAccept.
    pub fn is_local_prepared_expired_at(&self, epoch: Epoch) -> bool {
        self.current_stage().is_local_prepared() &&
            self.current_decision().is_commit() &&
            epoch.as_u64() > self.max_epoch.as_u64().saturating_add(LOCAL_PREPARED_GRACE_EPOCHS)
    }

    /// True if a block in `epoch` may propose this transaction's next command.
    pub fn is_ready_to_propose_in(&self, epoch: Epoch) -> bool {
        self.is_ready || self.is_local_prepared_expired_at(epoch)
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

    pub fn get_current_multi_shard_atom(&self) -> MultiShardAtom {
        MultiShardAtom {
            id: self.transaction_id,
            decision: self.current_decision(),
            evidence: self.evidence.clone(),
            transaction_fee: self.transaction_fee,
            leader_fee: self.leader_fee().cloned(),
        }
    }

    pub fn get_local_only_atom(&self) -> LocalOnlyAtom {
        LocalOnlyAtom {
            id: self.transaction_id,
            decision: self.current_decision(),
            transaction_fee: self.transaction_fee,
            leader_fee: self.leader_fee().cloned(),
        }
    }

    pub fn get_local_multi_shard_atom(&self) -> MultiShardAtom {
        MultiShardAtom {
            id: self.transaction_id,
            decision: self.current_local_decision(),
            evidence: self.evidence.clone(),
            transaction_fee: self.transaction_fee,
            leader_fee: self.leader_fee().cloned(),
        }
    }

    pub fn into_current_multi_shard_atom(self) -> MultiShardAtom {
        MultiShardAtom {
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
        // A transaction is only finalised once the shard groups involved in it have reached the agreement the
        // finalising command records: every one accepted for AllAccepted, at least one prepared for SomeAccepted.
        if self.current_stage().is_local_accepted() &&
            next_stage.is_finalising() &&
            !self.is_ready_for_pending_stage(local_shard_group)
        {
            return Err(TransactionPoolError::NotReadyToLeaveStage {
                transaction_id: self.transaction_id,
                from: self.current_stage(),
                to: next_stage,
            });
        }
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

    /// Returns the pool record with the latest update recorded by the chain up to its leaf applied.
    pub fn get<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        chain: &PendingChain,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, TransactionPoolError> {
        let rec = tx.transaction_pool_get_for_blocks_in_chain(chain, transaction_id)?;
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
        TObj: IntoIterator<Item = (&'a SubstateId, Option<(SubstateVersion, SubstateLockType)>)>,
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
    #[error(
        "Transaction {transaction_id} cannot move from {from} to {to} before the shard groups involved in it are ready"
    )]
    NotReadyToLeaveStage {
        transaction_id: TransactionId,
        from: TransactionPoolStage,
        to: TransactionPoolStage,
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

        pub(super) fn record_with_weight_and_stage(weight: u64, stage: TransactionPoolStage) -> TransactionPoolRecord {
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

    mod is_local_prepared_expired_at {
        use tari_engine_types::commit_result::AbortReason;

        use super::{proposal_weight::record_with_weight_and_stage, *};

        #[test]
        fn it_expires_after_the_grace_epoch() {
            let mut rec = record_with_weight_and_stage(1, TransactionPoolStage::LocalPrepared);
            rec.is_ready = false;
            assert_eq!(rec.max_epoch(), Epoch(1));

            assert!(!rec.is_local_prepared_expired_at(Epoch(1)));
            assert!(!rec.is_local_prepared_expired_at(Epoch(2)));
            assert!(!rec.is_ready_to_propose_in(Epoch(2)));
            assert!(rec.is_local_prepared_expired_at(Epoch(3)));
            assert!(rec.is_ready_to_propose_in(Epoch(3)));
        }

        #[test]
        fn it_never_expires_a_transaction_that_has_sent_its_local_accept() {
            for stage in [
                TransactionPoolStage::New,
                TransactionPoolStage::LocalAccepted,
                TransactionPoolStage::AllAccepted,
                TransactionPoolStage::SomeAccepted,
                TransactionPoolStage::LocalOnly,
            ] {
                let rec = record_with_weight_and_stage(1, stage);
                assert!(!rec.is_local_prepared_expired_at(Epoch(100)), "{stage} must not expire");
            }
        }

        #[test]
        fn it_does_not_expire_a_transaction_already_aborting() {
            let mut rec = record_with_weight_and_stage(1, TransactionPoolStage::LocalPrepared);
            rec.set_local_decision(Decision::Abort(AbortReason::ExecutionFailure));
            assert!(!rec.is_local_prepared_expired_at(Epoch(100)));
        }
    }

    mod select_proposal_batch {
        use std::collections::HashSet;

        use rand::{SeedableRng, rngs::StdRng};

        use super::{proposal_weight::record_with_weight_and_stage, *};
        use crate::consensus_models::{ProposalSample, select_proposal_batch};

        const BUDGET: u64 = 24_000;

        /// New-stage records with the given weights, each id derived from its index.
        fn records(weights: &[u64]) -> Vec<TransactionPoolRecord> {
            weights
                .iter()
                .enumerate()
                .map(|(i, weight)| {
                    let mut rec = record_with_weight_and_stage(*weight, TransactionPoolStage::New);
                    rec.transaction_id = TransactionId::new([u8::try_from(i).unwrap(); 32]);
                    rec
                })
                .collect()
        }

        fn weights_with_lights(heavy: u64) -> Vec<u64> {
            let mut weights = vec![heavy];
            weights.extend([150; 200]);
            weights
        }

        fn select(weights: &[u64], max_count: usize, heavy_turn: bool, seed: u64) -> Vec<TransactionPoolRecord> {
            select_proposal_batch(
                records(weights),
                BUDGET,
                max_count,
                heavy_turn,
                &mut StdRng::seed_from_u64(seed),
            )
        }

        fn contains_heavy(batch: &[TransactionPoolRecord]) -> bool {
            batch.iter().any(|rec| *rec.id() == TransactionId::new([0; 32]))
        }

        #[test]
        fn a_record_that_overflows_the_budget_is_skipped() {
            for seed in 0..20 {
                let batch = select(&[15_000, 15_000, 100], 1000, false, seed);
                let weights = batch.iter().map(|rec| rec.proposal_weight()).collect::<HashSet<_>>();
                assert_eq!(weights, HashSet::from([15_000, 100]));
            }
        }

        #[test]
        fn an_oversized_record_is_proposed_alone_when_nothing_fits() {
            let batch = select(&[25_000, 25_000], 1000, false, 0);
            assert_eq!(batch.len(), 1);
        }

        #[test]
        fn an_oversized_record_waits_for_a_heavy_turn_while_records_fit() {
            for seed in 0..20 {
                let batch = select(&weights_with_lights(BUDGET + 1), 1000, false, seed);
                assert!(!contains_heavy(&batch));
                assert_eq!(batch.len(), 160);

                let batch = select(&weights_with_lights(BUDGET + 1), 1000, true, seed);
                assert_eq!(batch.len(), 1);
                assert!(contains_heavy(&batch));
            }
        }

        #[test]
        fn a_heavy_turn_packs_a_heavy_record_first_and_fills_the_rest() {
            for seed in 0..20 {
                let batch = select(&weights_with_lights(20_000), 1000, true, seed);
                assert!(contains_heavy(&batch));
                let total = batch.iter().map(|rec| rec.proposal_weight()).sum::<u64>();
                assert_eq!(total, 20_000 + 26 * 150);
            }
        }

        #[test]
        fn it_respects_the_hard_count_cap() {
            let batch = select(&[1, 1, 1, 1], 2, true, 0);
            assert_eq!(batch.len(), 2);
        }

        #[test]
        fn the_sample_is_bounded_by_the_count_cap() {
            let mut rng = StdRng::seed_from_u64(0);
            let mut sample = ProposalSample::new(BUDGET, 10, false);
            for rec in records(&[100; 250]) {
                sample.offer(rec, &mut rng);
            }
            assert_eq!(sample.fitting.len(), 10);
            assert_eq!(sample.into_batch(&mut rng).len(), 10);
        }

        #[test]
        fn a_record_is_sampled_whatever_its_position() {
            let last = TransactionId::new([249; 32]);
            let packed = (0..200)
                .filter(|seed| {
                    select(&[100; 250], 10, false, *seed)
                        .iter()
                        .any(|rec| *rec.id() == last)
                })
                .count();
            // Kept with probability 10/250, so about 8 of 200 draws.
            assert!((1..=30).contains(&packed), "packed {packed} times");
        }

        #[test]
        fn the_batch_is_in_transaction_id_order() {
            for seed in 0..20 {
                let batch = select(&[100; 50], 1000, false, seed);
                assert_eq!(batch.len(), 50);
                assert!(batch.is_sorted_by_key(|rec| *rec.id()));
            }
        }
    }

    mod leaving_local_accepted {
        use tari_consensus_types::PcId;
        use tari_engine_types::commit_result::AbortReason;
        use tari_template_lib_types::ComponentAddress;

        use super::*;

        fn input_group() -> ShardGroup {
            ShardGroup::new(0, 127)
        }

        fn local_group() -> ShardGroup {
            ShardGroup::new(128, 255)
        }

        /// A record at LocalAccepted with an input and an output shard group, neither of which is justified.
        fn local_accepted_record() -> TransactionPoolRecord {
            let mut evidence = Evidence::empty();
            evidence
                .add_shard_group(input_group())
                .insert_unpledged_input(ComponentAddress::from_array([1; 32]).into());
            evidence
                .add_shard_group(local_group())
                .insert_output(ComponentAddress::from_array([2; 32]).into(), SubstateVersion::ZERO);

            TransactionPoolRecord {
                transaction_id: TransactionId::new([0; 32]),
                original_decision: Decision::Commit,
                evidence,
                transaction_fee: 0,
                leader_fee: None,
                stage: TransactionPoolStage::LocalAccepted,
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
                exhaust_burn: 0,
            }
        }

        #[test]
        fn all_accepted_requires_every_shard_group_to_have_accepted() {
            let mut rec = local_accepted_record();
            rec.evidence_mut()
                .get_mut(&input_group())
                .unwrap()
                .set_prepare_qc(PcId::zero());
            rec.evidence_mut()
                .get_mut(&local_group())
                .unwrap()
                .set_accept_qc(PcId::zero());
            let err = rec
                .set_next_stage_and_readiness(TransactionPoolStage::AllAccepted, local_group())
                .unwrap_err();
            assert!(
                matches!(err, TransactionPoolError::NotReadyToLeaveStage { .. }),
                "unexpected error: {err}"
            );
            assert_eq!(rec.current_stage(), TransactionPoolStage::LocalAccepted);

            rec.evidence_mut()
                .get_mut(&input_group())
                .unwrap()
                .set_accept_qc(PcId::zero());
            rec.set_next_stage_and_readiness(TransactionPoolStage::AllAccepted, local_group())
                .unwrap();
            assert_eq!(rec.current_stage(), TransactionPoolStage::AllAccepted);
        }

        #[test]
        fn some_accepted_requires_a_shard_group_to_have_prepared() {
            let mut rec = local_accepted_record();
            rec.set_local_decision(Decision::Abort(AbortReason::ExecutionFailure));
            let err = rec
                .set_next_stage_and_readiness(TransactionPoolStage::SomeAccepted, local_group())
                .unwrap_err();
            assert!(
                matches!(err, TransactionPoolError::NotReadyToLeaveStage { .. }),
                "unexpected error: {err}"
            );

            rec.evidence_mut()
                .get_mut(&local_group())
                .unwrap()
                .set_accept_qc(PcId::zero());
            rec.set_next_stage_and_readiness(TransactionPoolStage::SomeAccepted, local_group())
                .unwrap();
            assert_eq!(rec.current_stage(), TransactionPoolStage::SomeAccepted);
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
