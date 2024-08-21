//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{iter, time::Duration};

use rand::{distributions::Alphanumeric, rngs::OsRng, Rng};
use tari_common_types::types::PrivateKey;
use tari_dan_storage::consensus_models::{
    Decision,
    ExecutedTransaction,
    SubstateLockType,
    TransactionExecution,
    TransactionRecord,
    VersionedSubstateIdLockIntent,
};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    component::{ComponentBody, ComponentHeader},
    fees::FeeReceipt,
    substate::{Substate, SubstateDiff, SubstateId},
    transaction_receipt::{TransactionReceipt, TransactionReceiptAddress},
};
use tari_transaction::{Transaction, TransactionId, VersionedSubstateId};

use crate::support::{committee_number_to_shard_group, helpers::random_substate_in_shard_group, TEST_NUM_PRESHARDS};

pub fn build_transaction_from(
    tx: Transaction,
    decision: Decision,
    fee: u64,
    resolved_inputs: Vec<VersionedSubstateIdLockIntent>,
    resulting_outputs: Vec<VersionedSubstateIdLockIntent>,
) -> TransactionRecord {
    let mut tx = TransactionRecord::new(tx);
    if decision.is_abort() {
        tx.set_abort_reason(RejectReason::ExecutionFailure("Test aborted".to_string()));
    }

    let execution =
        create_execution_result_for_transaction(*tx.id(), decision, fee, resolved_inputs, resulting_outputs.clone());

    tx.execution_result = Some(execution.result);
    tx.resulting_outputs = Some(execution.resulting_outputs);
    tx.resolved_inputs = Some(execution.resolved_inputs);
    tx
}

pub fn create_execution_result_for_transaction(
    tx_id: TransactionId,
    decision: Decision,
    fee: u64,
    resolved_inputs: Vec<VersionedSubstateIdLockIntent>,
    mut resulting_outputs: Vec<VersionedSubstateIdLockIntent>,
) -> TransactionExecution {
    let result = if decision.is_commit() {
        let mut diff = SubstateDiff::new();
        for output in &resulting_outputs {
            if output.substate_id().is_transaction_receipt() {
                continue;
            }
            assert!(
                output.substate_id().is_component(),
                "create_execution_result_for_transaction: Test harness only supports generating component outputs. \
                 Got {output}"
            );

            let s = (0..100).map(|_| OsRng.sample(Alphanumeric) as char).collect::<String>();
            let random_state = tari_bor::to_value(&s).unwrap();
            diff.up(
                output.versioned_substate_id().substate_id.clone(),
                Substate::new(output.versioned_substate_id().version, ComponentHeader {
                    template_address: Default::default(),
                    module_name: "Test".to_string(),
                    owner_key: Default::default(),
                    owner_rule: Default::default(),
                    access_rules: Default::default(),
                    entity_id: output
                        .versioned_substate_id()
                        .substate_id
                        .as_component_address()
                        .unwrap()
                        .entity_id(),
                    body: ComponentBody { state: random_state },
                }),
            )
        }
        // We MUST create the transaction receipt
        diff.up(
            SubstateId::TransactionReceipt(TransactionReceiptAddress::from(tx_id)),
            Substate::new(0, TransactionReceipt {
                transaction_hash: tx_id.into_array().into(),
                events: vec![],
                logs: vec![],
                fee_receipt: FeeReceipt {
                    total_fee_payment: fee.try_into().unwrap(),
                    total_fees_paid: fee.try_into().unwrap(),
                    cost_breakdown: vec![],
                },
            }),
        );
        TransactionResult::Accept(diff)
    } else {
        TransactionResult::Reject(RejectReason::ExecutionFailure(
            "Transaction was set to ABORT in test".to_string(),
        ))
    };

    resulting_outputs.push(VersionedSubstateIdLockIntent::new(
        VersionedSubstateId::new(
            SubstateId::TransactionReceipt(TransactionReceiptAddress::from(tx_id)),
            0,
        ),
        SubstateLockType::Output,
    ));

    TransactionExecution::new(
        tx_id,
        ExecuteResult {
            finalize: FinalizeResult::new(tx_id.into_array().into(), vec![], vec![], result, FeeReceipt {
                total_fee_payment: fee.try_into().unwrap(),
                total_fees_paid: fee.try_into().unwrap(),
                cost_breakdown: vec![],
            }),
            execution_time: Duration::from_secs(0),
        },
        resolved_inputs,
        resulting_outputs,
        None,
    )
}

pub fn build_random_outputs(total_num_outputs: usize, num_committees: u32) -> Vec<VersionedSubstateIdLockIntent> {
    // We create these outputs so that the test VNs dont have to have any UP substates
    // Equal potion of shards to each committee
    (0..num_committees)
        .flat_map(|group_no| {
            random_substates_ids_for_committee_generator(group_no, num_committees).take(total_num_outputs)
        })
        .map(VersionedSubstateIdLockIntent::output)
        .collect::<Vec<_>>()
}

pub fn random_substates_ids_for_committee_generator(
    committee_no: u32,
    num_committees: u32,
) -> impl Iterator<Item = VersionedSubstateId> {
    iter::repeat_with(move || {
        random_substate_in_shard_group(
            committee_number_to_shard_group(TEST_NUM_PRESHARDS, committee_no, num_committees),
            TEST_NUM_PRESHARDS,
        )
    })
}

pub fn build_transaction_with_inputs_and_outputs(
    decision: Decision,
    fee: u64,
    inputs: Vec<VersionedSubstateIdLockIntent>,
    outputs: Vec<VersionedSubstateIdLockIntent>,
) -> TransactionRecord {
    let k = PrivateKey::default();
    let tx = Transaction::builder()
        .with_inputs(inputs.iter().map(|i| i.versioned_substate_id().clone().into()))
        .sign(&k)
        .build();

    build_transaction_from(tx, decision, fee, inputs, outputs)
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
