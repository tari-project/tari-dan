//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Boundary result records — the slice needed to type a submit acknowledgement.
//!
//! A self-contained boundary outcome ([`TransactionOutcome`]: `Commit` / `OnlyFeeCommit` /
//! `Reject`). The internal [`tari_engine_types::commit_result::RejectReason`] is flattened to a
//! boundary-friendly `{ code, message }` so hosts get a stable machine code plus a human-readable
//! message.

use serde::{Deserialize, Serialize};
use tari_engine_types::commit_result::{AbortReason, RejectReason as InternalRejectReason};

use crate::types::bytes::TransactionIdBytes;

/// The stable, SCREAMING_SNAKE code for a canonical [`AbortReason`] variant.
///
/// This is the **stable code set** for `AbortReason`. The strings are part of the public contract:
/// hosts branch on them, so **never rename a code; only add**. Covers all 9 variants in
/// `tari_engine_types::commit_result::AbortReason`.
pub fn abort_code(reason: &AbortReason) -> &'static str {
    match reason {
        AbortReason::ForeignPledgeInputConflict => "FOREIGN_PLEDGE_INPUT_CONFLICT",
        AbortReason::LockInputsFailed => "LOCK_INPUTS_FAILED",
        AbortReason::LockOutputsFailed => "LOCK_OUTPUTS_FAILED",
        AbortReason::LockInputsOutputsFailed => "LOCK_INPUTS_OUTPUTS_FAILED",
        AbortReason::ExecutionFailure => "EXECUTION_FAILURE",
        AbortReason::OneOrMoreInputsNotFound => "ONE_OR_MORE_INPUTS_NOT_FOUND",
        AbortReason::InsufficientFeesPaid => "INSUFFICIENT_FEES_PAID",
        AbortReason::FeePaymentInMainIntent => "FEE_PAYMENT_IN_MAIN_INTENT",
        AbortReason::EpochExpired => "EPOCH_EXPIRED",
        AbortReason::ValidityWindowTooLong => "VALIDITY_WINDOW_TOO_LONG",
    }
}

/// A boundary reject reason: a stable `code` plus the rendered `message`.
///
/// `code` is derived from the internal [`InternalRejectReason`] variant and is stable; hosts may
/// branch on it. `message` is the internal `Display` rendering. `abort_code` carries the canonical
/// [`AbortReason`] sub-code (via [`abort_code`]) when the reject is an abort — so hosts branch on, e.g.,
/// `EPOCH_EXPIRED` **without** parsing the human message. It is `None` for non-abort variants and is
/// omitted from JSON when absent (an additive field — existing `code` strings are unchanged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectReason {
    /// Stable variant code, e.g. `EXECUTION_FAILURE`.
    pub code: String,
    /// The canonical [`AbortReason`] sub-code (e.g. `EPOCH_EXPIRED`) when this is an abort; else `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abort_code: Option<String>,
    /// Human-readable detail (the internal `Display` output).
    pub message: String,
}

impl RejectReason {
    /// The stable code for an internal reject-reason variant.
    fn code_of(reason: &InternalRejectReason) -> &'static str {
        match reason {
            InternalRejectReason::ExecutionFailure(_) => "EXECUTION_FAILURE",
            InternalRejectReason::SubstateNotFound(_) => "SUBSTATE_NOT_FOUND",
            InternalRejectReason::FailedToLockInputs(_) => "FAILED_TO_LOCK_INPUTS",
            InternalRejectReason::FailedToLockOutputs(_) => "FAILED_TO_LOCK_OUTPUTS",
            InternalRejectReason::ForeignPledgeInputConflict => "FOREIGN_PLEDGE_INPUT_CONFLICT",
            InternalRejectReason::ForeignShardGroupDecidedToAbort { .. } => "FOREIGN_SHARD_GROUP_DECIDED_TO_ABORT",
            InternalRejectReason::InsufficientFeesPaid(_) => "INSUFFICIENT_FEES_PAID",
            InternalRejectReason::FeePaymentInMainIntent => "FEE_PAYMENT_IN_MAIN_INTENT",
            InternalRejectReason::Abort { .. } => "ABORT",
        }
    }

    /// The canonical abort sub-code for the variants that carry an [`AbortReason`].
    ///
    /// Only the `Abort` and `ForeignShardGroupDecidedToAbort` variants embed an `AbortReason`; every
    /// other variant returns `None` (its top-level `code` already fully describes it).
    fn abort_code_of(reason: &InternalRejectReason) -> Option<&'static str> {
        match reason {
            InternalRejectReason::Abort { reason } => Some(abort_code(reason)),
            InternalRejectReason::ForeignShardGroupDecidedToAbort { abort_reason, .. } => {
                Some(abort_code(abort_reason))
            },
            _ => None,
        }
    }

    /// Builds the boundary form from the internal reject reason.
    pub fn from_internal(reason: &InternalRejectReason) -> Self {
        Self {
            code: Self::code_of(reason).to_string(),
            abort_code: Self::abort_code_of(reason).map(str::to_string),
            message: reason.to_string(),
        }
    }

    /// Builds a boundary reject from a free-form **abort** message (e.g. an indexer `Finalized` result
    /// whose `abort_details` is set but which carries no `execution_result`). The stable `code` is
    /// `ABORT` (matching the engine `Abort` variant) and there is no structured abort sub-code.
    pub fn from_abort_details(message: impl Into<String>) -> Self {
        Self {
            code: "ABORT".to_string(),
            abort_code: None,
            message: message.into(),
        }
    }

    /// Builds a boundary reject for a **mempool** rejection — the indexer `Rejected{details}` case,
    /// where the transaction failed validation and was never sequenced by the network (so there is no
    /// engine execution at all). The stable `code` is `MEMPOOL_REJECTED`, distinct from the sequenced
    /// `ABORT` so hosts can tell "rejected before sequencing" from "sequenced but aborted".
    pub fn from_mempool_details(message: impl Into<String>) -> Self {
        Self {
            code: "MEMPOOL_REJECTED".to_string(),
            abort_code: None,
            message: message.into(),
        }
    }
}

/// The outcome of a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionOutcome {
    /// The transaction was fully committed.
    Commit,
    /// Only the fee intent was committed; the main transaction was not.
    OnlyFeeCommit(RejectReason),
    /// The transaction was rejected.
    Reject(RejectReason),
}

impl TransactionOutcome {
    /// True if the transaction was fully committed.
    pub fn is_commit(&self) -> bool {
        matches!(self, TransactionOutcome::Commit)
    }

    /// The reject reason, if any (set for both `OnlyFeeCommit` and `Reject`).
    pub fn reject_reason(&self) -> Option<&RejectReason> {
        match self {
            TransactionOutcome::OnlyFeeCommit(r) | TransactionOutcome::Reject(r) => Some(r),
            TransactionOutcome::Commit => None,
        }
    }
}

/// A submit acknowledgement: the transaction id plus its (optional, known-later) outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitResult {
    /// The submitted transaction's id.
    pub transaction_id: TransactionIdBytes,
    /// The outcome, once finalized. `None` while the transaction is still pending.
    pub outcome: Option<TransactionOutcome>,
}

/// A boundary fee receipt — the engine's [`tari_engine_types::fees::FeeReceipt`] flattened to plain,
/// **u64-safe** records (engine fee amounts are all `u64`, so no `U128` is needed here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeReceipt {
    /// The total amount of the fee payment(s), before refunds.
    pub total_fee_payment: u64,
    /// The total fees paid after refunds.
    pub total_fees_paid: u64,
    /// The non-refundable fees the user overpaid.
    pub total_fee_overcharge: u64,
    /// Per-source breakdown: `(FeeSource name, amount)` in the engine's canonical order.
    pub cost_breakdown: Vec<(String, u64)>,
}

/// One created (`up`) substate in a [`DiffSummary`]: its id and version (summary, not the body).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpSubstate {
    /// The substate id (canonical string form).
    pub substate_id: String,
    /// The created version.
    pub version: u64,
}

/// A boundary diff summary — the ids + versions of created/destroyed substates (not their bodies).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    /// Created (`up`) substates.
    pub up: Vec<UpSubstate>,
    /// Destroyed (`down`) substates as `(id, version)`.
    pub down: Vec<(String, u64)>,
}

/// A boundary event summary — the engine [`tari_engine_types::events::Event`] flattened to strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSummary {
    /// The emitting substate id, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substate_id: Option<String>,
    /// The template that emitted the event (canonical string form).
    pub template_address: String,
    /// The event topic.
    pub topic: String,
    /// The event payload as `(key, value)` string pairs (engine `Metadata` is `String -> String`).
    pub payload: Vec<(String, String)>,
}

/// A boundary log summary — the engine [`tari_engine_types::logs::LogEntry`] flattened to strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogSummary {
    /// The log message.
    pub message: String,
    /// The log level (e.g. `INFO`, `ERROR`).
    pub level: String,
}

/// The full, typed boundary view of a finalized transaction result.
///
/// One record with optional nested fields (not a union): the `submit` ack carries the id + accept/
/// reject outcome; the rest are present when the engine returned an execution result. A still-`Pending`
/// result yields a `FinalizedResult` whose `submit.outcome` is `None` and whose nested fields are empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalizedResult {
    /// The submit acknowledgement (id + outcome).
    pub submit: SubmitResult,
    /// The fee receipt, when an execution result was present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_receipt: Option<FeeReceipt>,
    /// The accepted substate diff summary, when the transaction (or its fee intent) was accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<DiffSummary>,
    /// The emitted events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<EventSummary>,
    /// The emitted logs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<LogSummary>,
    /// The epoch the transaction executed in, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    /// The estimated minimum fee (in microtari), surfaced **only for a dry-run** result.
    ///
    /// A dry-run executes the transaction without committing and meters the fee; this is the engine's
    /// [`tari_engine_types::fees::FeeReceipt::required_fees`] — the metered cost plus the allowance for
    /// the drift a differing `max_fee` introduces, i.e. the minimum `max_fee` the real submission can
    /// use. It is a native `u64` (never a float). A committed
    /// (non-dry-run) result leaves this `None` and exposes the realized fee in `fee_receipt` instead
    /// (additive field, omitted from JSON when absent so existing committed-result fixtures are
    /// unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_fee: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every canonical [`AbortReason`] variant — exhaustively listed so adding a 10th variant upstream
    /// forces this test (and the canonical code set) to be updated, not silently dropped.
    const ALL_ABORT_REASONS: &[AbortReason] = &[
        AbortReason::ForeignPledgeInputConflict,
        AbortReason::LockInputsFailed,
        AbortReason::LockOutputsFailed,
        AbortReason::LockInputsOutputsFailed,
        AbortReason::ExecutionFailure,
        AbortReason::OneOrMoreInputsNotFound,
        AbortReason::InsufficientFeesPaid,
        AbortReason::FeePaymentInMainIntent,
        AbortReason::EpochExpired,
        AbortReason::ValidityWindowTooLong,
    ];

    #[test]
    fn reject_reason_preserves_code_and_message() {
        let internal = InternalRejectReason::ExecutionFailure("boom".to_string());
        let boundary = RejectReason::from_internal(&internal);
        assert_eq!(boundary.code, "EXECUTION_FAILURE");
        assert_eq!(boundary.abort_code, None);
        assert_eq!(boundary.message, internal.to_string());
        assert!(boundary.message.contains("boom"));
    }

    #[test]
    fn every_variant_maps_to_a_code() {
        let variants = [
            InternalRejectReason::ExecutionFailure("a".into()),
            InternalRejectReason::SubstateNotFound("a".into()),
            InternalRejectReason::FailedToLockInputs("a".into()),
            InternalRejectReason::FailedToLockOutputs("a".into()),
            InternalRejectReason::ForeignPledgeInputConflict,
            InternalRejectReason::ForeignShardGroupDecidedToAbort {
                start_shard: 0,
                end_shard: 10,
                abort_reason: AbortReason::ExecutionFailure,
            },
            InternalRejectReason::InsufficientFeesPaid("a".into()),
            InternalRejectReason::FeePaymentInMainIntent,
            InternalRejectReason::Abort {
                reason: AbortReason::ExecutionFailure,
            },
        ];
        for v in &variants {
            let b = RejectReason::from_internal(v);
            assert!(!b.code.is_empty());
            assert_eq!(b.message, v.to_string());
        }
    }

    /// Existing top-level reject codes are frozen. This pins each one so a rename
    /// is caught at test time, not by a downstream host whose branch silently stops matching.
    #[test]
    fn top_level_reject_codes_are_stable() {
        let cases: &[(InternalRejectReason, &str)] = &[
            (InternalRejectReason::ExecutionFailure("a".into()), "EXECUTION_FAILURE"),
            (InternalRejectReason::SubstateNotFound("a".into()), "SUBSTATE_NOT_FOUND"),
            (
                InternalRejectReason::FailedToLockInputs("a".into()),
                "FAILED_TO_LOCK_INPUTS",
            ),
            (
                InternalRejectReason::FailedToLockOutputs("a".into()),
                "FAILED_TO_LOCK_OUTPUTS",
            ),
            (
                InternalRejectReason::ForeignPledgeInputConflict,
                "FOREIGN_PLEDGE_INPUT_CONFLICT",
            ),
            (
                InternalRejectReason::ForeignShardGroupDecidedToAbort {
                    start_shard: 0,
                    end_shard: 1,
                    abort_reason: AbortReason::ExecutionFailure,
                },
                "FOREIGN_SHARD_GROUP_DECIDED_TO_ABORT",
            ),
            (
                InternalRejectReason::InsufficientFeesPaid("a".into()),
                "INSUFFICIENT_FEES_PAID",
            ),
            (
                InternalRejectReason::FeePaymentInMainIntent,
                "FEE_PAYMENT_IN_MAIN_INTENT",
            ),
            (
                InternalRejectReason::Abort {
                    reason: AbortReason::ExecutionFailure,
                },
                "ABORT",
            ),
        ];
        for (reason, code) in cases {
            assert_eq!(RejectReason::from_internal(reason).code, *code);
        }
    }

    /// Each of the 10 canonical `AbortReason` variants surfaces a non-empty, unique stable code.
    #[test]
    fn abort_codes_are_non_empty_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for reason in ALL_ABORT_REASONS {
            let code = abort_code(reason);
            assert!(!code.is_empty(), "abort code for {reason:?} is empty");
            assert!(seen.insert(code), "duplicate abort code {code} for {reason:?}");
        }
        assert_eq!(seen.len(), 10, "expected exactly 10 canonical abort codes");
        assert_eq!(abort_code(&AbortReason::EpochExpired), "EPOCH_EXPIRED");
        assert_eq!(
            abort_code(&AbortReason::ValidityWindowTooLong),
            "VALIDITY_WINDOW_TOO_LONG"
        );
    }

    /// The `Abort` arm surfaces the canonical abort sub-code while keeping its top-level `ABORT` code.
    #[test]
    fn abort_variant_surfaces_sub_code() {
        let r = RejectReason::from_internal(&InternalRejectReason::Abort {
            reason: AbortReason::EpochExpired,
        });
        assert_eq!(r.code, "ABORT");
        assert_eq!(r.abort_code.as_deref(), Some("EPOCH_EXPIRED"));
    }

    #[test]
    fn outcome_helpers() {
        assert!(TransactionOutcome::Commit.is_commit());
        let r = RejectReason {
            code: "ABORT".into(),
            abort_code: None,
            message: "x".into(),
        };
        let oc = TransactionOutcome::Reject(r.clone());
        assert!(!oc.is_commit());
        assert_eq!(oc.reject_reason(), Some(&r));
    }

    /// Fee amounts must serialize as bare `u64` JSON numbers — including values `> 2^53`, which would
    /// lose precision if anything coerced them through an `f64`.
    #[test]
    fn fee_receipt_round_trips_large_u64() {
        let big = (1u64 << 53) + 12_345;
        let fr = FeeReceipt {
            total_fee_payment: big,
            total_fees_paid: big - 1,
            total_fee_overcharge: 0,
            cost_breakdown: vec![("Storage".into(), big), ("Initial".into(), 7)],
        };
        let json = serde_json::to_string(&fr).unwrap();
        assert!(
            json.contains(&big.to_string()),
            "large fee must serialize as a bare u64: {json}"
        );
        let back: FeeReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(back, fr);
    }

    #[test]
    fn finalized_result_round_trips() {
        let fr = FinalizedResult {
            submit: SubmitResult {
                transaction_id: TransactionIdBytes::from_bytes(&[0xab; 32]),
                outcome: Some(TransactionOutcome::Commit),
            },
            fee_receipt: Some(FeeReceipt {
                total_fee_payment: 100,
                total_fees_paid: 100,
                total_fee_overcharge: 0,
                cost_breakdown: vec![("Initial".into(), 100)],
            }),
            diff_summary: Some(DiffSummary {
                up: vec![UpSubstate {
                    substate_id: "component_aa".into(),
                    version: 0,
                }],
                down: vec![("vault_bb".into(), 3)],
            }),
            events: vec![EventSummary {
                substate_id: Some("component_aa".into()),
                template_address: "tmpl".into(),
                topic: "std.deposit".into(),
                payload: vec![("amount".into(), "100".into())],
            }],
            logs: vec![LogSummary {
                message: "hi".into(),
                level: "INFO".into(),
            }],
            epoch: Some(42),
            estimated_fee: None,
        };
        let json = serde_json::to_string(&fr).unwrap();
        // A committed result omits `estimated_fee` from the wire (it is `None`).
        assert!(
            !json.contains("estimated_fee"),
            "absent estimated_fee must be omitted: {json}"
        );
        let back: FinalizedResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, fr);
    }

    /// A dry-run result surfaces a large `estimated_fee` as a bare `u64` (no float coercion).
    #[test]
    fn finalized_result_estimated_fee_round_trips_large_u64() {
        let big = (1u64 << 53) + 99;
        let fr = FinalizedResult {
            submit: SubmitResult {
                transaction_id: TransactionIdBytes::from_bytes(&[0xcd; 32]),
                outcome: Some(TransactionOutcome::Commit),
            },
            fee_receipt: None,
            diff_summary: None,
            events: Vec::new(),
            logs: Vec::new(),
            epoch: None,
            estimated_fee: Some(big),
        };
        let json = serde_json::to_string(&fr).unwrap();
        assert!(
            json.contains(&big.to_string()),
            "estimated_fee must serialize as a bare u64: {json}"
        );
        let back: FinalizedResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, fr);
    }

    #[test]
    fn submit_result_serde_round_trips() {
        let sr = SubmitResult {
            transaction_id: TransactionIdBytes::from_bytes(&[0xde, 0xad, 0xbe, 0xef]),
            outcome: Some(TransactionOutcome::Commit),
        };
        let json = serde_json::to_string(&sr).unwrap();
        let back: SubmitResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sr);
    }
}
