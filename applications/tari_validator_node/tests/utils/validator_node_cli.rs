//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{path::PathBuf, str::FromStr};

use tari_engine_types::substate::{SubstateAddress, SubstateDiff};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_transaction_manifest::parse_manifest;
use tari_validator_node_cli::{
    command::{
        handle_submit,
        transaction::{submit_transaction, CliArg, CliInstruction, CommonSubmitArgs, SubmitArgs},
    },
    from_hex::FromHex,
    key_manager::KeyManager,
    versioned_substate_address::VersionedSubstateAddress,
};
use tari_validator_node_client::{types::SubmitTransactionResponse, ValidatorNodeClient};
use tempfile::tempdir;

use super::validator_node::get_vn_client;
use crate::TariWorld;

pub async fn create_dan_wallet(world: &mut TariWorld) {
    let data_dir = get_cli_data_dir(world);

    // initialize the account public/private keys
    let path = PathBuf::from(data_dir);
    let account_manager = KeyManager::init(path).unwrap();
    account_manager.create().unwrap();
}

pub async fn create_account(world: &mut TariWorld, account_name: String, validator_node_name: String) {
    let data_dir = get_cli_data_dir(world);

    // create an account component
    let instruction = CliInstruction::CallFunction {
        // The "account" template is builtin in the validator nodes with a constant address
        template_address: FromHex(ACCOUNT_TEMPLATE_ADDRESS),
        function_name: "new".to_owned(),
        args: vec![], // the account constructor does not have args
    };
    let args = SubmitArgs {
        instruction,
        common: CommonSubmitArgs {
            wait_for_result: true,
            wait_for_result_timeout: Some(60),
            num_outputs: Some(1),
            inputs: vec![],
            version: None,
            dump_outputs_into: None,
            account_template_address: None,
            dry_run: false,
        },
    };
    let mut client = get_validator_node_client(world, validator_node_name).await;
    let resp = handle_submit(args, data_dir, &mut client).await.unwrap();

    // store the account component address and other substate addresses for later reference
    add_substate_addresses(
        world,
        account_name,
        resp.result.unwrap().finalize.result.accept().unwrap(),
    );
}

pub async fn create_component(
    world: &mut TariWorld,
    outputs_name: String,
    template_name: String,
    vn_name: String,
    function_call: String,
    args: Vec<String>,
    num_outputs: u64,
) {
    let data_dir = get_cli_data_dir(world);

    let template_address = world
        .templates
        .get(&template_name)
        .unwrap_or_else(|| panic!("Template not found with name {}", template_name))
        .address;
    let args: Vec<CliArg> = args.iter().map(|a| CliArg::from_str(a).unwrap()).collect();
    let instruction = CliInstruction::CallFunction {
        template_address: FromHex(template_address),
        function_name: function_call,
        args,
    };

    let num_outputs = if num_outputs == 0 {
        None
    } else {
        Some(num_outputs as u8)
    };

    let args = SubmitArgs {
        instruction,
        common: CommonSubmitArgs {
            wait_for_result: true,
            wait_for_result_timeout: Some(60),
            num_outputs,
            inputs: vec![],
            version: None,
            dump_outputs_into: None,
            account_template_address: None,
            dry_run: false,
        },
    };
    let mut client = get_validator_node_client(world, vn_name).await;
    let resp = handle_submit(args, data_dir, &mut client).await.unwrap();

    // store the account component address and other substate addresses for later reference
    add_substate_addresses(
        world,
        outputs_name,
        resp.result.unwrap().finalize.result.accept().unwrap(),
    );
}

fn add_substate_addresses(world: &mut TariWorld, outputs_name: String, diff: &SubstateDiff) {
    let outputs = world.outputs.entry(outputs_name).or_default();
    let mut counters = [0usize, 0, 0, 0];
    for (addr, data) in diff.up_iter() {
        match addr {
            SubstateAddress::Component(_) => {
                let component = data.substate_value().component().unwrap();
                outputs.insert(
                    format!("components/{}", component.module_name),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[0] += 1;
            },
            SubstateAddress::Resource(_) => {
                outputs.insert(format!("resources/{}", counters[1]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[1] += 1;
            },
            SubstateAddress::Vault(_) => {
                outputs.insert(format!("vaults/{}", counters[2]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[2] += 1;
            },
            SubstateAddress::NonFungible(_, _) => {
                outputs.insert(format!("nfts/{}", counters[3]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[3] += 1;
            },
        }
    }
}

pub async fn call_method(
    world: &mut TariWorld,
    vn_name: String,
    fq_component_name: String,
    outputs_name: String,
    method_call: String,
    num_outputs: u64,
) -> SubmitTransactionResponse {
    let data_dir = get_cli_data_dir(world);
    let (input_group, component_name) = fq_component_name.split_once('/').unwrap_or_else(|| {
        panic!(
            "Component name must be in the format '{{group}}/components/{{template_name}}', got {}",
            fq_component_name
        )
    });
    let component = world
        .outputs
        .get(input_group)
        .unwrap_or_else(|| panic!("No outputs found with name {}", input_group))
        .iter()
        .find(|(name, _)| **name == component_name)
        .map(|(_, data)| data.clone())
        .unwrap_or_else(|| panic!("No component named {}", component_name));

    let instruction = CliInstruction::CallMethod {
        component_address: component.address,
        // TODO: actually parse the method call for arguments
        method_name: method_call,
        args: vec![],
    };

    let num_outputs = if num_outputs == 0 {
        None
    } else {
        Some(num_outputs as u8)
    };

    let args = SubmitArgs {
        instruction,
        common: CommonSubmitArgs {
            wait_for_result: true,
            wait_for_result_timeout: Some(60),
            num_outputs,
            inputs: vec![],
            version: None,
            dump_outputs_into: None,
            account_template_address: None,
            dry_run: false,
        },
    };
    let mut client = get_validator_node_client(world, vn_name).await;
    let resp = handle_submit(args, data_dir, &mut client).await.unwrap();
    // store the account component address and other substate addresses for later reference
    add_substate_addresses(
        world,
        outputs_name,
        resp.result.as_ref().unwrap().finalize.result.accept().unwrap(),
    );
    resp
}

pub async fn submit_manifest(
    world: &mut TariWorld,
    vn_name: String,
    outputs_name: String,
    manifest_content: String,
    inputs: String,
    num_outputs: u64,
) {
    let input_groups = inputs.split(',').map(|s| s.trim()).collect::<Vec<_>>();
    // generate globals for components addresses
    let globals = world
        .outputs
        .iter()
        .filter(|(name, _)| input_groups.contains(&name.as_str()))
        .flat_map(|(name, outputs)| {
            outputs
                .iter()
                .map(move |(child_name, addr)| (format!("{}/{}", name, child_name), addr.address.clone().into()))
        })
        .collect();

    // parse the manifest
    let instructions = parse_manifest(&manifest_content, globals).unwrap();

    // submit the instructions to the vn
    let mut client = get_validator_node_client(world, vn_name).await;
    let data_dir = get_cli_data_dir(world);
    let num_outputs = if num_outputs == 0 {
        None
    } else {
        Some(num_outputs as u8)
    };

    // Supply the inputs explicitly. If this is empty, the internal component manager will attempt to supply the correct
    // inputs
    let inputs = inputs
        .split(',')
        .flat_map(|s| {
            world
                .outputs
                .get(s.trim())
                .unwrap_or_else(|| panic!("No outputs named {}", s.trim()))
        })
        .map(|(_, addr)| addr.clone())
        .collect();

    let args = CommonSubmitArgs {
        wait_for_result: true,
        wait_for_result_timeout: Some(60),
        num_outputs,
        inputs,
        version: None,
        dump_outputs_into: None,
        account_template_address: None,
        dry_run: false,
    };
    let resp = submit_transaction(instructions, args, data_dir, &mut client)
        .await
        .unwrap();

    add_substate_addresses(
        world,
        outputs_name,
        resp.result.unwrap().finalize.result.accept().unwrap(),
    );
}

async fn get_validator_node_client(world: &TariWorld, validator_node_name: String) -> ValidatorNodeClient {
    let port = world.validator_nodes.get(&validator_node_name).unwrap().json_rpc_port;
    get_vn_client(port).await
}

fn get_cli_data_dir(world: &mut TariWorld) -> String {
    if let Some(dir) = &world.cli_data_dir {
        return dir.to_string();
    }

    let temp_dir = tempdir().unwrap().path().join("cli_data_dir");
    let temp_dir_path = temp_dir.display().to_string();
    world.cli_data_dir = Some(temp_dir_path.clone());
    temp_dir_path
}
