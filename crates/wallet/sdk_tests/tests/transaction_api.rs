//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use std::time::Duration;

use tari_consensus_types::Decision;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{
    Epoch,
    commit_result::{AbortReason, ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    fees::{FeeBreakdown, FeeReceipt, FeeSource},
    substate::SubstateDiff,
};
use tari_ootle_transaction::{Transaction, TransactionId, args};
use tari_ootle_wallet_sdk::{
    models::TransactionStatus,
    network::{TransactionFinalizedResult, TransactionQueryResult},
    storage::{WalletStoreWriter, WriteableWalletStore},
};
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::support::{CannedTransactionResultInterface, Test, TestWithNetwork};

fn now() -> PrimitiveDateTime {
    let now = OffsetDateTime::now_utc();
    PrimitiveDateTime::new(now.date(), now.time())
}

fn build_transaction() -> Transaction {
    Transaction::builder_localnet(Epoch(100))
        .allocate_component_address("component")
        .put_last_instruction_output_on_workspace("bucket")
        .call_method("component", "new", args!["bucket"])
        .build_and_seal(&RistrettoSecretKey::from(1))
}

/// A fee-only accept where the fee intent could not cover everything charged: 1000 was paid up front and all of it
/// was consumed, but the execution charged 2548.
fn underpaid_fee_receipt() -> FeeReceipt {
    let mut cost_breakdown = FeeBreakdown::default();
    cost_breakdown.add(FeeSource::Initial, 2548);

    FeeReceipt::builder()
        .with_total_fee_payment(1000)
        .with_total_fees_paid(1000)
        .with_cost_breakdown(cost_breakdown)
        .build()
}

fn finalized_result(transaction_id: TransactionId, fee_receipt: FeeReceipt) -> TransactionQueryResult {
    let finalize = FinalizeResult::new(
        transaction_id.into_array().into(),
        vec![],
        vec![],
        TransactionResult::AcceptFeeRejectRest(
            SubstateDiff::new(),
            RejectReason::execution_failure_unclassified("out of fees"),
        ),
        fee_receipt,
    );

    TransactionQueryResult {
        transaction_id,
        result: TransactionFinalizedResult::Finalized {
            final_decision: Decision::Commit,
            execution_result: Some(Box::new(ExecuteResult {
                finalize,
                execution_time: Duration::from_secs(1),
                execute_epoch: None,
                wasm_execution_points: 0,
                native_execution_points: 0,
            })),
            execution_time: Duration::from_secs(1),
            finalized_time: now(),
            abort_details: None,
        },
        receipt: None,
    }
}

#[tokio::test]
async fn final_fee_is_what_was_paid_not_what_was_required() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let fee_receipt = underpaid_fee_receipt();
    // `required_fees()` is a dry run estimate for a later submit, never what a settled transaction cost, so the two
    // must be distinguishable here.
    assert_ne!(fee_receipt.total_fees_paid(), fee_receipt.required_fees());

    let test = TestWithNetwork::with_network(CannedTransactionResultInterface::new(finalized_result(
        transaction_id,
        fee_receipt.clone(),
    )));
    test.store()
        .with_write_tx(|tx| tx.transactions_insert(&transaction, None, &[Test::test_account_address()], false))
        .unwrap();

    let finalized = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(finalized.status, TransactionStatus::OnlyFeeAccepted);
    assert_eq!(finalized.final_fee, Some(fee_receipt.total_fees_paid()));
}

#[tokio::test]
async fn final_fee_is_zero_for_a_rejected_transaction() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();

    let mut query_result = finalized_result(transaction_id, FeeReceipt::default());
    let TransactionFinalizedResult::Finalized {
        final_decision,
        execution_result,
        ..
    } = &mut query_result.result
    else {
        unreachable!()
    };
    *final_decision = Decision::Abort(AbortReason::InsufficientFeesPaid);
    execution_result.as_mut().unwrap().finalize.result =
        TransactionResult::Reject(RejectReason::execution_failure_unclassified("nope"));

    let test = TestWithNetwork::with_network(CannedTransactionResultInterface::new(query_result));
    test.store()
        .with_write_tx(|tx| tx.transactions_insert(&transaction, None, &[Test::test_account_address()], false))
        .unwrap();

    let finalized = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(finalized.status, TransactionStatus::Rejected);
    assert_eq!(finalized.final_fee, Some(0));
}
