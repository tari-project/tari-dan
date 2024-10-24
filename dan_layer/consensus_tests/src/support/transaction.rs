//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{iter, time::Duration};

use tari_common_types::types::PrivateKey;
use tari_dan_common_types::SubstateRequirement;
use tari_dan_storage::consensus_models::{Decision, TransactionRecord, VersionedSubstateIdLockIntent};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    component::{ComponentBody, ComponentHeader},
    fees::{FeeBreakdown, FeeReceipt},
    substate::{Substate, SubstateDiff, SubstateId},
    transaction_receipt::{TransactionReceipt, TransactionReceiptAddress},
};
use tari_template_lib::args;
use tari_transaction::Transaction;

use crate::support::{committee_number_to_shard_group, helpers::random_substate_in_shard_group, TEST_NUM_PRESHARDS};

pub fn build_transaction_from(tx: Transaction, decision: Decision) -> TransactionRecord {
    let mut tx = TransactionRecord::new(tx);
    if decision.is_abort() {
        tx.set_abort_reason(RejectReason::ExecutionFailure("Test aborted".to_string()));
    }
    tx
}

pub fn create_execution_result_for_transaction(
    transaction: &Transaction,
    decision: Decision,
    fee: u64,
    resolved_inputs: &[VersionedSubstateIdLockIntent],
    resulting_outputs: &[VersionedSubstateIdLockIntent],
) -> ExecuteResult {
    let result = if decision.is_commit() {
        let mut diff = SubstateDiff::new();
        for input in resolved_inputs.iter().filter(|input| input.lock_type().is_write()) {
            diff.down(
                input.versioned_substate_id().substate_id().clone(),
                input.versioned_substate_id().version(),
            );
        }
        for output in resulting_outputs {
            if output.substate_id().is_transaction_receipt() {
                continue;
            }
            assert!(
                output.substate_id().is_component(),
                "create_execution_result_for_transaction: Test harness only supports generating component outputs. \
                 Got {output}"
            );

            // Generate consistent state for the component by simply using the ID
            let state = tari_bor::to_value(output.versioned_substate_id()).unwrap();
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
                    body: ComponentBody { state },
                }),
            )
        }
        // We MUST create the transaction receipt
        diff.up(
            SubstateId::TransactionReceipt(TransactionReceiptAddress::from(*transaction.id())),
            Substate::new(0, TransactionReceipt {
                transaction_hash: transaction.id().into_array().into(),
                events: vec![],
                logs: vec![],
                fee_receipt: FeeReceipt {
                    total_fee_payment: fee.try_into().unwrap(),
                    total_fees_paid: fee.try_into().unwrap(),
                    cost_breakdown: FeeBreakdown::default(),
                },
            }),
        );

        TransactionResult::Accept(diff)
    } else {
        TransactionResult::Reject(RejectReason::ExecutionFailure(
            "Transaction was set to ABORT in test".to_string(),
        ))
    };

    let result = ExecuteResult {
        finalize: FinalizeResult::new(
            transaction.id().into_array().into(),
            vec![],
            vec![],
            result,
            FeeReceipt {
                total_fee_payment: fee.try_into().unwrap(),
                total_fees_paid: fee.try_into().unwrap(),
                cost_breakdown: FeeBreakdown::default(),
            },
        ),
        execution_time: Duration::from_secs(0),
    };

    result
}

pub fn build_substate_id_for_committee(committee_no: u32, num_committees: u32) -> SubstateId {
    random_substates_ids_for_committee_generator(committee_no, num_committees)
        .next()
        .unwrap()
}

pub fn random_substates_ids_for_committee_generator(
    committee_no: u32,
    num_committees: u32,
) -> impl Iterator<Item = SubstateId> {
    iter::repeat_with(move || {
        random_substate_in_shard_group(
            committee_number_to_shard_group(TEST_NUM_PRESHARDS, committee_no, num_committees),
            TEST_NUM_PRESHARDS,
        )
    })
}

pub fn build_transaction(decision: Decision, inputs: Vec<SubstateRequirement>) -> TransactionRecord {
    let k = PrivateKey::default();
    let tx = Transaction::builder()
        .call_function(Default::default(), "foo", args![])
        .with_inputs(inputs)
        .sign(&k)
        .build();
    let mut tx = TransactionRecord::new(tx);
    if decision.is_abort() {
        tx.set_abort_reason(RejectReason::ExecutionFailure("Test aborted".to_string()));
    }
    tx
}
