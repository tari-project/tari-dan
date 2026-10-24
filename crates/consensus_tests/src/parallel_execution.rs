//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A committee whose validators speculate executions on different numbers of threads. Every block one of them proposes
//! must be reproduced by the others, so a result that depends on the number of threads stalls the chain.

use tari_common_types::types::PrivateKey;
use tari_consensus_types::Decision;
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{Epoch, InputDeclaration, NodeHeight, SubstateLockType};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{SubstateRecord, TransactionRecord},
};
use tari_ootle_transaction::Transaction;

use crate::support::{ExecuteSpec, Test, TestAddress, TestVnDestination, logging::setup_logger};

const SPECULATING: [&str; 2] = ["3", "4"];

/// A transaction that writes `inputs`, sealed with `key` so that transactions with the same inputs differ.
fn writing(inputs: &[&SubstateId], key: u64) -> TransactionRecord {
    let mut builder = Transaction::builder_localnet(Epoch(1));
    for input in inputs {
        builder = builder.add_input(InputDeclaration::write((*input).clone()));
    }
    TransactionRecord::new(builder.build_and_seal(&PrivateKey::from(key)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn validators_speculating_on_different_numbers_of_threads_agree_on_every_block() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2", "3", "4"])
        .with_execution_threads(&SPECULATING, 4)
        .start()
        .await;

    let substates = test.create_substates_on_vns(TestVnDestination::All, 9);
    let (vault, accounts) = substates.split_first().unwrap();
    let vault = vault.substate_id();
    // Transfers between disjoint accounts, then a faucet's withdrawals, which all write one vault
    let disjoint = accounts
        .iter()
        .zip(1..)
        .map(|(account, key)| writing(&[account.substate_id()], key))
        .collect::<Vec<_>>();
    let faucet = (100..106).map(|key| writing(&[vault], key)).collect::<Vec<_>>();

    for transaction in disjoint.iter().chain(&faucet) {
        test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
            transaction: transaction.transaction().clone(),
            decision: Decision::Commit,
            fee: 1,
            input_locks: transaction
                .transaction()
                .all_inputs_substate_ids_iter()
                .map(|id| (id.clone(), SubstateLockType::Write))
                .collect(),
            new_outputs: vec![],
            validator_fee_withdrawals: vec![],
            calls_template: None,
        });
        test.send_transaction_to_destination(TestVnDestination::All, transaction.clone())
            .await;
    }
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;
        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height > NodeHeight(40) {
            test.dump_pool_info();
            panic!("Not all transactions committed after {} blocks", leaf.height);
        }
    }
    test.stop();

    for transaction in disjoint.iter().chain(&faucet) {
        test.assert_all_validators_have_decision(transaction.id(), Decision::Commit)
            .await;
    }

    let vault_versions = test
        .validators_iter()
        .map(|vn| {
            vn.state_store
                .with_read_tx(|tx| SubstateRecord::get_latest(tx, vault))
                .unwrap()
                .version()
        })
        .collect::<Vec<_>>();
    assert!(
        vault_versions.windows(2).all(|pair| pair[0] == pair[1]),
        "validators hold different vault versions: {vault_versions:?}"
    );

    for vn in test.validators_iter() {
        // A speculated execution that the block takes is the only execution of its transaction
        for transaction in &disjoint {
            assert_eq!(
                vn.transaction_executor.get_execute_count(transaction.id()),
                1,
                "{} executed {} more than once",
                vn.address,
                transaction.id()
            );
        }
        let num_speculated = vn.transaction_executor.get_speculation_thread_execution_count();
        if SPECULATING.contains(&vn.address.as_str()) {
            assert!(num_speculated > 0, "{} never speculated", vn.address);
        } else {
            assert_eq!(num_speculated, 0, "{} speculated on one thread", vn.address);
        }
    }

    test.assert_clean_shutdown().await;
}
