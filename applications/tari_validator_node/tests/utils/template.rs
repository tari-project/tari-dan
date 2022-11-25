//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use tari_dan_engine::{crypto::create_key_pair, transaction::Transaction, wasm::compile::compile_template};
use tari_engine_types::{hashing::hasher, instruction::Instruction, TemplateAddress};
use tari_validator_node_client::types::{
    SubmitTransactionRequest,
    SubmitTransactionResponse,
    TemplateRegistrationRequest,
    TemplateRegistrationResponse,
};

use super::http_server::MockHttpServer;
use crate::{utils::validator_node::get_vn_client, TariWorld};

#[derive(Debug)]
pub struct RegisteredTemplate {
    pub name: String,
    pub address: TemplateAddress,
}

pub async fn send_template_transaction(
    world: &mut TariWorld,
    vn_name: String,
    template_name: String,
    function_name: String,
) -> SubmitTransactionResponse {
    let template_address = world.templates.get(&template_name).unwrap().address;

    let instruction = Instruction::CallFunction {
        template_address,
        function: function_name,
        args: vec![],
    };

    let (secret_key, _public_key) = create_key_pair();

    let mut builder = Transaction::builder();
    builder.add_instruction(instruction).sign(&secret_key).fee(1);
    let transaction = builder.build();

    let req = SubmitTransactionRequest {
        instructions: transaction.instructions().to_vec(),
        signature: transaction.signature().clone(),
        fee: transaction.fee(),
        sender_public_key: transaction.sender_public_key().clone(),
        wait_for_result: true,
        wait_for_result_timeout: None,
        inputs: vec![],
        num_outputs: 0,
    };

    // send the template transaction request
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;
    client.submit_transaction(req).await.unwrap()
}

pub async fn send_template_registration(
    world: &mut TariWorld,
    template_name: String,
    vn_name: String,
) -> TemplateRegistrationResponse {
    let binary_sha = get_template_binary_hash(template_name.clone());

    // publish the wasm file into http to be able to be fetched by the VN later
    let wasm_file_path = get_template_wasm_path(template_name.clone());
    if world.http_server.is_none() {
        world.http_server = Some(MockHttpServer::new(46000).await);
    }
    let binary_url = world
        .http_server
        .as_ref()
        .unwrap()
        .publish_file(template_name.clone(), wasm_file_path.display().to_string());

    // build the template registration request
    let request = TemplateRegistrationRequest {
        template_name,
        template_version: 0,
        repo_url: String::new(),
        commit_hash: vec![],
        binary_sha,
        binary_url,
    };

    // send the template registration request
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;

    // store the template address for future reference
    client.register_template(request).await.unwrap()
}

fn get_template_binary_hash(template_name: String) -> Vec<u8> {
    let mut template_path = get_template_root_path();
    template_path.push(template_name);
    let wasm_module = compile_template(template_path.as_path(), &[]).unwrap();
    let wasm_code = wasm_module.code();
    hasher("template").chain(&wasm_code).result().to_vec()
}

fn get_template_wasm_path(template_name: String) -> PathBuf {
    let mut wasm_path = get_template_root_path();
    wasm_path.push(template_name.clone());
    wasm_path.push(format!("target/wasm32-unknown-unknown/release/{}.wasm", template_name));

    wasm_path
}

fn get_template_root_path() -> PathBuf {
    let mut template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    template_path.push("tests/templates");
    template_path
}
