//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf, str::FromStr};

use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    models::{ComponentAddress, TemplateAddress},
    Hash,
};
use tari_transaction_manifest::parse_manifest;
use tari_validator_node_cli::{
    command::{
        handle_submit,
        transaction::{submit_transaction, CliArg, CliInstruction, CommonSubmitArgs, SubmitArgs},
    },
    from_hex::FromHex,
    key_manager::KeyManager,
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
        template_address: FromHex(TemplateAddress::from([0; 32])),
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
    let resp = handle_submit(args, data_dir, &mut client).await.unwrap().unwrap();

    // store the account component id for later reference
    let results = resp.result.unwrap().finalize.execution_results;
    let component_id: Hash = results.first().unwrap().decode().unwrap();
    world.components.insert(account_name, component_id);
}

pub async fn create_component(
    world: &mut TariWorld,
    component_name: String,
    template_name: String,
    vn_name: String,
    function_call: String,
    args: Vec<String>,
    num_outputs: u64,
) {
    let data_dir = get_cli_data_dir(world);

    let template_address = world.templates.get(&template_name).unwrap().address;
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
    let resp = handle_submit(args, data_dir, &mut client).await.unwrap().unwrap();

    // store the account component id for later reference
    let results = resp.result.unwrap().finalize.execution_results;
    let component_id: Hash = results.first().unwrap().decode().unwrap();
    world.components.insert(component_name, component_id);
}

pub async fn call_method(
    world: &mut TariWorld,
    vn_name: String,
    component_name: String,
    method_call: String,
    num_outputs: u64,
) -> SubmitTransactionResponse {
    let data_dir = get_cli_data_dir(world);
    let component_address = world.components.get(&component_name).unwrap();

    let instruction = CliInstruction::CallMethod {
        component_address: ComponentAddress::new(*component_address).into(),
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
    handle_submit(args, data_dir, &mut client).await.unwrap().unwrap()
}

pub async fn submit_manifest(world: &mut TariWorld, vn_name: String, manifest_content: String, num_outputs: u64) {
    // generate globals for components addresses
    let mut globals = HashMap::new();
    for component in &world.components {
        let name = component.0.to_string();
        let component_address_hash = component.1;
        let substate_address = SubstateAddress::Component(ComponentAddress::new(*component_address_hash)).into();
        globals.insert(name, substate_address);
    }

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
    let args = CommonSubmitArgs {
        wait_for_result: true,
        wait_for_result_timeout: Some(60),
        num_outputs,
        inputs: vec![],
        version: None,
        dump_outputs_into: None,
        account_template_address: None,
        dry_run: false,
    };
    submit_transaction(instructions, args, data_dir, &mut client)
        .await
        .unwrap();
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
