//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use std::time::Duration;

use ootle_byte_type::ToByteType;
use tari_consensus_types::Decision;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{
    Epoch,
    commit_result::{AbortReason, ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    crypto::commit_amount,
    fees::{FeeBreakdown, FeeReceipt, FeeSource},
    substate::{SubstateDiff, SubstateId},
};
use tari_ootle_common_types::SubstateVersion;
use tari_ootle_transaction::{Transaction, TransactionId, args};
use tari_ootle_wallet_sdk::{
    models::TransactionStatus,
    network::{TransactionFinalizedResult, TransactionQueryResult},
    storage::{WalletStoreWriter, WriteableWalletStore},
};
use tari_template_lib::types::{ConfidentialOutputAddress, constants::STEALTH_TARI_RESOURCE_ADDRESS};
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

#[tokio::test]
async fn an_abort_releases_the_locked_outputs_whatever_its_execution_accepted() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();

    // The execution before the abort decision accepted a diff that spends both outputs the transaction locks.
    let mut spends = SubstateDiff::new();
    for amount in [25u64, 49] {
        let commitment = commit_amount(&Default::default(), amount.into())
            .unwrap()
            .to_byte_type();
        spends.down(
            SubstateId::ConfidentialOutput(ConfidentialOutputAddress::new(
                STEALTH_TARI_RESOURCE_ADDRESS,
                commitment,
            )),
            SubstateVersion::ZERO,
        );
    }
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
    execution_result.as_mut().unwrap().finalize.result = TransactionResult::Accept(spends);

    let test = TestWithNetwork::with_network(CannedTransactionResultInterface::new(query_result));
    test.add_unspent_output(25u64);
    test.add_unspent_output(49u64);
    test.store()
        .with_write_tx(|tx| tx.transactions_insert(&transaction, None, &[Test::test_account_address()], false))
        .unwrap();
    let lock_id = test.new_lock().keep_locked();
    test.sdk()
        .confidential_outputs_api()
        .lock_outputs_by_amount(lock_id, &Test::test_vault_address(), 50u64)
        .unwrap();
    test.sdk()
        .transaction_api()
        .locks_set_transaction_id(lock_id, transaction_id)
        .unwrap();
    assert_eq!(test.get_unspent_balance(), 0);

    let finalized = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(finalized.status, TransactionStatus::Rejected);
    assert_eq!(test.get_unspent_balance(), 74, "both outputs are released, not spent");
}
