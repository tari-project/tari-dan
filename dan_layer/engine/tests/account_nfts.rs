//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    models::{ComponentAddress, ResourceAddress},
    prelude::Metadata,
    Hash,
};
use tari_template_test_tooling::TemplateTest;

#[test]
fn basic_nft_mint() {
    let mut account_nft_template_test = TemplateTest::new(vec!["../template_builtin/templates/account_nfts/"]);

    let account_nft_template = account_nft_template_test.get_template_address("AccountNonFungible");

    let (owner_component_address, owner_token, _) = account_nft_template_test.create_owned_account();
    let token_symbol: &str = "ACCNFT";

    let result = account_nft_template_test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: account_nft_template,
                function: "create".to_string(),
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
            vec![],
        )
        .unwrap();

    assert!(result.finalize.is_accept());
    assert_eq!(
        result.finalize.execution_results[0]
            .decode::<ResourceAddress>()
            .unwrap(),
        ResourceAddress::new(Hash::from_array([
            190, 68, 108, 55, 177, 254, 106, 118, 91, 103, 229, 52, 244, 117, 117, 179, 2, 112, 56, 20, 4, 142, 53, 96,
            31, 165, 116, 253, 5, 209, 2, 101
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
    println!("FLAG: CUCUMBER {:?}", result.finalize.events);
}
