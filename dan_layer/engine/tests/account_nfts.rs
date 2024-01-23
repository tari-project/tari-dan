//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    models::{ComponentAddress, NonFungibleId},
    prelude::Metadata,
    resource::TOKEN_SYMBOL,
};
use tari_template_test_tooling::TemplateTest;

#[test]
fn basic_nft_mint() {
    let mut account_nft_template_test = TemplateTest::new::<_, &str>([]);

    let account_nft_template = account_nft_template_test.get_template_address("AccountNonFungible");

    let (owner_component_address, owner_token, _) = account_nft_template_test.create_owned_account();

    let result = account_nft_template_test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: account_nft_template,
                function: "create".to_string(),
                args: args![owner_token],
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
                method: "non_fungible_token_get_resource_address".to_string(),
                args: args![],
            }],
            vec![],
        )
        .unwrap();

    assert!(result.finalize.is_accept());

    let mut metadata = Metadata::new();

    metadata.insert(TOKEN_SYMBOL.to_string(), "ACCNFT".to_string());
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
                Instruction::CallFunction {
                    template_address: account_nft_template_test.get_template_address("Account"),
                    function: "get_non_fungible_ids_for_bucket".to_string(),
                    args: args![Variable("my_nft")],
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

    let bucket_nfts = result.finalize.execution_results[2]
        .decode::<Vec<NonFungibleId>>()
        .unwrap();
    assert_eq!(bucket_nfts.len(), 1);
}
