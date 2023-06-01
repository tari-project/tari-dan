//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress, ResourceAddress},
    prelude::Metadata,
    Hash,
};
use tari_template_test_tooling::TemplateTest;
use tari_transaction::Transaction;

#[test]
fn basic_nft_mint() {
    let mut account_nft_template_test = TemplateTest::new(vec!["../template_builtin/templates/account_nfts/"]);

    let account_nft_template = account_nft_template_test.get_template_address("AccountNonFungible");

    let (owner_component_address, owner_token, a) = account_nft_template_test.create_owned_account();
    let token_symbol: &str = "ACCNFT";

    let result = account_nft_template_test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: account_nft_template,
                function: "new".to_string(),
                args: args![owner_token, token_symbol],
            }],
            vec![],
        )
        .unwrap();

    assert!(result.finalize.result.is_accept());
    let nft_component_address: ComponentAddress = result.finalize.execution_results[0].decode().unwrap();

    let result = account_nft_template_test
        .execute_and_commit(
            vec![Instruction::CallMethod {
                component_address: nft_component_address,
                method: "get_resource_address".to_string(),
                args: args![],
            }],
            vec![owner_token.clone()],
        )
        .unwrap();

    assert!(result.finalize.is_accept());
    assert_eq!(
        result.finalize.execution_results[0]
            .decode::<ResourceAddress>()
            .unwrap(),
        ResourceAddress::new(Hash::from_array([
            111, 55, 52, 37, 223, 64, 196, 187, 156, 159, 233, 83, 173, 242, 171, 202, 185, 41, 8, 165, 148, 46, 29,
            61, 151, 144, 62, 253, 183, 220, 7, 77
        ]))
    );

    let mut metadata = Metadata::new();

    metadata.insert("name".to_string(), "my_custom_nft".to_string());
    metadata.insert("brightness".to_string(), "100".to_string());

    let result = account_nft_template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: nft_component_address,
                    method: "mint".to_string(),
                    args: args![metadata],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"my_nft".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: owner_component_address,
                    method: "deposit".to_string(),
                    args: args![Variable("my_nft")],
                },
            ],
            vec![owner_token],
        )
        .unwrap();

    assert!(result.finalize.result.is_accept());
}
