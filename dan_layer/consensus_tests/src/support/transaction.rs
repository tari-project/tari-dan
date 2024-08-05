//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{iter, time::Duration};

use rand::{distributions::Alphanumeric, rngs::OsRng, Rng};
use tari_common_types::types::PrivateKey;
use tari_dan_storage::consensus_models::{
    BlockId,
    Decision,
    ExecutedTransaction,
    TransactionExecution,
    TransactionRecord,
    VersionedSubstateIdLockIntent,
};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    component::{ComponentBody, ComponentHeader},
    fees::FeeReceipt,
    substate::{Substate, SubstateDiff},
};
use tari_transaction::{Transaction, TransactionId, VersionedSubstateId};

use crate::support::{committee_number_to_shard_group, helpers::random_substate_in_shard_group, TEST_NUM_PRESHARDS};

pub fn build_transaction_from(
    tx: Transaction,
    decision: Decision,
    fee: u64,
    resolved_inputs: Vec<VersionedSubstateIdLockIntent>,
    resulting_outputs: Vec<VersionedSubstateId>,
) -> TransactionRecord {
    let mut tx = TransactionRecord::new(tx);
    if decision.is_abort() {
        tx.set_current_decision_to_abort("Test aborted");
    }

    let execution = create_execution_result_for_transaction(
        // We're just building the execution here for DRY purposes, so genesis block id isn't used
        BlockId::zero(),
        *tx.id(),
        decision,
        fee,
        resolved_inputs,
        resulting_outputs.clone(),
    );

    tx.execution_result = Some(execution.result);
    tx.resulting_outputs = execution.resulting_outputs;
    tx.execution_time = Some(execution.execution_time);
    tx.resolved_inputs = Some(execution.resolved_inputs);
    tx
}

pub fn create_execution_result_for_transaction(
    block_id: BlockId,
    tx_id: TransactionId,
    decision: Decision,
    fee: u64,
    resolved_inputs: Vec<VersionedSubstateIdLockIntent>,
    resulting_outputs: Vec<VersionedSubstateId>,
) -> TransactionExecution {
    let result = if decision.is_commit() {
        let mut diff = SubstateDiff::new();
        for output in &resulting_outputs {
            let s = (0..100).map(|_| OsRng.sample(Alphanumeric) as char).collect::<String>();
            let random_state = tari_bor::to_value(&s).unwrap();
            diff.up(
                output.substate_id.clone(),
                Substate::new(output.version, ComponentHeader {
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
        TransactionResult::Reject(RejectReason::ExecutionFailure("Expected failure".to_string()))
    };

    TransactionExecution::new(
        block_id,
        tx_id,
        ExecuteResult {
            finalize: FinalizeResult::new(tx_id.into_array().into(), vec![], vec![], result, FeeReceipt {
                total_fee_payment: fee.try_into().unwrap(),
                total_fees_paid: fee.try_into().unwrap(),
                cost_breakdown: vec![],
            }),
        },
        resolved_inputs,
        resulting_outputs,
        Duration::from_secs(0),
    )
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
        .flat_map(|group_no| {
            iter::repeat_with(move || {
                random_substate_in_shard_group(
                    committee_number_to_shard_group(TEST_NUM_PRESHARDS, group_no, num_committees),
                    TEST_NUM_PRESHARDS,
                )
            })
            .take(total_num_outputs.div_ceil(num_committees as usize))
        })
        .collect::<Vec<_>>();

    build_transaction_from(tx, decision, fee, vec![], outputs)
}

pub fn change_decision(tx: ExecutedTransaction, new_decision: Decision) -> TransactionRecord {
    let total_fees_paid = tx
        .result()
        .finalize
        .fee_receipt
        .total_allocated_fee_payments()
        .as_u64_checked()
        .unwrap();
    let (tx, _, resolved_inputs, resulting_outputs) = tx.dissolve();
    build_transaction_from(tx, new_decision, total_fees_paid, resolved_inputs, resulting_outputs)
}
