//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{iter, time::Duration};

use rand::{distributions::Alphanumeric, rngs::OsRng, Rng};
use tari_common_types::types::PrivateKey;
use tari_dan_storage::consensus_models::{Decision, ExecutedTransaction, TransactionRecord};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    component::{ComponentBody, ComponentHeader},
    fees::FeeReceipt,
    substate::{Substate, SubstateDiff},
};
use tari_transaction::{SubstateRequirement, Transaction, VersionedSubstateId};

use crate::support::helpers::random_substate_in_shard;

pub fn build_transaction_from(
    tx: Transaction,
    decision: Decision,
    fee: u64,
    resulting_outputs: Vec<VersionedSubstateId>,
) -> TransactionRecord {
    let tx_id = *tx.id();
    let result = if decision.is_commit() {
        let mut diff = SubstateDiff::new();
        for output in &resulting_outputs {
            let s = (0..100).map(|_| OsRng.sample(Alphanumeric) as char).collect::<String>();
            let random_state = tari_bor::to_value(&s).unwrap();
            diff.up(
                output.substate_id.clone(),
                Substate::new(0, ComponentHeader {
                    template_address: Default::default(),
                    module_name: "Test".to_string(),
                    owner_key: Default::default(),
                    owner_rule: Default::default(),
                    access_rules: Default::default(),
                    entity_id: output.substate_id.as_component_address().unwrap().entity_id(),
                    body: ComponentBody { state: random_state },
                }),
            )
        }
        TransactionResult::Accept(diff)
    } else {
        TransactionResult::Reject(RejectReason::ExecutionFailure("Test failure".to_string()))
    };

    let mut tx = TransactionRecord::new(tx);
    tx.result = Some(ExecuteResult {
        finalize: FinalizeResult::new(tx_id.into_array().into(), vec![], vec![], result, FeeReceipt {
            total_fee_payment: fee.try_into().unwrap(),
            total_fees_paid: fee.try_into().unwrap(),
            cost_breakdown: vec![],
        }),
    });
    tx.resulting_outputs = resulting_outputs;
    tx.execution_time = Some(Duration::from_secs(0));
    // If the transaction does not require any inputs, we immediately resolve them to an empty set
    if tx.transaction.all_inputs_iter().next().is_none() {
        tx.resolved_inputs = Some(Default::default());
    }
    tx
}

pub fn build_transaction(
    decision: Decision,
    fee: u64,
    total_num_outputs: usize,
    num_committees: u32,
) -> TransactionRecord {
    let k = PrivateKey::default();
    let tx = Transaction::builder().sign(&k).build();

    // We create these outputs so that the test VNs dont have to have any UP substates
    // Equal potion of shards to each committee
    let outputs = (0..num_committees)
        .flat_map(|shard| {
            iter::repeat_with(move || random_substate_in_shard(shard.into(), num_committees))
                .take(total_num_outputs.div_ceil(num_committees as usize))
        })
        .collect::<Vec<_>>();

    build_transaction_from(tx, decision, fee, outputs)
}

pub fn build_transaction_with_inputs<I: IntoIterator<Item = SubstateRequirement>>(
    decision: Decision,
    fee: u64,
    inputs: I,
) -> TransactionRecord {
    let k = PrivateKey::default();
    let tx = Transaction::builder().with_inputs(inputs).sign(&k).build();
    build_transaction_from(tx, decision, fee, vec![])
}

pub fn change_decision(tx: ExecutedTransaction, new_decision: Decision) -> TransactionRecord {
    let total_fees_paid = tx
        .result()
        .finalize
        .fee_receipt
        .total_allocated_fee_payments()
        .as_u64_checked()
        .unwrap();
    let resulting_outputs = tx.resulting_outputs().to_vec();
    build_transaction_from(tx.into_transaction(), new_decision, total_fees_paid, resulting_outputs)
}
