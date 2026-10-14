//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    fmt::{self, Display, Formatter},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tari_template_lib::types::{ComponentAddress, Hash32, ResourceAddress, TemplateAddress};

use crate::{
    Epoch,
    events::Event,
    fees::{FEE_ESTIMATE_ALLOWANCE, FeeReceipt},
    logs::LogEntry,
    resource::Resource,
    substate::SubstateDiff,
    transaction_receipt::TransactionReceipt,
};

#[derive(Debug, Clone, Serialize, Deserialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ExecuteResult {
    /// The finalized result to commit. If the fee transaction succeeds but the transaction fails, this will be accept.
    #[n(0)]
    pub finalize: FinalizeResult,
    /// The time taken to execute the transaction (excluding finalization).
    #[cfg_attr(feature = "ts", ts(type = "{secs: number, nanos: number}"))]
    #[n(1)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    pub execution_time: Duration,
    /// The epoch during which the transaction was executed. This may be None if consensus aborted before being able to
    /// lock to an epoch.
    #[n(2)]
    pub execute_epoch: Option<Epoch>,
    /// Total WASM metering points consumed by the transaction across all calls, including failed/aborted execution.
    /// Metering is deterministic, so every validator computes the identical value for the same transaction and
    /// pledged state. Defaults to 0 when decoding executions persisted before this field existed.
    #[n(3)]
    #[cbor(default)]
    #[serde(default)]
    pub wasm_execution_points: u64,
    /// Total native-verification points (stealth transfers, confidential withdraws, burn claims) charged to the
    /// transaction, priced in WASM-point equivalents by [`NativeExecutionPoints`]. Deterministic for the same
    /// reason as [`Self::wasm_execution_points`] — the price is a pure function of the declared statement — so
    /// every validator computes the identical value. Defaults to 0 when decoding executions persisted before this
    /// field existed.
    ///
    /// [`NativeExecutionPoints`]: crate::limits::NativeExecutionPoints
    #[n(4)]
    #[cbor(default)]
    #[serde(default)]
    pub native_execution_points: u64,
}

impl ExecuteResult {
    /// Every metering point the transaction cost a validator to execute — WASM and native crypto alike. This, not
    /// the WASM figure alone, is what the per-block execution budget must be enforced against: native verification
    /// is real CPU time on every replica, and bounding only the WASM half leaves it ungoverned.
    pub fn total_execution_points(&self) -> u64 {
        self.wasm_execution_points.saturating_add(self.native_execution_points)
    }

    pub fn new_rejected(transaction_hash: Hash32, reason: RejectReason, execute_epoch: Option<Epoch>) -> Self {
        Self {
            finalize: FinalizeResult::new_rejected(transaction_hash, reason),
            execution_time: Duration::default(),
            execute_epoch,
            wasm_execution_points: 0,
            native_execution_points: 0,
        }
    }

    /// Returns the ExecuteResult if successful, or panic if the transaction was not successful.
    #[track_caller]
    pub fn unwrap_success(self) -> Self {
        self.expect_success();
        self
    }

    /// Returns the SubstateDiff if the transaction was successful, or panic if the transaction was not successful.
    #[track_caller]
    pub fn expect_success(&self) -> &SubstateDiff {
        let diff = self.expect_finalization_success();

        if let Some(reason) = self.finalize.any_reject() {
            panic!("Transaction failed: {}", reason);
        }

        diff
    }

    #[track_caller]
    pub fn expect_finalization_failure(&self) -> &RejectReason {
        match self.finalize.result {
            TransactionResult::Accept(_) => panic!("Expected transaction to fail but it succeeded"),
            TransactionResult::AcceptFeeRejectRest(_, ref reason) => reason,
            TransactionResult::Reject(ref reason) => reason,
        }
    }

    #[track_caller]
    pub fn expect_fee_accept_transaction_reject(&self) -> (&SubstateDiff, &RejectReason) {
        match self.finalize.result {
            TransactionResult::AcceptFeeRejectRest(ref diff, ref reason) => (diff, reason),
            _ => panic!(
                "Expected fee accept, transaction reject but got {:?}",
                self.finalize.result
            ),
        }
    }

    #[track_caller]
    pub fn expect_failure(&self) -> &RejectReason {
        if let Some(reason) = self.finalize.any_reject() {
            reason
        } else {
            panic!("Transaction succeeded but it was expected to fail");
        }
    }

    #[track_caller]
    pub fn expect_finalization_success(&self) -> &SubstateDiff {
        match self.finalize.result {
            TransactionResult::Accept(ref diff) => diff,
            TransactionResult::AcceptFeeRejectRest(ref diff, _) => diff,
            TransactionResult::Reject(ref reason) => panic!("Transaction failed: {}", reason),
        }
    }

    #[track_caller]
    pub fn expect_fees_paid_in_full(&self) -> &FeeReceipt {
        let receipt = &self.finalize.fee_receipt;
        assert!(receipt.is_paid_in_full(), "Fees not paid in full");
        receipt
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FinalizeResult {
    #[n(0)]
    pub transaction_hash: Hash32,
    #[n(1)]
    pub events: Vec<Event>,
    #[n(2)]
    pub logs: Vec<LogEntry>,
    // Index 3 is retired: it carried template return values, which payloads from older nodes may
    // still hold and decoding skips. It must not be reused.
    #[n(4)]
    pub result: TransactionResult,
    #[n(5)]
    pub fee_receipt: FeeReceipt,
    /// What committing the whole transaction was priced at, which is what the commit-or-reject
    /// decision was made against.
    ///
    /// Equal to `fee_receipt.total_fees_charged()` on a full commit. When only the fee intent
    /// commits, the charges are re-derived over the fee checkpoint alone and so fall below what was
    /// paid; this keeps the figure the main intent was rejected for — the one a resubmission has to
    /// clear. Execution metadata rather than settled state, so it stays out of the persisted
    /// receipt, whose every field is priced into the storage charge of each transaction.
    #[n(6)]
    #[serde(default)]
    pub total_fees_required: u64,
}

impl FinalizeResult {
    pub fn new(
        transaction_hash: Hash32,
        logs: Vec<LogEntry>,
        events: Vec<Event>,
        result: TransactionResult,
        fee_receipt: FeeReceipt,
    ) -> Self {
        Self {
            total_fees_required: fee_receipt.total_fees_charged(),
            transaction_hash,
            logs,
            events,
            result,
            fee_receipt,
        }
    }

    /// Records what committing the whole transaction was priced at, when that differs from what the
    /// receipt was ultimately charged.
    pub fn with_total_fees_required(mut self, amount: u64) -> Self {
        self.total_fees_required = amount.max(self.fee_receipt.total_fees_charged());
        self
    }

    /// What committing this transaction costs, with no estimate allowance on top.
    ///
    /// Falls back to what the receipt was charged. `total_fees_required` is additive on the wire, so
    /// a payload from a peer that predates it decodes as zero — reading it raw would price a
    /// transaction at nothing.
    pub fn charged_fees(&self) -> u64 {
        self.total_fees_required.max(self.fee_receipt.total_fees_charged())
    }

    /// The minimum `max_fee` a resubmission of this transaction has to carry: [`Self::charged_fees`]
    /// plus the allowance covering the metering drift a different `max_fee` causes.
    pub fn required_fees(&self) -> u64 {
        self.charged_fees().saturating_add(FEE_ESTIMATE_ALLOWANCE)
    }

    pub fn new_rejected(transaction_hash: Hash32, reason: RejectReason) -> Self {
        Self {
            transaction_hash,
            logs: vec![],
            events: vec![],
            result: TransactionResult::Reject(reason),
            fee_receipt: FeeReceipt::default(),
            total_fees_required: 0,
        }
    }

    /// Returns the substate diff if the fee and main intents were accepted, otherwise None.
    pub fn accept(&self) -> Option<&SubstateDiff> {
        self.result.accept()
    }

    /// Returns the accept diff if the transaction was accepted, otherwise None.
    /// Acceptance includes fee-only acceptance.
    pub fn any_accept(&self) -> Option<&SubstateDiff> {
        self.result.any_accept()
    }

    pub fn into_any_accept(self) -> Option<SubstateDiff> {
        match self.result {
            TransactionResult::Accept(diff) => Some(diff),
            TransactionResult::AcceptFeeRejectRest(diff, _) => Some(diff),
            TransactionResult::Reject(_) => None,
        }
    }

    pub fn fee_reject(&self) -> Option<&RejectReason> {
        self.result.fee_reject()
    }

    pub fn any_reject(&self) -> Option<&RejectReason> {
        self.result.any_reject()
    }

    pub fn reject(&self) -> Option<&RejectReason> {
        self.result.reject()
    }

    pub fn fee_accept_transaction_reject(&self) -> Option<(&SubstateDiff, &RejectReason)> {
        self.result.fee_accept_transaction_reject()
    }

    pub fn is_full_accept(&self) -> bool {
        matches!(self.result, TransactionResult::Accept(_))
    }

    pub fn is_accept(&self) -> bool {
        self.is_full_accept() || self.is_fee_only()
    }

    pub fn is_fee_only(&self) -> bool {
        matches!(self.result, TransactionResult::AcceptFeeRejectRest(_, _))
    }

    pub fn is_reject(&self) -> bool {
        matches!(self.result, TransactionResult::Reject(_))
    }

    pub fn get_transaction_receipt(&self) -> Option<&TransactionReceipt> {
        self.result.any_accept().and_then(|diff| {
            diff.up_iter()
                .find_map(|(_, s)| s.substate_value().as_transaction_receipt())
        })
    }

    pub fn get_components_by_template(&self, template_address: &TemplateAddress) -> Vec<ComponentAddress> {
        let mut components = Vec::new();
        if let Some(diff) = self.any_accept() {
            for (id, substate) in diff.up_iter() {
                if let Some(component) = substate.substate_value().as_component() &&
                    component.template_address() == template_address
                {
                    components.push(
                        id.as_component_address()
                            .expect("Substate value is a component but address is not a component address"),
                    );
                }
            }
        }
        components
    }

    pub fn created_resources(&self) -> impl Iterator<Item = (ResourceAddress, &Resource)> + '_ {
        self.any_accept()
            .into_iter()
            .flat_map(|diff| diff.up_iter())
            .filter_map(|(id, s)| id.as_resource_address().zip(s.substate_value().as_resource()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum TransactionResult {
    #[n(0)]
    Accept(#[n(0)] SubstateDiff),
    #[n(1)]
    AcceptFeeRejectRest(#[n(0)] SubstateDiff, #[n(1)] RejectReason),
    #[n(2)]
    Reject(#[n(0)] RejectReason),
}

impl TransactionResult {
    pub fn is_any_accept(&self) -> bool {
        matches!(self, Self::Accept(_) | Self::AcceptFeeRejectRest(_, _))
    }

    pub fn is_accept(&self) -> bool {
        matches!(self, Self::Accept(_))
    }

    pub fn is_reject(&self) -> bool {
        matches!(self, Self::Reject(_))
    }

    pub fn accept(&self) -> Option<&SubstateDiff> {
        match self {
            Self::Accept(substate_diff) => Some(substate_diff),
            _ => None,
        }
    }

    pub fn any_accept(&self) -> Option<&SubstateDiff> {
        match self {
            Self::Accept(substate_diff) => Some(substate_diff),
            Self::AcceptFeeRejectRest(substate_diff, _) => Some(substate_diff),
            Self::Reject(_) => None,
        }
    }

    pub fn into_any_accept(self) -> Option<SubstateDiff> {
        match self {
            Self::Accept(substate_diff) => Some(substate_diff),
            Self::AcceptFeeRejectRest(substate_diff, _) => Some(substate_diff),
            Self::Reject(_) => None,
        }
    }

    pub fn fee_reject(&self) -> Option<&RejectReason> {
        match self {
            Self::Accept(_) => None,
            Self::AcceptFeeRejectRest(_, _) => None,
            Self::Reject(reject_result) => Some(reject_result),
        }
    }

    pub fn fee_accept_transaction_reject(&self) -> Option<(&SubstateDiff, &RejectReason)> {
        match self {
            Self::AcceptFeeRejectRest(substate_diff, reject_result) => Some((substate_diff, reject_result)),
            _ => None,
        }
    }

    pub fn any_reject(&self) -> Option<&RejectReason> {
        match self {
            Self::Accept(_) => None,
            Self::AcceptFeeRejectRest(_, reject_result) => Some(reject_result),
            Self::Reject(reject_result) => Some(reject_result),
        }
    }

    pub fn reject(&self) -> Option<&RejectReason> {
        match self {
            Self::Reject(reject_result) => Some(reject_result),
            _ => None,
        }
    }

    pub fn expect(self, msg: &str) -> SubstateDiff {
        match self {
            Self::Accept(substate_diff) => substate_diff,
            Self::AcceptFeeRejectRest(substate_diff, _) => substate_diff,
            Self::Reject(reject_result) => {
                panic!("{}: {:?}", msg, reject_result);
            },
        }
    }
}

impl Display for TransactionResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept(diff) => write!(f, "Accept({} up, {} down)", diff.up_len(), diff.down_len()),
            Self::AcceptFeeRejectRest(diff, reason) => write!(
                f,
                "AcceptFee({} up, {} down), Reject {}",
                diff.up_len(),
                diff.down_len(),
                reason
            ),
            Self::Reject(reason) => write!(f, "Reject: {}", reason),
        }
    }
}

#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, minicbor::Encode, minicbor::Decode, minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum RejectReason {
    #[n(0)]
    ExecutionFailure {
        #[n(0)]
        code: ExecutionFailureCode,
        #[n(1)]
        message: String,
    },
    #[n(1)]
    SubstateNotFound(#[n(0)] String),
    #[n(2)]
    FailedToLockInputs(#[n(0)] String),
    #[n(3)]
    FailedToLockOutputs(#[n(0)] String),
    #[n(4)]
    ForeignPledgeInputConflict,
    #[n(5)]
    ForeignShardGroupDecidedToAbort {
        #[n(0)]
        start_shard: u32,
        #[n(1)]
        end_shard: u32,
        #[n(2)]
        abort_reason: AbortReason,
    },
    #[n(6)]
    InsufficientFeesPaid(#[n(0)] String),
    #[n(7)]
    FeePaymentInMainIntent,
    #[n(8)]
    Abort {
        #[n(0)]
        reason: AbortReason,
    },
}

impl Display for RejectReason {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::ExecutionFailure { code, message } => write!(f, "Execution failure ({code}): {message}"),
            Self::SubstateNotFound(msg) => write!(f, "Substates not found: {}", msg),
            Self::FailedToLockInputs(msg) => write!(f, "Failed to lock inputs: {}", msg),
            Self::FailedToLockOutputs(msg) => write!(f, "Failed to lock outputs: {}", msg),
            Self::ForeignPledgeInputConflict => {
                write!(f, "Transaction conflicts with an existing foreign pledge",)
            },
            Self::ForeignShardGroupDecidedToAbort {
                start_shard,
                end_shard,
                abort_reason,
            } => {
                write!(
                    f,
                    "Foreign shard group ({start_shard}-{end_shard}) decided to abort: {abort_reason}"
                )
            },
            Self::InsufficientFeesPaid(msg) => write!(f, "Insufficient fees paid: {}", msg),
            Self::FeePaymentInMainIntent => {
                write!(f, "Fee payment found in main intent instead of fee intent")
            },
            Self::Abort { reason } => write!(f, "Abnormal abort: {reason}"),
        }
    }
}

impl RejectReason {
    /// An execution failure whose cause has no [`ExecutionFailureCode`] yet.
    ///
    /// Engine code must not call this: the engine classifies through `failure_code`, and a caller that
    /// reaches for `Unclassified` by hand puts a failure beyond the reach of every consumer that branches
    /// on the code. It exists for test fixtures and for boundaries that only ever had a message.
    pub fn execution_failure_unclassified(message: impl Into<String>) -> Self {
        Self::ExecutionFailure {
            code: ExecutionFailureCode::Unclassified,
            message: message.into(),
        }
    }

    /// The failure code, for the one variant that carries one.
    pub fn execution_failure_code(&self) -> Option<ExecutionFailureCode> {
        match self {
            Self::ExecutionFailure { code, .. } => Some(*code),
            _ => None,
        }
    }
}

/// Why a transaction failed during execution, coarse enough for a consumer to act on.
///
/// The engine's internal error types carry far more detail than a consumer can branch on, and that detail
/// is a rendered string by the time it reaches [`RejectReason::ExecutionFailure`]. This enum is the part of
/// it an application can make a decision from: whether to retry, to raise the fee, to re-quote, to fix its
/// own call, or to tell the user their action was refused.
///
/// A code must be derivable from the error variant alone. The rendered message reaches consensus, so two
/// validators that disagreed about the code would already have disagreed about the message — but a code is
/// matched on rather than read, so a distinction that is invisible in prose becomes a fork here. See
/// `SubstateStoreError::SubstateNotFound`, where two conditions are deliberately collapsed because a node
/// with pruned history cannot tell them apart.
///
/// The variant names are a public contract — hosts branch on them, and the discriminants are encoded to
/// consensus. **Never rename or renumber a variant; only add.**
#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Deserialize,
    Serialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ExecutionFailureCode {
    /// The fault lies in the template: it returned an error, panicked, trapped, or misused the engine.
    /// A caller cannot fix its request to make the call succeed, and a retry will fail the same way.
    ///
    /// A panic carries the template author's own message, which is the only failure text in this enum
    /// written to be read by an end user; the rest carry engine wording.
    #[n(0)]
    TemplateError,
    /// An access rule, owner rule or auth hook refused the action.
    #[n(1)]
    AccessDenied,
    /// A transaction-level assertion did not hold, e.g. a bucket did not contain the resource or amount
    /// the manifest asserted. The usual cause is a quote that moved between building and executing.
    #[n(2)]
    AssertionFailed,
    /// A vault or bucket did not hold enough of a resource, or a supply would over/underflow.
    #[n(3)]
    InsufficientFunds,
    /// Execution ran past what the fee paid for. Paying more clears it, which is the line between this and
    /// [`Self::LimitExceeded`]. Distinct from [`RejectReason::InsufficientFeesPaid`], which is the
    /// settled-up shortfall rather than a mid-execution stop.
    #[n(4)]
    OutOfCompute,
    /// A fixed engine ceiling was reached: compute caps, substate/event/log size, event and log counts, call
    /// depth, generated outputs or entities, template size. No fee raises any of them, so the work has to be
    /// made smaller or split up.
    #[n(5)]
    LimitExceeded,
    /// The call was malformed — wrong argument count, undecodable argument, unknown function, an amount
    /// that is not valid for the operation. A client-side bug rather than anything the user did.
    #[n(6)]
    InvalidArgument,
    /// Something named by the transaction did not exist at execution time: a template, component, resource,
    /// vault, non-fungible or workspace item. Distinct from [`RejectReason::SubstateNotFound`], which is a
    /// declared *input* that could not be resolved.
    #[n(7)]
    NotFound,
    /// The transaction left engine-owned values unaccounted for — buckets, proofs, address allocations or
    /// substates that were neither consumed nor returned. A manifest bug.
    #[n(8)]
    DanglingResources,
    /// The resource itself refused: a frozen vault, a burnt non-fungible, a recall or freeze aimed at the
    /// wrong resource. The user's own authority is not the problem, so it is not [`Self::AccessDenied`].
    #[n(9)]
    ResourceRestricted,
    /// A cryptographic proof did not verify, or a spend script rejected the spend: range proofs, balance
    /// proofs, confidential and stealth proofs, burn-claim proofs.
    #[n(10)]
    InvalidProof,
    /// The engine reached a state it holds to be impossible. Always a bug in the engine, never in the
    /// transaction; a consumer should surface it as such rather than asking the user to retry.
    #[n(11)]
    EngineInvariant,
    /// The transaction is well-formed, but it depends on chain state this epoch does not have. The same
    /// transaction, unchanged, can succeed once the state arrives, so a consumer schedules a retry rather
    /// than reporting a failure. Nothing the caller sends differently would help now.
    #[n(13)]
    NotYetValid,
    /// No code has been assigned to this failure yet. Consumers fall back to the message.
    ///
    /// Not a resting place: the engine emits a `warn!` whenever it produces one, so the tail can be
    /// prioritised from real traffic instead of guessed at.
    #[n(12)]
    Unclassified,
}

impl Display for ExecutionFailureCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Deserialize,
    Serialize,
    borsh::BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum AbortReason {
    #[n(0)]
    ForeignPledgeInputConflict,
    #[n(1)]
    LockInputsFailed,
    #[n(2)]
    LockOutputsFailed,
    #[n(3)]
    LockInputsOutputsFailed,
    #[n(4)]
    ExecutionFailure,
    #[n(5)]
    OneOrMoreInputsNotFound,
    #[n(6)]
    InsufficientFeesPaid,
    #[n(7)]
    FeePaymentInMainIntent,
    #[n(8)]
    EpochExpired,
    /// The transaction's `max_epoch` is further ahead of the epoch it was pinned to than the network
    /// permits. Evaluated against the pinned epoch, so every shard group reaches the same verdict.
    #[n(9)]
    ValidityWindowTooLong,
    /// The transaction's inputs held by one shard group are more than a block may pledge to the other shard
    /// groups involved, so no block could deliver them.
    #[n(10)]
    InputPledgeTooLarge,
}

impl Display for AbortReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<&RejectReason> for AbortReason {
    fn from(reject_reason: &RejectReason) -> Self {
        match reject_reason {
            RejectReason::Abort { reason } => *reason,
            RejectReason::ExecutionFailure { .. } => Self::ExecutionFailure,
            RejectReason::SubstateNotFound(_) => Self::OneOrMoreInputsNotFound,
            RejectReason::ForeignPledgeInputConflict => Self::ForeignPledgeInputConflict,
            RejectReason::FailedToLockInputs(_) => Self::LockInputsFailed,
            RejectReason::FailedToLockOutputs(_) => Self::LockOutputsFailed,
            RejectReason::ForeignShardGroupDecidedToAbort { abort_reason, .. } => *abort_reason,
            RejectReason::InsufficientFeesPaid(_) => Self::InsufficientFeesPaid,
            RejectReason::FeePaymentInMainIntent => Self::FeePaymentInMainIntent,
        }
    }
}

#[cfg(test)]
mod tests {
    use tari_template_lib::types::Hash32;

    use super::*;
    use crate::fees::{FeeBreakdown, FeeSource};

    fn result_charged(amount: u64) -> FinalizeResult {
        let mut breakdown = FeeBreakdown::default();
        breakdown.add(FeeSource::Storage, amount);
        FinalizeResult::new(
            Hash32::from_array([0u8; Hash32::LENGTH]),
            vec![],
            vec![],
            TransactionResult::Reject(RejectReason::execution_failure_unclassified("failed")),
            FeeReceipt::builder().with_cost_breakdown(breakdown).build(),
        )
    }

    /// `total_fees_required` is additive on the wire, so a payload from a peer that predates it
    /// decodes as zero. An estimate built from it must still clear what the transaction cost.
    #[test]
    fn a_missing_required_total_falls_back_to_what_was_charged() {
        let mut result = result_charged(5_000);
        result.total_fees_required = 0;
        assert!(result.required_fees() > 5_000);
        // The same fallback, for a caller that wants the cost without the allowance on top: reading
        // the raw field would price this transaction at nothing.
        assert_eq!(result.charged_fees(), 5_000);
    }

    /// When the two differ, the gating figure is the one a resubmission has to clear.
    #[test]
    fn required_fees_reports_the_larger_of_the_two() {
        let result = result_charged(400).with_total_fees_required(9_000);
        assert!(result.required_fees() > 9_000);
    }

    /// A result from an older node carries its template return values at index 3.
    #[test]
    fn a_payload_carrying_return_values_still_decodes() {
        let result = result_charged(400);
        let return_values = tari_bor::Value::Array(vec![tari_bor::Value::Array(vec![
            tari_bor::Value::Text("returned".to_string()),
            tari_bor::Value::Null,
        ])]);

        let tari_bor::Value::Array(mut fields) = tari_bor::to_value(&result).unwrap() else {
            panic!("FinalizeResult is not encoded as an array");
        };
        fields[3] = return_values;
        let decoded: FinalizeResult =
            tari_bor::decode_exact(&tari_bor::encode(&tari_bor::Value::Array(fields)).unwrap()).unwrap();
        assert_eq!(decoded.fee_receipt.total_fees_charged(), 400);

        let mut json = serde_json::to_value(&result).unwrap();
        json["execution_results"] = serde_json::json!([{ "indexed": null, "return_type": "Unit" }]);
        let decoded: FinalizeResult = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.fee_receipt.total_fees_charged(), 400);
    }
}
