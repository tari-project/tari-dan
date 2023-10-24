//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf, str::FromStr};

use tari_engine_types::{
    instruction::Instruction,
    substate::{SubstateAddress, SubstateDiff},
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::args;
use tari_transaction_manifest::{parse_manifest, ManifestValue};
use tari_validator_node_cli::{
    command::transaction::{handle_submit, submit_transaction, CliArg, CliInstruction, CommonSubmitArgs, SubmitArgs},
    from_hex::FromHex,
    key_manager::KeyManager,
    versioned_substate_address::VersionedSubstateAddress,
};
use tari_validator_node_client::types::SubmitTransactionResponse;

use crate::{logging::get_base_dir_for_scenario, TariWorld};

fn get_key_manager(world: &mut TariWorld) -> KeyManager {
    let path = get_cli_data_dir(world);

    // initialize the account public/private keys
    KeyManager::init(path).unwrap()
}
pub fn create_or_use_key(world: &mut TariWorld, key_name: String) {
    let km = get_key_manager(world);
    if let Some((_, k)) = world.account_keys.get(&key_name) {
        km.set_active_key(&k.to_string()).unwrap();
    } else {
        let key = km.create().expect("Could not create a new key pair");
        km.set_active_key(&key.public_key.to_string()).unwrap();
        world.account_keys.insert(key_name, (key.secret_key, key.public_key));
    }
}
pub fn create_key(world: &mut TariWorld, key_name: String) {
    let key = get_key_manager(world)
        .create()
        .expect("Could not create a new key pair");

    world.account_keys.insert(key_name, (key.secret_key, key.public_key));
}

pub async fn create_account(world: &mut TariWorld, account_name: String, validator_node_name: String) {
    let data_dir = get_cli_data_dir(world);
    let key = get_key_manager(world).create().expect("Could not create keypair");
    let owner_token = key.to_owner_token();
    world
        .account_keys
        .insert(account_name.clone(), (key.secret_key.clone(), key.public_key.clone()));
    // create an account component
    let instruction = Instruction::CallFunction {
        // The "account" template is builtin in the validator nodes with a constant address
        template_address: *ACCOUNT_TEMPLATE_ADDRESS,
        function: "create".to_string(),
        args: args!(owner_token),
    };
    let common = CommonSubmitArgs {
        wait_for_result: true,
        wait_for_result_timeout: Some(120),
        inputs: vec![],
        input_refs: vec![],
        version: None,
        dump_outputs_into: None,
        account_template_address: None,
        dry_run: false,
    };
    let mut client = world.get_validator_node(&validator_node_name).get_client();
    let resp = submit_transaction(vec![instruction], common, data_dir, &mut client)
        .await
        .unwrap();

    if let Some(ref failure) = resp.dry_run_result.as_ref().unwrap().transaction_failure {
        panic!("Transaction failed: {:?}", failure);
    }

    // store the account component address and other substate addresses for later reference
    add_substate_addresses(
        world,
        account_name,
        resp.dry_run_result.unwrap().finalize.result.accept().unwrap(),
    );
}

pub async fn create_component(
    world: &mut TariWorld,
    outputs_name: String,
    template_name: String,
    vn_name: String,
    function_call: String,
    args: Vec<String>,
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

    let args = SubmitArgs {
        instruction,
        common: CommonSubmitArgs {
            wait_for_result: true,
            wait_for_result_timeout: Some(300),
            inputs: vec![],
            input_refs: vec![],
            version: None,
            dump_outputs_into: None,
            account_template_address: None,
            dry_run: false,
        },
    };
    let mut client = world.get_validator_node(&vn_name).get_client();
    let resp = handle_submit(args, data_dir, &mut client).await.unwrap();

    if let Some(ref failure) = resp.dry_run_result.as_ref().unwrap().transaction_failure {
        panic!("Transaction failed: {:?}", failure);
    }
    // store the account component address and other substate addresses for later reference
    add_substate_addresses(
        world,
        outputs_name,
        resp.dry_run_result.unwrap().finalize.result.accept().unwrap(),
    );
}

pub(crate) fn add_substate_addresses(world: &mut TariWorld, outputs_name: String, diff: &SubstateDiff) {
    let outputs = world.outputs.entry(outputs_name).or_default();
    let mut counters = [0usize, 0, 0, 0, 0, 0, 0, 0, 0];
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
            SubstateAddress::NonFungible(_) => {
                outputs.insert(format!("nfts/{}", counters[3]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[3] += 1;
            },
            SubstateAddress::UnclaimedConfidentialOutput(_) => {
                outputs.insert(
                    format!("layer_one_commitments/{}", counters[4]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[4] += 1;
            },
            SubstateAddress::NonFungibleIndex(_) => {
                outputs.insert(format!("nft_indexes/{}", counters[5]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[5] += 1;
            },
            SubstateAddress::TransactionReceipt(_) => {
                outputs.insert(
                    format!("transaction_receipt/{}", counters[6]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[6] += 1;
            },
            SubstateAddress::FeeClaim(_) => {
                outputs.insert(format!("fee_claim/{}", counters[7]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[7] += 1;
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
        component_address: component.address.clone(),
        // TODO: actually parse the method call for arguments
        method_name: method_call,
        args: vec![],
    };

    println!("Inputs: {}", component);
    let args = SubmitArgs {
        instruction,
        common: CommonSubmitArgs {
            wait_for_result: true,
            wait_for_result_timeout: Some(60),
            inputs: vec![component],
            input_refs: vec![],
            version: None,
            dump_outputs_into: None,
            account_template_address: None,
            dry_run: false,
        },
    };
    let mut client = world.get_validator_node(&vn_name).get_client();
    let resp = handle_submit(args, data_dir, &mut client).await.unwrap();

    if let Some(ref failure) = resp.dry_run_result.as_ref().unwrap().transaction_failure {
        panic!("Transaction failed: {:?}", failure);
    }
    // store the account component address and other substate addresses for later reference
    add_substate_addresses(
        world,
        outputs_name,
        resp.dry_run_result.as_ref().unwrap().finalize.result.accept().unwrap(),
    );
    resp
}

pub async fn submit_manifest(
    world: &mut TariWorld,
    vn_name: String,
    outputs_name: String,
    manifest_content: String,
    input_str: String,
    signing_key_name: String,
) {
    // HACKY: Sets the active key so that submit_transaction will use it.
    let (_, key) = world.account_keys.get(&signing_key_name).unwrap();
    let key_str = key.to_string();
    get_key_manager(world).set_active_key(&key_str).unwrap();

    let input_groups = input_str.split(',').map(|s| s.trim()).collect::<Vec<_>>();
    // generate globals for components addresses
    let globals: HashMap<String, ManifestValue> = world
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
    let mut client = world.get_validator_node(&vn_name).get_client();
    let data_dir = get_cli_data_dir(world);

    // Supply the inputs explicitly. If this is empty, the internal component manager will attempt to supply the correct
    // inputs
    let inputs = input_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.starts_with("ref:"))
        .flat_map(|s| {
            world
                .outputs
                .get(s)
                .unwrap_or_else(|| panic!("No outputs named {}", s.trim()))
        })
        .filter(|(_, addr)| !addr.address.is_transaction_receipt())
        .map(|(_, addr)| addr.clone())
        .collect::<Vec<_>>();

    // Remove inputs that have been downed
    let inputs = select_latest_version(inputs);

    let input_refs = input_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| s.starts_with("ref:"))
        .flat_map(|s| {
            world
                .outputs
                .get(s)
                .unwrap_or_else(|| panic!("No outputs named {}", s.trim()))
        })
        .filter(|(_, addr)| !addr.address.is_transaction_receipt())
        .map(|(_, addr)| addr.clone())
        .collect::<Vec<_>>();

    // Remove inputs that have been downed
    let inputs = select_latest_version(inputs);

    let args = CommonSubmitArgs {
        wait_for_result: true,
        wait_for_result_timeout: Some(60),
        inputs,
        input_refs,
        version: None,
        dump_outputs_into: None,
        account_template_address: None,
        dry_run: false,
    };
    let resp = submit_transaction(instructions, args, data_dir, &mut client)
        .await
        .unwrap();

    if let Some(ref failure) = resp.dry_run_result.as_ref().unwrap().transaction_failure {
        panic!("Transaction failed: {:?}", failure);
    }

    add_substate_addresses(
        world,
        outputs_name,
        resp.dry_run_result.unwrap().finalize.result.accept().unwrap(),
    );
}

pub(crate) fn get_cli_data_dir(world: &mut TariWorld) -> PathBuf {
    get_base_dir_for_scenario("vn_cli", world.current_scenario_name.as_ref().unwrap(), "SHARED")
}

// Remove inputs that have been downed
fn select_latest_version(mut inputs: Vec<VersionedSubstateAddress>) -> Vec<VersionedSubstateAddress> {
    inputs.sort_by(|a, b| b.address.cmp(&a.address).then(b.version.cmp(&a.version)));
    inputs.dedup_by(|a, b| a.address == b.address);
    inputs
}
