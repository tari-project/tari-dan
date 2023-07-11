//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use digest::Digest;
use tari_crypto::hash::blake2::Blake256;
use tari_dan_engine::runtime::ConsensusContext;
use tari_engine_types::{commit_result::ExecuteResult, instruction::Instruction};
use tari_template_lib::{
    args,
    prelude::{Amount, ComponentAddress, NonFungibleAddress, TemplateAddress, CONFIDENTIAL_TARI_RESOURCE_ADDRESS},
    Hash,
};
use tari_template_test_tooling::TemplateTest;

#[derive(Clone)]
struct User {
    account_address: ComponentAddress,
    owner_token: NonFungibleAddress,
}

struct AtomicSwapTest {
    template_test: TemplateTest,
    atomic_swap_template: TemplateAddress,
    alice: User,
    bob: User,
    preimage: [u8; 32],
    hashlock: Hash,
    amount: Amount,
}

fn setup() -> AtomicSwapTest {
    let mut template_test = TemplateTest::new(vec!["../template_builtin/templates/atomic_swap/"]);
    let atomic_swap_template = template_test.get_template_address("HashedTimelock");

    // Create Alice and Bob accounts
    let (alice_account, alice_token, _) = template_test.create_owned_account();
    let alice = User {
        account_address: alice_account,
        owner_token: alice_token,
    };
    let (bob_account, bob_token, _) = template_test.create_owned_account();
    let bob = User {
        account_address: bob_account,
        owner_token: bob_token,
    };

    // Default values for the contracts
    let amount = Amount(100);
    let preimage = [0u8; 32];
    let hashlock = create_hashlock(preimage);

    AtomicSwapTest {
        template_test,
        atomic_swap_template,
        alice,
        bob,
        preimage,
        hashlock,
        amount,
    }
}

fn create_hashlock(preimage: [u8; 32]) -> Hash {
    let hashlock: [u8; 32] = Blake256::new().chain(preimage).finalize().into();
    hashlock.into()
}

fn create_lock_contract(test: &mut AtomicSwapTest, sender: User, receiver: User, timelock: u64) -> ComponentAddress {
    let result = test
        .template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: sender.account_address,
                    method: "withdraw".to_string(),
                    args: args![*CONFIDENTIAL_TARI_RESOURCE_ADDRESS, test.amount],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"bucket".to_vec(),
                },
                Instruction::CallFunction {
                    template_address: test.atomic_swap_template,
                    function: "create".to_string(),
                    args: args![
                        Variable("bucket"),
                        sender.owner_token,
                        receiver.owner_token,
                        test.hashlock,
                        timelock
                    ],
                },
            ],
            // Sender proof needed to withdraw
            vec![sender.owner_token],
        )
        .unwrap();
    result.finalize.execution_results[2].decode().unwrap()
}

fn withdraw_funds(
    test: &mut AtomicSwapTest,
    contract: ComponentAddress,
    preimage: [u8; 32],
    user: User,
) -> Result<ExecuteResult, anyhow::Error> {
    test.template_test.execute_and_commit(
        vec![
            Instruction::CallMethod {
                component_address: contract,
                method: "withdraw".to_string(),
                args: args![preimage],
            },
            Instruction::PutLastInstructionOutputOnWorkspace {
                key: b"bucket".to_vec(),
            },
            Instruction::CallMethod {
                component_address: user.account_address,
                method: "deposit".to_string(),
                args: args![Variable("bucket")],
            },
        ],
        // Sender proof needed to withdraw
        vec![user.owner_token],
    )
}

fn refund(test: &mut AtomicSwapTest, contract: ComponentAddress, user: User) -> Result<ExecuteResult, anyhow::Error> {
    test.template_test.execute_and_commit(
        vec![
            Instruction::CallMethod {
                component_address: contract,
                method: "refund".to_string(),
                args: args![],
            },
            Instruction::PutLastInstructionOutputOnWorkspace {
                key: b"bucket".to_vec(),
            },
            Instruction::CallMethod {
                component_address: user.account_address,
                method: "deposit".to_string(),
                args: args![Variable("bucket")],
            },
        ],
        // Sender proof needed to refund
        vec![user.owner_token],
    )
}

// This test simulates an atomic swap between two accounts inside the Tari network.
// Obviously atomic swaps inside the same network does not have any real world utility,
// but for testing purposes it's useful as it allow us to verify the Tari atomic swap template
#[test]
fn successful_swap() {
    let mut test = setup();
    let alice = test.alice.clone();
    let bob = test.bob.clone();
    let preimage = test.preimage;

    // Alice will start the atomic swap by locking her funds first
    let timelock_c1 = 10u64;
    let contract_1_component = create_lock_contract(&mut test, alice.clone(), bob.clone(), timelock_c1);

    // Then Bob will lock his funds
    // Note that the timelock MUST be lower than the previous timelock to give time to Bob to withdraw funds later
    let timelock_c2 = 5u64;
    let contract_2_component = create_lock_contract(&mut test, bob.clone(), alice.clone(), timelock_c2);

    // Alice withdraws the funds from Bob's contract, revealing the preimage in the process
    withdraw_funds(&mut test, contract_2_component, preimage, alice).unwrap();

    // Bob now knows the preimage, so he can withdraw funds from Alice's contract
    withdraw_funds(&mut test, contract_1_component, preimage, bob).unwrap();
}

#[test]
fn alice_can_refund() {
    let mut test = setup();
    let alice = test.alice.clone();
    let bob = test.bob.clone();

    // Alice will start the atomic swap by locking her funds first
    let timelock_c1 = 10u64;
    let contract_1_component = create_lock_contract(&mut test, alice.clone(), bob, timelock_c1);

    // Bob never publishes his locking contract
    // So Alice needs to wait until after the timelock in her conctract to retrieve her funds
    test.template_test.set_consensus_context(ConsensusContext {
        current_epoch: timelock_c1 + 1,
    });
    refund(&mut test, contract_1_component, alice).unwrap();
}

#[test]
fn bob_can_refund() {
    let mut test = setup();
    let alice = test.alice.clone();
    let bob = test.bob.clone();

    // Alice will start the atomic swap by locking her funds first
    let timelock_c1 = 10u64;
    let _contract_1_component = create_lock_contract(&mut test, alice.clone(), bob.clone(), timelock_c1);

    // Then Bob will lock his funds
    // Note that the timelock MUST be lower than the previous timelock to give time to Bob to withdraw funds later
    let timelock_c2 = 5u64;
    let contract_2_component = create_lock_contract(&mut test, bob.clone(), alice, timelock_c2);

    // Alice never withdraws funds from Bob's contract, so Bob will never know the preimage and cannot complete the swap
    // So Bob needs to wait until after the timelock in his conctract to retrieve his funds
    test.template_test.set_consensus_context(ConsensusContext {
        current_epoch: timelock_c2 + 1,
    });
    refund(&mut test, contract_2_component, bob).unwrap();
}

#[test]
fn refunds_cannot_be_done_before_timelock() {
    let mut test = setup();
    let alice = test.alice.clone();
    let bob = test.bob.clone();

    // Alice will start the atomic swap by locking her funds first
    let timelock_c1 = 10u64;
    let contract_1_component = create_lock_contract(&mut test, alice.clone(), bob.clone(), timelock_c1);

    // Then Bob will lock his funds
    // Note that the timelock MUST be lower than the previous timelock to give time to Bob to withdraw funds later
    let timelock_c2 = 5u64;
    let contract_2_component = create_lock_contract(&mut test, bob.clone(), alice.clone(), timelock_c2);

    // Bob should not be able to refund if his timelock has not expired
    test.template_test.set_consensus_context(ConsensusContext {
        current_epoch: timelock_c2,
    });
    let err = refund(&mut test, contract_2_component, bob).unwrap_err();
    assert!(err.to_string().contains("Timelock not yet passed"));

    // Alice should not be able to refund if her timelock has not expired
    test.template_test.set_consensus_context(ConsensusContext {
        current_epoch: timelock_c1,
    });
    let err = refund(&mut test, contract_1_component, alice).unwrap_err();
    assert!(err.to_string().contains("Timelock not yet passed"));
}

#[test]
fn it_does_not_allow_withdrawals_with_invalid_preimage() {
    let mut test = setup();
    let alice = test.alice.clone();
    let bob = test.bob.clone();
    let preimage = test.preimage;

    // Alice will start the atomic swap by locking her funds first
    let timelock_c1 = 10u64;
    let _contract_1_component = create_lock_contract(&mut test, alice.clone(), bob.clone(), timelock_c1);

    // Then Bob will lock his funds
    // Note that the timelock MUST be lower than the previous timelock to give time to Bob to withdraw funds later
    let timelock_c2 = 5u64;
    let contract_2_component = create_lock_contract(&mut test, bob, alice.clone(), timelock_c2);

    // Alice cannot withdraw from Bob's contract with an invalid preimage
    let invalid_preimage = [1u8; 32];
    assert_ne!(invalid_preimage, preimage);
    let err = withdraw_funds(&mut test, contract_2_component, invalid_preimage, alice).unwrap_err();
    assert!(err.to_string().contains("Invalid preimage"));
}

#[test]
fn it_does_not_allow_withdrawals_from_undesignated_users() {
    let mut test = setup();
    let alice = test.alice.clone();
    let bob = test.bob.clone();
    let preimage = test.preimage;

    // Alice will start the atomic swap by locking her funds first
    let timelock_c1 = 10u64;
    let contract_1_component = create_lock_contract(&mut test, alice.clone(), bob, timelock_c1);

    // No one other than Bob can withdraw even if providing a valid preimage
    let err = withdraw_funds(&mut test, contract_1_component, preimage, alice).unwrap_err();
    assert!(err.to_string().contains("Access Denied"));
}

#[test]
fn it_does_not_allow_refunds_from_undesignated_users() {
    let mut test = setup();
    let alice = test.alice.clone();
    let bob = test.bob.clone();

    // Alice will start the atomic swap by locking her funds first
    let timelock_c1 = 10u64;
    let contract_1_component = create_lock_contract(&mut test, alice, bob.clone(), timelock_c1);

    // No one other than Alice can refund after the timelock
    test.template_test.set_consensus_context(ConsensusContext {
        current_epoch: timelock_c1 + 1,
    });
    let err = refund(&mut test, contract_1_component, bob).unwrap_err();
    assert!(err.to_string().contains("Access Denied"));
}
