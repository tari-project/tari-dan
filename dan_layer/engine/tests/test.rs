//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::mem::size_of;

use tari_bor::{borsh, Decode, Encode};
use tari_dan_engine::{
    packager::{PackageError, TemplateModuleLoader},
    transaction::TransactionError,
    wasm::{compile::compile_template, WasmExecutionError},
};
use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
};
use tari_template_test_tooling::{SubstateType, TemplateTest};

#[test]
fn test_hello_world() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/hello_world"]);
    let result: String = template_test.call_function("HelloWorld", "greet", args![]);

    assert_eq!(result, "Hello World!");
}

#[test]
fn test_state() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/state"]);
    let store = template_test.read_only_state_store();

    // constructor
    let component_address1: ComponentAddress = template_test.call_function("State", "new", args![]);
    template_test.assert_calls(&[
        "set_current_runtime_state",
        "emit_log",
        "component_invoke",
        "set_last_instruction_output",
        "finalize",
    ]);
    template_test.clear_calls();

    let component_address2: ComponentAddress = template_test.call_function("State", "new", args![]);
    assert_ne!(component_address1, component_address2);

    let component = store.get_component(component_address1).unwrap();
    assert_eq!(component.module_name, "State");

    let component = store.get_component(component_address2).unwrap();
    assert_eq!(component.module_name, "State");

    // call the "set" method to update the instance value
    let new_value = 20_u32;
    template_test.call_method::<()>(component_address2, "set", args![new_value]);

    // call the "get" method to get the current value
    let value: u32 = template_test.call_method(component_address2, "get", args![]);

    assert_eq!(value, new_value);
}

#[test]
fn test_composed() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/state", "tests/templates/hello_world"]);

    let functions = template_test
        .get_module("HelloWorld")
        .template_def()
        .functions
        .iter()
        .map(|f| f.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(functions, vec!["greet", "new", "custom_greeting"]);

    let functions = template_test
        .get_module("State")
        .template_def()
        .functions
        .iter()
        .map(|f| f.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(functions, vec!["new", "set", "get"]);

    let component_state: ComponentAddress = template_test.call_function("State", "new", args![]);
    let component_hw: ComponentAddress = template_test.call_function("HelloWorld", "new", args!["أهلا"]);

    let result: String = template_test.call_method(component_hw, "custom_greeting", args!["Wasm"]);
    assert_eq!(result, "أهلا Wasm!");

    // call the "set" method to update the instance value
    let new_value = 20_u32;
    template_test.call_method::<()>(component_state, "set", args![new_value]);

    // call the "get" method to get the current value
    let value: u32 = template_test.call_method(component_state, "get", args![]);

    assert_eq!(value, new_value);
}

#[test]
fn test_dodgy_template() {
    let err = compile_template("tests/templates/buggy", &["call_engine_in_abi"])
        .unwrap()
        .load_template()
        .unwrap_err();
    assert!(matches!(err, PackageError::TemplateCalledEngineDuringInitialization));

    let err = compile_template("tests/templates/buggy", &["return_null_abi"])
        .unwrap()
        .load_template()
        .unwrap_err();
    assert!(matches!(
        err,
        PackageError::WasmModuleError(WasmExecutionError::AbiDecodeError)
    ));

    let err = compile_template("tests/templates/buggy", &["unexpected_export_function"])
        .unwrap()
        .load_template()
        .unwrap_err();
    assert!(matches!(
        err,
        PackageError::WasmModuleError(WasmExecutionError::UnexpectedAbiFunction { .. })
    ));
}

#[test]
fn test_account() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/faucet"]);

    let faucet_template = template_test.get_template_address("TestFaucet");

    let initial_supply = Amount(1_000_000_000_000);
    let result = template_test
        .execute_and_commit(vec![Instruction::CallFunction {
            template_address: faucet_template,
            function: "mint".to_string(),
            args: args![initial_supply],
        }])
        .unwrap();
    let faucet_component: ComponentAddress = result.execution_results[0].decode().unwrap();
    let faucet_resource = result
        .result
        .expect("Faucet mint failed")
        .up_iter()
        .find_map(|(_, s)| s.substate_address().as_resource_address())
        .unwrap();

    // Create sender and receiver accounts
    let sender_address: ComponentAddress = template_test.call_function("Account", "new", args![]);
    let receiver_address: ComponentAddress = template_test.call_function("Account", "new", args![]);

    let _result = template_test
        .execute_and_commit(vec![
            Instruction::CallMethod {
                component_address: faucet_component,
                method: "take_free_coins".to_string(),
                args: args![],
            },
            Instruction::PutLastInstructionOutputOnWorkspace {
                key: b"free_coins".to_vec(),
            },
            Instruction::CallMethod {
                component_address: sender_address,
                method: "deposit".to_string(),
                args: args![Variable("free_coins")],
            },
        ])
        .unwrap();

    let result = template_test
        .execute_and_commit(vec![
            Instruction::CallMethod {
                component_address: sender_address,
                method: "withdraw".to_string(),
                args: args![faucet_resource, Amount(100)],
            },
            Instruction::PutLastInstructionOutputOnWorkspace {
                key: b"foo_bucket".to_vec(),
            },
            Instruction::CallMethod {
                component_address: receiver_address,
                method: "deposit".to_string(),
                args: args![Variable("foo_bucket")],
            },
            Instruction::CallMethod {
                component_address: sender_address,
                method: "balance".to_string(),
                args: args![faucet_resource],
            },
            Instruction::CallMethod {
                component_address: receiver_address,
                method: "balance".to_string(),
                args: args![faucet_resource],
            },
        ])
        .unwrap();
    for log in result.logs {
        eprintln!("LOG: {}", log);
    }
    eprintln!("{:?}", result.execution_results);
    assert_eq!(result.execution_results[3].decode::<Amount>().unwrap(), 900);
    assert_eq!(result.execution_results[4].decode::<Amount>().unwrap(), 100);
}

#[test]
fn test_private_function() {
    // instantiate the counter
    let mut template_test = TemplateTest::new(vec!["tests/templates/private_function"]);

    // check that the private method and function are not exported
    let functions = template_test
        .get_module("PrivateCounter")
        .template_def()
        .functions
        .iter()
        .map(|f| f.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(functions, vec!["new", "get", "increase"]);

    // check that public methods can still internally call private ones
    let component: ComponentAddress = template_test.call_function("PrivateCounter", "new", args![]);
    template_test.call_method::<()>(component, "increase", args![]);
    let value: u32 = template_test.call_method(component, "get", args![]);
    assert_eq!(value, 1);
}

#[test]
fn test_tuples() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/tuples"]);

    // tuples returned in a regular function
    let (message, number): (String, u32) = template_test.call_function("Tuple", "tuple_output", args![]);
    assert_eq!(message, "Hello World!");
    assert_eq!(number, 100);

    // tuples returned in a constructor
    template_test.clear_calls();
    let (component_id, message): (ComponentAddress, String) = template_test.call_function("Tuple", "new", args![]);
    assert_eq!(message, "Hello World!");

    // the component id returned in the tuple must be valid and usable
    let new_value = 20_u32;
    template_test.call_method::<()>(component_id, "set", args![new_value]);
    let value: u32 = template_test.call_method(component_id, "get", args![]);
    assert_eq!(value, new_value);
}

mod errors {
    use super::*;

    #[test]
    fn panic() {
        let template_test = TemplateTest::new(vec!["tests/templates/errors"]);

        let err = template_test
            .try_execute(vec![Instruction::CallFunction {
                template_address: template_test.get_template_address("Errors"),
                function: "panic".to_string(),
                args: args![],
            }])
            .unwrap_err();
        match err {
            TransactionError::WasmExecutionError(WasmExecutionError::Panic { message, .. }) => {
                assert_eq!(message, "This error message should be included in the execution result");
            },
            _ => panic!("Unexpected error: {}", err),
        }
    }

    #[test]
    fn invalid_args() {
        let template_test = TemplateTest::new(vec!["tests/templates/errors"]);

        let text = "this isn't an amount";
        let err = template_test
            .try_execute(vec![Instruction::CallFunction {
                template_address: template_test.get_template_address("Errors"),
                function: "please_pass_invalid_args".to_string(),
                args: args![text],
            }])
            .unwrap_err();
        match err {
            TransactionError::WasmExecutionError(WasmExecutionError::Panic { message, .. }) => {
                assert_eq!(
                    message,
                    format!(
                        "failed to decode argument at position 0 for function 'please_pass_invalid_args': \
                         decode_exact: {} bytes remaining on input",
                        text.len() - size_of::<i64>() + size_of::<u32>()
                    )
                );
            },
            _ => panic!("Unexpected error: {}", err),
        }
    }
}

mod basic_nft {

    use super::*;

    #[test]
    fn create_resource_mint_and_deposit() {
        let mut template_test = TemplateTest::new(vec!["tests/templates/nft/basic_nft"]);

        let account_address: ComponentAddress = template_test.call_function("Account", "new", args![]);
        let nft_component: ComponentAddress = template_test.call_function("SparkleNft", "new", args![]);

        let vars = vec![
            ("account", account_address.into()),
            ("nft", nft_component.into()),
            (
                "nft_resx",
                template_test.get_previous_output_address(SubstateType::Resource).into(),
            ),
        ];

        let total_supply: Amount = template_test.call_method(nft_component, "total_supply", args![]);
        assert_eq!(total_supply, Amount(1));

        let result = template_test
            .execute_and_commit_manifest(
                r#"
            let account = var!["account"];
            let sparkle_nft = var!["nft"];
        
            let nft_bucket = sparkle_nft.mint();
            account.deposit(nft_bucket);
        "#,
                vars.clone(),
            )
            .unwrap();

        let diff = result.result.expect("execution failed");

        // Resource is changed
        assert_eq!(diff.down_iter().filter(|(addr, _)| addr.is_resource()).count(), 1);
        assert_eq!(diff.up_iter().filter(|(addr, _)| addr.is_resource()).count(), 1);

        // NFT and account components changed
        assert_eq!(diff.down_iter().filter(|(addr, _)| addr.is_component()).count(), 2);
        assert_eq!(diff.up_iter().filter(|(addr, _)| addr.is_component()).count(), 2);

        // One new vault created
        assert_eq!(diff.down_iter().filter(|(addr, _)| addr.is_vault()).count(), 0);
        assert_eq!(diff.up_iter().filter(|(addr, _)| addr.is_vault()).count(), 1);

        // One new NFT minted
        assert_eq!(diff.down_iter().filter(|(addr, _)| addr.is_non_fungible()).count(), 0);
        assert_eq!(diff.up_iter().filter(|(addr, _)| addr.is_non_fungible()).count(), 1);

        let total_supply: Amount = template_test.call_method(nft_component, "total_supply", args![]);
        assert_eq!(total_supply, Amount(2));

        let result = template_test
            .execute_and_commit_manifest(
                r#"
            let account = var!["account"];
            let sparkle_nft = var!["nft"];
        
            let nft_bucket = sparkle_nft.withdraw_all();
            account.deposit(nft_bucket);
            sparkle_nft.inner_vault_balance();
            
            let nft_resx = var!["nft_resx"];
            account.balance(nft_resx);
        "#,
                vars,
            )
            .unwrap();
        result.result.expect("execution failed");
        assert_eq!(result.execution_results[3].decode::<Amount>().unwrap(), Amount(0));
    }

    #[test]
    fn change_nft_mutable_data() {
        let mut template_test = TemplateTest::new(vec!["tests/templates/nft/basic_nft"]);

        let account_address: ComponentAddress = template_test.call_function("Account", "new", args![]);
        let nft_component: ComponentAddress = template_test.call_function("SparkleNft", "new", args![]);

        let total_supply: Amount = template_test.call_method(nft_component, "total_supply", args![]);
        assert_eq!(total_supply, Amount(1));

        let vars = [("account", account_address.into()), ("nft", nft_component.into())];

        let result = template_test
            .execute_and_commit_manifest(
                r#"
            let account = var!["account"];
            let sparkle_nft = var!["nft"];
        
            let nft_bucket = sparkle_nft.mint();
            account.deposit(nft_bucket);
        "#,
                vars,
            )
            .unwrap();

        result.result.expect("execution failed");

        let nft_addr = template_test.get_previous_output_address(SubstateType::NonFungible);
        let (nft_resource_addr, _) = nft_addr.as_non_fungible_address().unwrap();

        let vars = [
            ("account", account_address.into()),
            ("nft", nft_component.into()),
            ("nft_resx", nft_resource_addr.into()),
            ("nft_id", nft_addr.into()),
        ];

        template_test
            .execute_and_commit_manifest(
                r#"
            let account = var!["account"];
            let sparkle_nft_resource = var!["nft_resx"];
            account.get_non_fungible_ids(sparkle_nft_resource);
            
            let sparkle_nft = var!["nft"];
            let sparkle_nft_id = var!["nft_id"];
            sparkle_nft.inc_brightness(sparkle_nft_id, 10u32);
        "#,
                vars.clone(),
            )
            .unwrap();

        let nft = template_test
            .read_only_state_store()
            .get_substate(&nft_addr)
            .unwrap()
            .into_substate_value()
            .into_non_fungible()
            .unwrap();
        #[derive(Debug, Clone, Encode, Decode)]
        pub struct Sparkle {
            pub brightness: u32,
        }
        assert_eq!(nft.get_data::<Sparkle>().brightness, 10);

        let err = template_test
            .execute_and_commit_manifest(
                r#"
            let sparkle_nft = var!["nft"];
            let sparkle_nft_id = var!["nft_id"];
            sparkle_nft.dec_brightness(sparkle_nft_id, 11u32);
        "#,
                vars,
            )
            .unwrap_err();

        assert!(err.to_string().contains("Not enough brightness remaining"));
    }
}

mod emoji_id {
    use super::*;

    #[derive(Debug, Clone, Encode, Decode, Hash)]
    pub enum Emoji {
        Smile,
        Sweat,
        Laugh,
        Wink,
    }

    #[derive(Debug, Clone, Encode, Decode, Hash)]
    pub struct EmojiId {
        pub emojis: Vec<Emoji>,
    }

    #[test]
    fn mint_and_withdraw() {
        let mut template_test = TemplateTest::new(vec!["tests/templates/faucet", "tests/templates/nft/emoji_id"]);

        // create an account
        let account_address: ComponentAddress = template_test.call_function("Account", "new", args![]);

        // create a fungible token faucet, we are going to use those tokens as payments
        // TODO: use Thaums instead when they're implemented
        let faucet_template = template_test.get_template_address("TestFaucet");
        let initial_supply = Amount(1_000_000_000_000);
        let result = template_test
            .execute_and_commit(vec![Instruction::CallFunction {
                template_address: faucet_template,
                function: "mint".to_string(),
                args: args![initial_supply],
            }])
            .unwrap();
        let faucet_component: ComponentAddress = result.execution_results[0].decode().unwrap();
        let faucet_resource = result
            .result
            .expect("Faucet mint failed")
            .up_iter()
            .find_map(|(_, s)| s.substate_address().as_resource_address())
            .unwrap();

        // initialize the emoji id minter
        let emoji_id_minter: ComponentAddress =
            template_test.call_function("EmojiIdMinter", "new", args![faucet_resource, 10_u64, Amount(20)]);

        // at the beggining we don't have any emojis minted
        let total_supply: Amount = template_test.call_method(emoji_id_minter, "total_supply", args![]);
        assert_eq!(total_supply, Amount(0));

        // get some funds into the account
        let vars = vec![
            ("account", account_address.into()),
            ("faucet", faucet_component.into()),
            ("emoji_id_minter", emoji_id_minter.into()),
        ];
        template_test
            .execute_and_commit_manifest(
                r#"
            let account = var!["account"];
            let faucet = var!["faucet"];
        
            let coins = faucet.take_free_coins();
            account.deposit(coins);
        "#,
                vars,
            )
            .unwrap();

        // mint a new emoji_id
        // TODO: transaction manifests do not support passing a arbitrary type (like Vec<Emoji>)
        let emoji_vec = vec![Emoji::Smile, Emoji::Laugh];
        template_test
            .execute_and_commit(vec![
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "withdraw".to_string(),
                    args: args![faucet_resource, Amount(25)],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"payment".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: emoji_id_minter,
                    method: "mint".to_string(),
                    args: args![Literal(emoji_vec), Variable("payment")],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"emoji_id".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "deposit".to_string(),
                    args: args![Variable("emoji_id")],
                },
            ])
            .unwrap();

        // the supply of emoji ids should have increased
        let total_supply: Amount = template_test.call_method(emoji_id_minter, "total_supply", args![]);
        assert_eq!(total_supply, Amount(1));
    }

    #[ignore]
    #[test]
    fn insufficient_payments_are_rejected() {
        todo!()
    }

    #[ignore]
    #[test]
    fn emoji_ids_are_unique() {
        // check that duplicated emoji ids cannot be minted
        todo!()
    }

    #[ignore]
    #[test]
    fn invalid_emoji_lengths_are_rejected() {
        todo!()
    }
}
