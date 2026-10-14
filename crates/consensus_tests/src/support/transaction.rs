//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{iter, time::Duration};

use tari_common_types::types::PrivateKey;
use tari_consensus_types::Decision;
use tari_engine_types::{
    Epoch,
    ProtocolVersion,
    ValidatorFeeWithdrawal,
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    component::{Component, ComponentBody, ComponentHeader},
    fees::{FeeBreakdown, FeeReceipt, FeeSource},
    published_template::PublishedTemplate,
    substate::{Substate, SubstateDiff, SubstateId},
    transaction_receipt::{FinalizeOutcome, TransactionReceipt},
};
use tari_ootle_common_types::{InputDeclaration, LockIntent, SubstateVersion};
use tari_ootle_storage::consensus_models::{TransactionRecord, VersionedSubstateIdLockIntent};
use tari_ootle_transaction::{Network, Transaction, TransactionIntent, args};
use tari_template_lib_types::{SubstateOwnerRule, TransactionReceiptAddress};

use crate::support::{TEST_NUM_PRESHARDS, committee_number_to_shard_group, helpers::random_substate_in_shard_group};

/// WASM metering points attributed to every fabricated test execution — a typical "normal"
/// transaction. Sized so that even a block filled to `max_commands_in_block` stays within the
/// default `max_block_execution_points` budget: normal test transactions must never hit the budget.
/// `TestBuilder::start` asserts this invariant against the final test config.
pub const TEST_WASM_EXECUTION_POINTS: u64 = 5_000_000;

pub fn build_transaction_from(tx: Transaction) -> TransactionRecord {
    TransactionRecord::new(tx)
}

/// Fabricates a fully-paid fee receipt for a test execution. `fee` is the leader's share; a 5% burn share of
/// the payment sits on top of it, mirroring the shape the real executor produces.
fn create_test_fee_receipt(fee: u64) -> FeeReceipt {
    // 5% of the total paid is burned, so the leader's share is 95% of it. Scaling the leader fee
    // by 1/19 gives the burn that makes the total paid split exactly this way.
    let exhaust_burn = fee / 19;
    let paid = fee + exhaust_burn;
    let mut cost_breakdown = FeeBreakdown::default();
    cost_breakdown.add(FeeSource::WasmExecution, paid);
    FeeReceipt::builder()
        .with_total_fee_payment(paid)
        .with_total_fees_paid(paid)
        .with_total_fee_overcharge(0)
        .with_cost_breakdown(cost_breakdown)
        .with_exhaust_burn(exhaust_burn)
        .build()
}

#[allow(clippy::too_many_lines)]
pub fn create_execution_result_for_transaction(
    transaction: &Transaction,
    decision: Decision,
    fee: u64,
    resolved_inputs: &[VersionedSubstateIdLockIntent],
    resulting_outputs: &[VersionedSubstateIdLockIntent],
    validator_fee_withdrawals: Vec<ValidatorFeeWithdrawal>,
) -> ExecuteResult {
    let result = if decision.is_commit() {
        let mut diff = SubstateDiff::new();
        for input in resolved_inputs
            .iter()
            .filter(|input| input.lock_type().is_write() && !input.substate_id().is_validator_fee_pool())
        {
            diff.down(
                input.versioned_substate_id().substate_id().clone(),
                input.versioned_substate_id().version(),
            );
        }
        for output in resulting_outputs {
            if output.substate_id().is_transaction_receipt() {
                continue;
            }

            match output.substate_id() {
                SubstateId::Component(_) => {
                    // Generate consistent state for the component by simply using the ID
                    let state = tari_bor::to_value(output.versioned_substate_id()).unwrap();
                    diff.up(
                        output.versioned_substate_id().substate_id().clone(),
                        Substate::new(output.versioned_substate_id().version(), Component {
                            header: ComponentHeader {
                                template_address: Default::default(),
                                owner_rule: SubstateOwnerRule::None,
                                access_rules: Default::default(),
                                entity_id: output
                                    .versioned_substate_id()
                                    .substate_id()
                                    .as_component_address()
                                    .unwrap()
                                    .entity_id(),
                            },
                            body: ComponentBody { state },
                        }),
                    );
                },
                SubstateId::Template(_) => {
                    let binary = transaction
                        .instructions()
                        .iter()
                        .find_map(|i| i.published_template_binary_index())
                        .map(|idx| transaction.blobs().get(idx).expect("blob not found"))
                        .expect("No publish template instruction found in transaction");
                    diff.up(
                        output.versioned_substate_id().substate_id().clone(),
                        Substate::new(output.versioned_substate_id().version(), PublishedTemplate {
                            template_name: "test".try_into().expect("valid name"),
                            author: *transaction.seal_signature().public_key(),
                            binary: binary.to_vec().try_into().expect("Template binary too large"),
                            at_epoch: 0,
                            metadata_hash: None,
                        }),
                    );
                },
                _ => {
                    panic!(
                        "create_execution_result_for_transaction: Test harness only supports generating component and \
                         template outputs. Got {output}"
                    );
                },
            }
        }
        // We MUST create the transaction receipt
        diff.up(
            SubstateId::TransactionReceipt(TransactionReceiptAddress::from(transaction.calculate_id())),
            Substate::new(SubstateVersion::ZERO, TransactionReceipt {
                outcome: FinalizeOutcome::Commit,
                diff_summary: Default::default(),
                fee_withdrawals: Default::default(),
                events: Default::default(),
                fee_receipt: create_test_fee_receipt(fee),
                epoch: Epoch::zero(),
                intent_commitment: transaction.calculate_intent_commitment(),
            }),
        );

        diff.set_once_fee_withdrawals(validator_fee_withdrawals);

        TransactionResult::Accept(diff)
    } else {
        TransactionResult::Reject(RejectReason::execution_failure_unclassified(
            "Transaction was set to ABORT in test",
        ))
    };

    ExecuteResult {
        finalize: FinalizeResult::new(
            transaction.calculate_id().into_array().into(),
            vec![],
            vec![],
            result,
            create_test_fee_receipt(fee),
        ),
        execution_time: Duration::from_secs(0),
        execute_epoch: None,
        wasm_execution_points: TEST_WASM_EXECUTION_POINTS,
        native_execution_points: 0,
    }
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

/// Transactions stay valid through the first V2 epoch, so a test can commit them under every protocol version.
const MAX_EPOCH: Epoch = ProtocolVersion::activation_epoch(Network::LocalNet, ProtocolVersion::V2).unwrap();

pub fn build_transaction(inputs: Vec<InputDeclaration>) -> TransactionRecord {
    let k = PrivateKey::default();
    let tx = Transaction::builder_localnet(MAX_EPOCH)
        .call_function(Default::default(), "foo", args![])
        .with_inputs(inputs)
        .build_and_seal(&k);
    TransactionRecord::new(tx)
}
