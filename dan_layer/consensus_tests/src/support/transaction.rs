//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{iter, time::Duration};

use tari_common_types::types::PrivateKey;
use tari_dan_common_types::ShardId;
use tari_dan_storage::consensus_models::{Decision, ExecutedTransaction};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    fees::{FeeCostBreakdown, FeeReceipt},
    substate::SubstateDiff,
};
use tari_transaction::Transaction;

use crate::support::helpers::random_shard_in_bucket;

pub fn build_transaction_from(
    tx: Transaction,
    decision: Decision,
    fee: u64,
    resulting_outputs: Vec<ShardId>,
) -> ExecutedTransaction {
    let tx_id = *tx.id();
    ExecutedTransaction::new(
        tx,
        ExecuteResult {
            finalize: FinalizeResult::new(
                tx_id.into_array().into(),
                vec![],
                vec![],
                if decision.is_commit() {
                    TransactionResult::Accept(SubstateDiff::new())
                } else {
                    TransactionResult::Reject(RejectReason::ExecutionFailure("Test failure".to_string()))
                },
                FeeCostBreakdown {
                    total_fees_charged: fee.try_into().unwrap(),
                    breakdown: vec![],
                },
            ),
            transaction_failure: None,
            fee_receipt: Some(FeeReceipt {
                total_fee_payment: fee.try_into().unwrap(),
                total_fees_paid: fee.try_into().unwrap(),
                cost_breakdown: vec![],
            }),
        },
        resulting_outputs,
        Duration::from_secs(0),
    )
}

pub fn build_transaction(decision: Decision, fee: u64, num_shards: usize, num_committees: u32) -> ExecutedTransaction {
    let k = PrivateKey::default();
    let tx = Transaction::builder().sign(&k).build();

    // We create these outputs so that the test VNs dont have to have any UP substates
    // Equal potion of shards to each committee
    let outputs = (0..num_committees)
        .flat_map(|bucket| {
            iter::repeat_with(move || random_shard_in_bucket(bucket.into(), num_committees))
                .take(num_shards / num_committees as usize)
        })
        .collect::<Vec<_>>();

    build_transaction_from(tx, decision, fee, outputs)
}

pub fn change_decision(tx: ExecutedTransaction, new_decision: Decision) -> ExecutedTransaction {
    let total_fees_paid = tx
        .result()
        .fee_receipt
        .as_ref()
        .unwrap()
        .total_allocated_fee_payments()
        .as_u64_checked()
        .unwrap();
    let resulting_outputs = tx.resulting_outputs().to_vec();
    build_transaction_from(tx.into_transaction(), new_decision, total_fees_paid, resulting_outputs)
}
