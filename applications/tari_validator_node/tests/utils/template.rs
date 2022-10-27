use std::path::PathBuf;

use tari_dan_engine::wasm::compile::compile_template;
use tari_engine_types::{hashing::hasher, TemplateAddress};
use tari_validator_node_client::types::{TemplateRegistrationRequest, TemplateRegistrationResponse};

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

    // build the template registration request
    let request = TemplateRegistrationRequest {
        template_name,
        template_version: 0,
        repo_url: String::new(),
        commit_hash: vec![],
        binary_sha,
        binary_url: String::new(),
    };

    // send the template registration request
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;

    // store the template address for future reference
    client.register_template(request).await.unwrap()
}

fn get_template_binary_hash(template_name: String) -> Vec<u8> {
    let mut template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    template_path.push("tests/templates");
    template_path.push(template_name);
    let wasm_module = compile_template(template_path.as_path(), &[]).unwrap();
    let wasm_code = wasm_module.code();
    hasher("template").chain(&wasm_code).result().to_vec()
}
