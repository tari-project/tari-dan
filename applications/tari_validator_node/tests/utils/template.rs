//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use tari_dan_engine::wasm::compile::compile_template;
use tari_engine_types::{
    hashing::{hasher, EngineHashDomainLabel},
    TemplateAddress,
};
use tari_validator_node_client::types::{TemplateRegistrationRequest, TemplateRegistrationResponse};

use super::http_server::MockHttpServer;
use crate::{utils::validator_node::get_vn_client, TariWorld};

#[derive(Debug)]
pub struct RegisteredTemplate {
    pub name: String,
    pub address: TemplateAddress,
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
    hasher(EngineHashDomainLabel::Template)
        .chain(&wasm_code)
        .result()
        .to_vec()
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
