//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{iter, time::Duration};

use tari_common_types::types::PrivateKey;
use tari_dan_storage::consensus_models::{Decision, ExecutedTransaction};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    fees::{FeeCostBreakdown, FeeReceipt},
    resource_container::ResourceContainer,
    substate::SubstateDiff,
};
use tari_transaction::Transaction;

use crate::support::helpers::random_shard_in_bucket;

pub fn build_transaction_from(tx: Transaction, decision: Decision, fee: u64) -> ExecutedTransaction {
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
                fee_resource: ResourceContainer::Confidential {
                    address: "resource_0000000000000000000000000000000000000000000000000000000000000000"
                        .parse()
                        .unwrap(),
                    commitments: Default::default(),
                    revealed_amount: fee.try_into().unwrap(),
                },
                cost_breakdown: vec![],
            }),
        },
        Duration::from_secs(0),
    )
}

pub fn build_transaction(decision: Decision, fee: u64, num_shards: usize, num_committees: u32) -> ExecutedTransaction {
    let k = PrivateKey::default();
    let mut tx = Transaction::builder().sign(&k).build();
    for bucket in 0..num_committees {
        // We fill these outputs so that the test VNs dont have to have any UP substates
        // Equal potion of shards to each committee
        tx.filled_outputs_mut().extend(
            iter::repeat_with(|| random_shard_in_bucket(bucket, num_committees))
                .take(num_shards / num_committees as usize),
        );
    }

    build_transaction_from(tx, decision, fee)
}

pub fn change_decision(tx: ExecutedTransaction, new_decision: Decision) -> ExecutedTransaction {
    let total_fees_charged = tx.result().fee_receipt.as_ref().unwrap().total_fee_payment;
    build_transaction_from(tx.into_transaction(), new_decision, total_fees_charged)
}
