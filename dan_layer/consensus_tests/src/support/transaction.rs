//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::iter;

use tari_common_types::types::PrivateKey;
use tari_dan_storage::consensus_models::{Decision, ExecutedTransaction};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    fees::{FeeCostBreakdown, FeeReceipt},
    resource_container::ResourceContainer,
    substate::SubstateDiff,
};

use crate::support::helpers::random_shard;

pub fn build_transaction(decision: Decision, fee: u64, num_shards: usize) -> ExecutedTransaction {
    let k = PrivateKey::default();

    let mut tx = tari_transaction::Transaction::builder().sign(&k).build();
    // We fill these outputs so that the test VNs dont have to have any actual substates
    tx.filled_outputs_mut()
        .extend(iter::repeat_with(random_shard).take(num_shards));

    let tx_id = *tx.id();
    ExecutedTransaction::new(tx, ExecuteResult {
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
    })
}
