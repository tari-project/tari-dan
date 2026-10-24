//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Full finalized-result parsing (the inverse of the build path).
//!
//! A host hands the core the raw **indexer** finalized-result JSON — exactly what
//! `GET /transactions/{id}/result` returns (`IndexerTransactionFinalizedResult`) — and the core parses
//! it into the typed, u64-safe boundary records in [`crate::types::result`]: a [`SubmitResult`]
//! outcome with the canonical [`RejectReason`] (incl. the canonical [`AbortReason`] sub-code), plus the
//! fee receipt, diff summary, events, and logs. The accept/reject mapping is
//! `Accept => Commit`, `AcceptFeeRejectRest => OnlyFeeCommit`, `Reject => Reject`.
//!
//! ## Why a local wire mirror, not `tari_indexer_client`
//!
//! [`tari_indexer_client::types::IndexerTransactionFinalizedResult`] is the canonical wire type, but
//! that crate pulls `reqwest`/`tokio` into its tree, which the pure core forbids. So we deserialize
//! into a local serde mirror ([`WireFinalizedResult`]) whose variant names and fields match the
//! indexer enum byte-for-byte on the wire, embedding the engine's [`ExecuteResult`] in its base64 CBOR encoding
//! and the canonical [`Decision`]. The timestamp / duration fields are accepted as opaque
//! `serde_json::Value` (we never read them), so we avoid a dependency on the `time` crate.
//!
//! ## Indexer vs wallet-daemon wrapper
//!
//! This parses the **indexer** wrapper. The wallet daemon's `TransactionWaitResultResponse` is a
//! different shape (`result: Option<FinalizeResult>` — no `final_decision`/`abort_details` envelope);
//! a daemon-path parser would reuse [`finalized_from_execute_result`] over its inner `FinalizeResult`.
//!
//! ## Dry-run results
//!
//! A **dry run** (the indexer `POST /transactions/dry-run`, response
//! `SubmitTransactionDryRunResponse { transaction_id, result }`, `result` the base64 CBOR `ExecuteResult`) executes
//! the transaction without committing and meters the fee. Its wire shape is a different object — an
//! unwrapped `ExecuteResult` under a `result` key rather than the `Finalized`/`Rejected`/`Pending`
//! enum envelope — so [`parse_finalized_result`] shape-dispatches: a top-level `result` key routes to
//! [`parse_dry_run_result`], which fills the additive `estimated_fee` (the engine's
//! [`tari_engine_types::fees::FeeReceipt::required_fees`]). A committed result never sets
//! `estimated_fee`; it exposes the realized `fee_receipt` instead.

use serde::Deserialize;
use tari_consensus_types::Decision;
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, TransactionResult},
    encoded::Encoded,
    fees::FeeSource,
    substate::SubstateDiff,
};

use crate::types::{
    bytes::TransactionIdBytes,
    error::OotleSdkError,
    result::{
        DiffSummary,
        EventSummary,
        FeeReceipt,
        FinalizedResult,
        LogSummary,
        RejectReason,
        SubmitResult,
        TransactionOutcome,
        UpSubstate,
    },
};

/// A local, deserialize-only mirror of `tari_indexer_client::types::IndexerTransactionFinalizedResult`.
///
/// The variant names + field names match the indexer enum exactly so this deserializes the same JSON
/// (`serde`'s default externally-tagged enum representation). Timestamp / duration fields are accepted
/// as opaque `Value` because the boundary records never expose them. The `final_decision` and the
/// timestamp/duration fields are part of the wire shape but unread by the parser (the outcome is
/// derived from `finalize.result`); they are kept so deserialization matches the indexer JSON exactly.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
enum WireFinalizedResult {
    /// The transaction is not yet finalized.
    Pending,
    /// The transaction was finalized (committed or aborted) by the network.
    Finalized {
        final_decision: Decision,
        execution_result: Option<Encoded<ExecuteResult>>,
        #[serde(default)]
        execution_time: serde_json::Value,
        #[serde(default)]
        finalized_time: serde_json::Value,
        abort_details: Option<String>,
    },
    /// The transaction was rejected by mempool validation and never sequenced.
    Rejected {
        details: String,
        #[serde(default)]
        rejected_time: serde_json::Value,
    },
}

/// Parses a raw **indexer** finalized-result JSON into the typed boundary [`FinalizedResult`].
///
/// - `Pending` ⇒ a clean pending result: `submit.outcome == None`, nested fields empty.
/// - `Finalized` ⇒ maps `finalize.result` via the `tx_watcher` arms and fills fee/diff/events/logs/epoch. If
///   `execution_result` is absent (an abort with no execution detail) the outcome is a `Reject` built from
///   `abort_details`.
/// - `Rejected` ⇒ a `Reject` outcome built from `details`.
///
/// A deserialize failure is an [`OotleSdkError::Parse`].
pub fn parse_finalized_result(raw: &str) -> Result<FinalizedResult, OotleSdkError> {
    // A dry-run response (`SubmitTransactionDryRunResponse`) is an unwrapped `ExecuteResult` under a
    // top-level `result` key — structurally distinct from the committed `Finalized`/`Rejected`/`Pending`
    // enum envelope. Detect it cheaply and route to the dry-run parser (which fills `estimated_fee`).
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| OotleSdkError::Parse(format!("indexer finalized-result JSON: {e}")))?;
    if value.get("result").is_some() {
        return parse_dry_run_result_value(value);
    }

    let wire: WireFinalizedResult = serde_json::from_value(value)
        .map_err(|e| OotleSdkError::Parse(format!("indexer finalized-result JSON: {e}")))?;

    match wire {
        WireFinalizedResult::Pending => Ok(FinalizedResult {
            submit: SubmitResult {
                // No execution result yet ⇒ no transaction hash to surface; empty id is the pending sentinel.
                transaction_id: TransactionIdBytes::from_bytes(&[]),
                outcome: None,
            },
            fee_receipt: None,
            diff_summary: None,
            events: Vec::new(),
            logs: Vec::new(),
            epoch: None,
            estimated_fee: None,
        }),
        WireFinalizedResult::Rejected { details, .. } => Ok(FinalizedResult {
            submit: SubmitResult {
                transaction_id: TransactionIdBytes::from_bytes(&[]),
                outcome: Some(TransactionOutcome::Reject(RejectReason::from_mempool_details(details))),
            },
            fee_receipt: None,
            diff_summary: None,
            events: Vec::new(),
            logs: Vec::new(),
            epoch: None,
            estimated_fee: None,
        }),
        WireFinalizedResult::Finalized {
            final_decision,
            execution_result,
            abort_details,
            ..
        } => match execution_result {
            Some(encoded) => {
                let result = encoded
                    .decode()
                    .map_err(|e| OotleSdkError::Parse(format!("indexer execution_result CBOR: {e}")))?;
                // Defensive cross-check: the envelope's `final_decision` is redundant with
                // `finalize.result`. If they disagree (a version-skewed / buggy indexer), refuse to
                // silently return a wrong outcome.
                let derived = Decision::from(&result.finalize.result);
                if !derived.is_same_outcome(final_decision) {
                    return Err(OotleSdkError::Parse(format!(
                        "indexer final_decision ({final_decision}) disagrees with execution_result ({derived})"
                    )));
                }
                Ok(finalized_from_execute_result(&result))
            },
            // A commit always carries an execution result, so a missing one is a rejection: fall
            // back to `abort_details` for the message.
            None => Ok(FinalizedResult {
                submit: SubmitResult {
                    transaction_id: TransactionIdBytes::from_bytes(&[]),
                    outcome: Some(TransactionOutcome::Reject(RejectReason::from_abort_details(
                        abort_details.unwrap_or_else(|| "Unknown".to_string()),
                    ))),
                },
                fee_receipt: None,
                diff_summary: None,
                events: Vec::new(),
                logs: Vec::new(),
                epoch: None,
                estimated_fee: None,
            }),
        },
    }
}

/// A local, deserialize-only mirror of `tari_indexer_client::types::SubmitTransactionDryRunResponse`.
///
/// Only the `result` field is declared; the response's `transaction_id` is silently ignored by serde
/// (no `deny_unknown_fields`). That is intentional — a dry-run is never committed, so the authoritative
/// id is the `finalize.transaction_hash` carried inside `result` (the same id the committed path
/// surfaces), and we never need the wrapper's `transaction_id`.
#[derive(Debug, Deserialize)]
struct WireDryRunResult {
    /// The CBOR-encoded engine execution result (same `ExecuteResult` a committed transaction yields),
    /// metered for fees but never written to the ledger.
    result: Encoded<ExecuteResult>,
}

/// Parses a raw **indexer dry-run** result JSON (`SubmitTransactionDryRunResponse`) into the typed
/// boundary [`FinalizedResult`], with the additive `estimated_fee` populated from the metered receipt.
///
/// A dry-run executes fully (so it carries the same events / diff / logs an accepted transaction would)
/// but is never committed; the surfaced `estimated_fee` is the engine's
/// [`tari_engine_types::fees::FeeReceipt::required_fees`] — the metered cost plus the allowance for
/// the drift a differing `max_fee` introduces, i.e. the minimum `max_fee` the real submission can
/// use. A deserialize failure is an [`OotleSdkError::Parse`].
pub fn parse_dry_run_result(raw: &str) -> Result<FinalizedResult, OotleSdkError> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| OotleSdkError::Parse(format!("indexer dry-run result JSON: {e}")))?;
    parse_dry_run_result_value(value)
}

/// Shared body for [`parse_dry_run_result`] and the dry-run arm of [`parse_finalized_result`].
fn parse_dry_run_result_value(value: serde_json::Value) -> Result<FinalizedResult, OotleSdkError> {
    let wire: WireDryRunResult =
        serde_json::from_value(value).map_err(|e| OotleSdkError::Parse(format!("indexer dry-run result JSON: {e}")))?;
    let result = wire
        .result
        .decode()
        .map_err(|e| OotleSdkError::Parse(format!("indexer dry-run result CBOR: {e}")))?;
    let mut finalized = finalized_from_execute_result(&result);
    finalized.estimated_fee = Some(result.finalize.required_fees());
    Ok(finalized)
}

/// Builds the full [`FinalizedResult`] from an engine [`ExecuteResult`].
///
/// Shared between the indexer path here and any future daemon path (which also yields a `FinalizeResult`).
pub fn finalized_from_execute_result(result: &ExecuteResult) -> FinalizedResult {
    let finalize = &result.finalize;
    let outcome = outcome_from_result(&finalize.result);

    FinalizedResult {
        submit: SubmitResult {
            transaction_id: TransactionIdBytes::from_bytes(finalize.transaction_hash.as_slice()),
            outcome: Some(outcome),
        },
        fee_receipt: Some(fee_receipt_summary(finalize)),
        // Accept (full or fee-only) carries a diff; a pure reject does not.
        diff_summary: finalize.any_accept().map(diff_summary),
        events: event_summaries(finalize),
        logs: log_summaries(finalize),
        epoch: result.execute_epoch.map(|e| e.as_u64()),
        // `estimated_fee` is a dry-run-only field; the committed path exposes the realized
        // `fee_receipt` above and leaves this `None`. The dry-run parser overwrites it.
        estimated_fee: None,
    }
}

/// Maps a [`TransactionResult`] to a boundary [`TransactionOutcome`].
fn outcome_from_result(result: &TransactionResult) -> TransactionOutcome {
    match result {
        TransactionResult::Accept(_) => TransactionOutcome::Commit,
        TransactionResult::AcceptFeeRejectRest(_, reason) => {
            TransactionOutcome::OnlyFeeCommit(RejectReason::from_internal(reason))
        },
        TransactionResult::Reject(reason) => TransactionOutcome::Reject(RejectReason::from_internal(reason)),
    }
}

/// Flattens the engine fee receipt to the u64-safe boundary [`FeeReceipt`].
fn fee_receipt_summary(finalize: &FinalizeResult) -> FeeReceipt {
    let receipt = &finalize.fee_receipt;
    FeeReceipt {
        total_fee_payment: receipt.total_fee_payment(),
        total_fees_paid: receipt.total_fees_paid(),
        total_fee_overcharge: receipt.total_fee_overcharge(),
        cost_breakdown: receipt
            .fee_breakdown()
            .iter()
            .map(|(source, amount)| (fee_source_name(*source).to_string(), *amount))
            .collect(),
    }
}

/// The stable, SDK-owned name for a [`FeeSource`] — exhaustive so a new upstream variant fails to
/// compile here (forcing an explicit decision) rather than silently changing a boundary string. Kept
/// SDK-owned (not the upstream `Debug`) so the `cost_breakdown` keys hosts read are a stable contract.
fn fee_source_name(source: FeeSource) -> &'static str {
    match source {
        FeeSource::Initial => "Initial",
        FeeSource::RuntimeCall => "RuntimeCall",
        FeeSource::Storage => "Storage",
        FeeSource::TransactionWeight => "TransactionWeight",
        FeeSource::TemplateLoad => "TemplateLoad",
        FeeSource::SubstateCreate => "SubstateCreate",
        FeeSource::WasmExecution => "WasmExecution",
        FeeSource::TemplatePublish => "TemplatePublish",
        FeeSource::NativeExecution => "NativeExecution",
        FeeSource::InputLoad => "InputLoad",
        FeeSource::InputFanout => "InputFanout",
    }
}

/// Summarizes a [`SubstateDiff`] to ids + versions only (no bodies).
fn diff_summary(diff: &SubstateDiff) -> DiffSummary {
    DiffSummary {
        up: diff
            .up_iter()
            .map(|(id, substate)| UpSubstate {
                substate_id: id.to_string(),
                version: substate.version().as_u64(),
            })
            .collect(),
        down: diff.down_iter().map(|(id, v)| (id.to_string(), v.as_u64())).collect(),
    }
}

/// Flattens the engine events to boundary [`EventSummary`] records.
fn event_summaries(finalize: &FinalizeResult) -> Vec<EventSummary> {
    finalize
        .events
        .iter()
        .map(|event| EventSummary {
            substate_id: event.substate_id().map(|id| id.to_string()),
            template_address: event.template_address().to_string(),
            topic: event.topic().to_string(),
            payload: event
                .payload()
                .iter()
                .map(|(key, value)| (key.clone(), payload_value_json(value)))
                .collect(),
        })
        .collect()
}

/// The JSON form of one event payload value, as [`EventSummary::payload`] documents it.
fn payload_value_json(value: &tari_bor::RawCbor) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_else(|_| {
        serde_json::json!({
            "@cbor": "raw",
            "hex": hex::encode(value.as_bytes()),
        })
    })
}

/// Flattens the engine logs to boundary [`LogSummary`] records.
fn log_summaries(finalize: &FinalizeResult) -> Vec<LogSummary> {
    finalize
        .logs
        .iter()
        .map(|entry| LogSummary {
            message: entry.message.clone(),
            level: entry.level.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tari_engine_types::{
        SubstateVersion,
        commit_result::{
            AbortReason,
            ExecuteResult,
            FinalizeResult,
            RejectReason as InternalRejectReason,
            TransactionResult,
        },
        events::Event,
        fees::{FeeBreakdown, FeeReceiptBuilder, FeeSource},
        logs::LogEntry,
        substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
    };
    use tari_template_lib_types::{Amount, ComponentAddress, Hash32, LogLevel, Metadata, ObjectKey, VaultId};

    use super::*;

    fn execute_result(result: TransactionResult, epoch: Option<u64>) -> ExecuteResult {
        let mut breakdown = FeeBreakdown::default();
        breakdown.add(FeeSource::Initial, 7);
        // A fee `> 2^53` to prove the u64-safe path survives parsing.
        breakdown.add(FeeSource::Storage, (1u64 << 53) + 11);
        let fee_receipt = FeeReceiptBuilder::default()
            .with_total_fee_payment((1u64 << 53) + 20)
            .with_total_fees_paid((1u64 << 53) + 18)
            .with_total_fee_overcharge(0)
            .with_cost_breakdown(breakdown)
            .build();

        let mut metadata = Metadata::new();
        metadata.insert("amount", &Amount::from(100u64));
        let event = Event::new(None, Hash32::from_array([9u8; 32]), "std.deposit".to_string(), metadata);

        let finalize = FinalizeResult {
            transaction_hash: Hash32::from_array([0xab; 32]),
            events: vec![event],
            logs: vec![LogEntry::new(LogLevel::Info, "hello".to_string())],
            result,
            total_fees_required: fee_receipt.total_fees_charged(),
            fee_receipt,
        };

        ExecuteResult {
            finalize,
            execution_time: std::time::Duration::from_secs(1),
            execute_epoch: epoch.map(tari_engine_types::Epoch),
            wasm_execution_points: 0,
            native_execution_points: 0,
        }
    }

    fn accept_diff() -> SubstateDiff {
        let mut diff = SubstateDiff::new();
        let component = ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH]));
        diff.up(
            SubstateId::Component(component),
            Substate::new(SubstateVersion::ZERO, SubstateValue::Component(sample_component())),
        );
        diff.down(
            SubstateId::Vault(VaultId::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH]))),
            SubstateVersion::new(3),
        );
        diff
    }

    fn sample_component() -> tari_engine_types::component::Component {
        use tari_engine_types::component::{Component, ComponentBody, ComponentHeader};
        use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
        use tari_template_lib_types::{EntityId, SubstateOwnerRule, access_rules::ComponentAccessRules};
        Component {
            header: ComponentHeader {
                template_address: ACCOUNT_TEMPLATE_ADDRESS,
                owner_rule: SubstateOwnerRule::None,
                access_rules: ComponentAccessRules::new(),
                entity_id: EntityId::from_array([0u8; EntityId::LENGTH]),
            },
            body: ComponentBody::from_cbor_value(tari_bor::to_value(&Vec::<VaultId>::new()).unwrap()),
        }
    }

    /// Serializes an `ExecuteResult` into the indexer **dry-run** response JSON
    /// (`SubmitTransactionDryRunResponse { transaction_id, result }`), exactly as `POST
    /// /transactions/dry-run` hands it back (the `result` is its base64 CBOR encoding).
    fn dry_run_wire_json(result: ExecuteResult) -> String {
        serde_json::json!({
            "transaction_id": result.finalize.transaction_hash,
            "result": Encoded::encode(&result).unwrap(),
        })
        .to_string()
    }

    /// Serializes an `ExecuteResult` into the indexer `Finalized` wrapper JSON, exactly as the indexer
    /// REST API hands it back (the `execution_result` is its base64 CBOR encoding).
    fn finalized_wire_json(result: ExecuteResult) -> String {
        let decision = Decision::from(&result.finalize.result);
        serde_json::json!({
            "Finalized": {
                "final_decision": decision,
                "execution_result": Encoded::encode(&result).unwrap(),
                "execution_time": { "secs": 1, "nanos": 0 },
                "finalized_time": "2026-06-12 00:00:00.0",
                "abort_details": null,
            }
        })
        .to_string()
    }

    #[test]
    fn parses_accept_with_fees_event_and_diff() {
        let json = finalized_wire_json(execute_result(TransactionResult::Accept(accept_diff()), Some(42)));
        let parsed = parse_finalized_result(&json).unwrap();

        assert_eq!(parsed.submit.outcome, Some(TransactionOutcome::Commit));
        assert_eq!(parsed.epoch, Some(42));

        let fr = parsed.fee_receipt.expect("accept has a fee receipt");
        assert_eq!(fr.total_fee_payment, (1u64 << 53) + 20);
        // Breakdown carries the large u64 intact.
        assert!(
            fr.cost_breakdown
                .iter()
                .any(|(s, a)| s == "Storage" && *a == (1u64 << 53) + 11)
        );

        let diff = parsed.diff_summary.expect("accept carries a diff");
        assert_eq!(diff.up.len(), 1);
        assert_eq!(diff.up[0].version, 0);
        assert_eq!(diff.down.len(), 1);
        assert_eq!(diff.down[0].1, 3);

        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].topic, "std.deposit");
        assert_eq!(parsed.events[0].payload, vec![(
            "amount".to_string(),
            serde_json::json!(100)
        )]);
        assert_eq!(parsed.logs.len(), 1);
        assert_eq!(parsed.logs[0].level, "INFO");
        // transaction id is the finalize transaction hash.
        assert_eq!(parsed.submit.transaction_id.to_hex(), "ab".repeat(32));
        // A committed result never sets the dry-run-only `estimated_fee`.
        assert_eq!(parsed.estimated_fee, None);
    }

    /// A dry-run result parses through the shape-dispatching `parse_finalized_result` and surfaces a
    /// `estimated_fee` of `required_fees()`, alongside the metered receipt, events, diff, and logs
    /// the full execution produced.
    #[test]
    fn parses_dry_run_with_estimated_fee() {
        let exec = execute_result(TransactionResult::Accept(accept_diff()), Some(5));
        // The metered total: Initial(7) + Storage((1<<53)+11) = (1<<53)+18, plus the fee-estimate
        // allowance the engine adds on top.
        let expected_estimate = exec.finalize.required_fees();
        assert!(expected_estimate > (1u64 << 53) + 18, "the estimate must clear 2^53");
        let json = dry_run_wire_json(exec);

        // Both the shape-dispatching entry point and the explicit dry-run parser agree.
        for parsed in [
            parse_finalized_result(&json).unwrap(),
            parse_dry_run_result(&json).unwrap(),
        ] {
            assert_eq!(parsed.estimated_fee, Some(expected_estimate));
            // The dry-run executed fully, so it carries the same receipt / diff / events / epoch.
            assert!(parsed.fee_receipt.is_some());
            assert!(parsed.diff_summary.is_some());
            assert_eq!(parsed.events.len(), 1);
            assert_eq!(parsed.epoch, Some(5));
            assert_eq!(parsed.submit.outcome, Some(TransactionOutcome::Commit));
            // The dry-run id is the finalize transaction hash.
            assert_eq!(parsed.submit.transaction_id.to_hex(), "ab".repeat(32));
        }
    }

    /// A malformed dry-run payload is a `PARSE` error, not a panic.
    #[test]
    fn malformed_dry_run_is_a_parse_error() {
        let err = parse_dry_run_result("{not json").unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    #[test]
    fn parses_accept_fee_reject_rest() {
        let result = TransactionResult::AcceptFeeRejectRest(
            accept_diff(),
            InternalRejectReason::execution_failure_unclassified("boom"),
        );
        let json = finalized_wire_json(execute_result(result, Some(7)));
        let parsed = parse_finalized_result(&json).unwrap();

        match parsed.submit.outcome {
            Some(TransactionOutcome::OnlyFeeCommit(ref r)) => {
                assert_eq!(r.code, "EXECUTION_FAILURE");
                assert!(r.message.contains("boom"));
            },
            other => panic!("expected OnlyFeeCommit, got {other:?}"),
        }
        // Fee-only acceptance still carries a diff + fee receipt.
        assert!(parsed.diff_summary.is_some());
        assert!(parsed.fee_receipt.is_some());
    }

    #[test]
    fn parses_reject_epoch_expired_with_abort_sub_code() {
        let result = TransactionResult::Reject(InternalRejectReason::Abort {
            reason: AbortReason::EpochExpired,
        });
        let json = finalized_wire_json(execute_result(result, None));
        let parsed = parse_finalized_result(&json).unwrap();

        match parsed.submit.outcome {
            Some(TransactionOutcome::Reject(ref r)) => {
                assert_eq!(r.code, "ABORT");
                assert_eq!(r.abort_code.as_deref(), Some("EPOCH_EXPIRED"));
            },
            other => panic!("expected Reject, got {other:?}"),
        }
        // A pure reject has no accepted diff.
        assert!(parsed.diff_summary.is_none());
    }

    #[test]
    fn parses_pending() {
        let parsed = parse_finalized_result("\"Pending\"").unwrap();
        assert!(parsed.submit.outcome.is_none());
        assert!(parsed.fee_receipt.is_none());
        assert!(parsed.diff_summary.is_none());
        assert!(parsed.events.is_empty());
    }

    #[test]
    fn parses_mempool_rejected() {
        let json = serde_json::json!({
            "Rejected": { "details": "bad signature", "rejected_time": "2026-06-12 00:00:00.0" }
        })
        .to_string();
        let parsed = parse_finalized_result(&json).unwrap();
        match parsed.submit.outcome {
            Some(TransactionOutcome::Reject(ref r)) => {
                // Mempool rejection has its own stable code, distinct from a sequenced `ABORT`.
                assert_eq!(r.code, "MEMPOOL_REJECTED");
                assert_eq!(r.abort_code, None);
                assert_eq!(r.message, "bad signature");
            },
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        let err = parse_finalized_result("{not json").unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    /// A `Finalized` envelope whose `final_decision` contradicts the embedded `execution_result` is a
    /// parse error — never a silently-wrong outcome.
    #[test]
    fn final_decision_disagreeing_with_execution_result_is_an_error() {
        // execution_result is an Accept (⇒ Commit), but final_decision says Abort.
        let exec = execute_result(TransactionResult::Accept(accept_diff()), Some(1));
        let json = serde_json::json!({
            "Finalized": {
                "final_decision": { "Abort": "EpochExpired" },
                "execution_result": Encoded::encode(&exec).unwrap(),
                "execution_time": { "secs": 1, "nanos": 0 },
                "finalized_time": "2026-06-12 00:00:00.0",
                "abort_details": null,
            }
        })
        .to_string();
        let err = parse_finalized_result(&json).unwrap_err();
        assert_eq!(err.code(), "PARSE");
        assert!(err.to_string().contains("disagrees"), "{err}");
    }
}
