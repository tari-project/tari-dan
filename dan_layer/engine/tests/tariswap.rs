//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
    prelude::{NonFungibleAddress, ResourceAddress},
};
use tari_template_test_tooling::{SubstateType, TemplateTest};

// comes from the faucet template code
const INITIAL_BALANCE: Amount = Amount::new(1000);

struct TariSwapTest {
    template_test: TemplateTest,
    a_resource: ResourceAddress,
    b_resource: ResourceAddress,
    tariswap: ComponentAddress,
    account_address: ComponentAddress,
    account_proof: NonFungibleAddress,
}

fn setup() -> TariSwapTest {
    let mut template_test = TemplateTest::new(vec!["tests/templates/tariswap", "tests/templates/faucet"]);

    // create the token pair for the swap pool
    let (a_faucet, a_resource) = create_faucet_component(&mut template_test, "A".to_string());
    let (b_faucet, b_resource) = create_faucet_component(&mut template_test, "B".to_string());

    let tariswap = create_tariswap_component(&mut template_test, a_faucet, b_faucet);

    let (account_address, account_proof, _) = template_test.create_owned_account();
    fund_account(&mut template_test, account_address, a_faucet);
    fund_account(&mut template_test, account_address, b_faucet);

    TariSwapTest {
        template_test,
        a_resource,
        b_resource,
        tariswap,
        account_address,
        account_proof,
    }
}

fn create_faucet_component(template_test: &mut TemplateTest, symbol: String) -> (ComponentAddress, ResourceAddress) {
    let initial_supply = Amount(1_000_000_000_000);
    let component_address: ComponentAddress =
        template_test.call_function("TestFaucet", "mint_with_symbol", args![initial_supply, symbol], vec![]);

    let resource_address = template_test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();

    (component_address, resource_address)
}

fn create_tariswap_component(
    template_test: &mut TemplateTest,
    a_faucet: ComponentAddress,
    b_faucet: ComponentAddress,
) -> ComponentAddress {
    let module_name = "TariSwapPool";
    let tariswap_template = template_test.get_template_address(module_name);

    let res = template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: a_faucet,
                    method: "take_free_coins".to_string(),
                    args: args![],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"a_bucket".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: b_faucet,
                    method: "take_free_coins".to_string(),
                    args: args![],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"b_bucket".to_vec(),
                },
                Instruction::CallFunction {
                    template_address: tariswap_template,
                    function: "new".to_string(),
                    args: args![Variable("a_bucket"), Variable("b_bucket")],
                },
            ],
            vec![],
        )
        .unwrap();

    let (substate_addr, _) = res
        .expect_success()
        .up_iter()
        .find(|(address, substate)| {
            address.is_component() && substate.substate_value().component().unwrap().module_name == module_name
        })
        .unwrap();

    substate_addr.as_component_address().unwrap()
}

fn fund_account(
    template_test: &mut TemplateTest,
    account_address: ComponentAddress,
    faucet_component: ComponentAddress,
) {
    template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: faucet_component,
                    method: "take_free_coins".to_string(),
                    args: args![],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"free_coins".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "deposit".to_string(),
                    args: args![Variable("free_coins")],
                },
            ],
            vec![],
        )
        .unwrap();
}

fn do_swap(
    test: &mut TariSwapTest,
    input_resource: &ResourceAddress,
    output_resource: &ResourceAddress,
    amount: Amount,
) {
    test.template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: test.account_address,
                    method: "withdraw".to_string(),
                    args: args![input_resource, amount],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"input_bucket".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: test.tariswap,
                    method: "swap".to_string(),
                    args: args![Variable("input_bucket"), output_resource],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"output_bucket".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: test.account_address,
                    method: "deposit".to_string(),
                    args: args![Variable("output_bucket"),],
                },
            ],
            // proof needed to withdraw
            vec![test.account_proof.clone()],
        )
        .unwrap();
}

fn get_pool_balance(test: &mut TariSwapTest, resource_address: ResourceAddress) -> Amount {
    test.template_test
        .call_method(test.tariswap, "get_pool_balance", args![resource_address], vec![])
}

fn get_account_balance(test: &mut TariSwapTest, resource_address: ResourceAddress) -> Amount {
    test.template_test
        .call_method(test.account_address, "balance", args![resource_address], vec![])
}

#[test]
fn swap() {
    // init the test
    let mut test = setup();
    let a_resource = test.a_resource;
    let b_resource = test.b_resource;

    // check initial balances
    let mut account_a_balance = get_account_balance(&mut test, a_resource);
    let mut account_b_balance = get_account_balance(&mut test, a_resource);
    assert_eq!(account_a_balance, INITIAL_BALANCE);
    assert_eq!(account_b_balance, INITIAL_BALANCE);

    let mut pool_a_balance = get_pool_balance(&mut test, a_resource);
    let mut pool_b_balance = get_pool_balance(&mut test, b_resource);
    assert_eq!(pool_a_balance, INITIAL_BALANCE);
    assert_eq!(pool_b_balance, INITIAL_BALANCE);

    // ------- FIRST SWAP -------
    // do a swap, giving 100 "A" tokens for "B" tokens
    let input_amount = Amount::new(100);
    let expected_output_amount = Amount::new(91); // following the constant product formula: b = k / a
    do_swap(&mut test, &a_resource, &b_resource, input_amount);

    // check that the new pool balances are expected
    let new_pool_a_balance = get_pool_balance(&mut test, a_resource);
    let new_pool_b_balance = get_pool_balance(&mut test, b_resource);
    assert_eq!(new_pool_a_balance, pool_a_balance + input_amount);
    assert_eq!(new_pool_b_balance, pool_b_balance - expected_output_amount);
    pool_a_balance = new_pool_a_balance;
    pool_b_balance = new_pool_b_balance;

    // check that the new account balances are expected
    let new_account_a_balance = get_account_balance(&mut test, a_resource);
    let new_account_b_balance = get_account_balance(&mut test, b_resource);
    assert_eq!(new_account_a_balance, account_a_balance - input_amount);
    assert_eq!(new_account_b_balance, account_b_balance + expected_output_amount);
    account_a_balance = new_account_a_balance;
    account_b_balance = new_account_b_balance;

    // ------- SECOND SWAP -------
    // do another swap
    // this time we are providing "B" tokens which are more scarce now, so we receive a more of "A" tokens in return
    let input_amount = Amount::new(100);
    let expected_output_amount = Amount::new(110); // following the constant product formula: b = k / a
    do_swap(&mut test, &b_resource, &a_resource, input_amount);
    assert_eq!(
        get_pool_balance(&mut test, a_resource),
        pool_a_balance - expected_output_amount
    );
    assert_eq!(get_pool_balance(&mut test, b_resource), pool_b_balance + input_amount);
    assert_eq!(
        get_account_balance(&mut test, a_resource),
        account_a_balance + expected_output_amount
    );
    assert_eq!(
        get_account_balance(&mut test, b_resource),
        account_b_balance - input_amount
    );
}
