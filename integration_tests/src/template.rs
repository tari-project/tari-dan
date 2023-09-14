//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use tari_dan_engine::wasm::compile::compile_template;
use tari_engine_types::{hashing::template_hasher, TemplateAddress};
use tari_template_lib::Hash;
use tari_validator_node_client::types::{TemplateRegistrationRequest, TemplateRegistrationResponse};

use crate::TariWorld;

#[derive(Debug)]
pub struct RegisteredTemplate {
    pub name: String,
    pub address: TemplateAddress,
}

pub async fn send_template_registration(
    world: &mut TariWorld,
    template_name: String,
    vn_name: String,
) -> anyhow::Result<TemplateRegistrationResponse> {
    let binary_sha = compile_wasm_template(template_name.clone())?;

    // publish the wasm file into http to be able to be fetched by the VN later
    let wasm_file_path = get_template_wasm_path(template_name.clone());

    let mock = world
        .get_mock_server()
        .publish_file(template_name.clone(), wasm_file_path.display().to_string())
        .await;

    // build the template registration request
    let request = TemplateRegistrationRequest {
        template_name,
        template_version: 0,
        template_type: "wasm".to_string(),
        repo_url: String::new(),
        commit_hash: vec![],
        binary_sha: binary_sha.to_vec(),
        binary_url: mock.url,
    };

    // send the template registration request
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.get_client();

    // store the template address for future reference
    let resp = client.register_template(request).await?;
    Ok(resp)
}

pub fn compile_wasm_template(template_name: String) -> Result<Hash, anyhow::Error> {
    let mut template_path = get_template_root_path();

    template_path.push(template_name);
    let wasm_module = compile_template(template_path.as_path(), &[])?;
    let wasm_code = wasm_module.code();
    Ok(template_hasher().chain(&wasm_code).result())
}

pub fn get_template_wasm_path(template_name: String) -> PathBuf {
    let mut wasm_path = get_template_root_path();
    wasm_path.push(template_name.clone());
    wasm_path.push(format!("target/wasm32-unknown-unknown/release/{}.wasm", template_name));

    wasm_path
}

// pub fn get_all_template_names() -> Vec<String> {
//     let mut template_path = get_template_root_path();
//     let mut templates = Vec::new();
//     for entry in std::fs::read_dir(template_path).unwrap() {
//         let entry = entry.unwrap();
//         let path = entry.path();
//         if path.is_dir() {
//             templates.push(path.file_name().unwrap().to_str().unwrap().to_string());
//         }
//     }
//     templates
// }
//
fn get_template_root_path() -> PathBuf {
    let mut template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    template_path.push("src/templates");
    template_path
}
