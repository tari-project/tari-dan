//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;
use minotari_app_grpc::tari_rpc::{CreateTemplateRegistrationRequest, template_type, TemplateRegistration, WasmInfo};

use tari_dan_engine::wasm::compile::compile_template;
use tari_engine_types::{hashing::template_hasher32, TemplateAddress};
use tari_template_lib::Hash;
use tari_transaction::TransactionId;
use minotari_app_grpc::tari_rpc::BuildInfo;
use minotari_app_grpc::tari_rpc::TemplateType;
use crate::template::template_type::TemplateType::Wasm;



use crate::TariWorld;

#[derive(Debug)]
pub struct RegisteredTemplate {
    pub name: String,
    pub address: TemplateAddress,
}

pub async fn send_template_registration(
    world: &mut TariWorld,
    template_name: String,
    wallet_name: String,
) -> anyhow::Result<TemplateAddress> {
    dbg!("here");
    let binary_sha = compile_wasm_template(template_name.clone())?;

    // publish the wasm file into http to be able to be fetched by the VN later
    let wasm_file_path = get_template_wasm_path(template_name.clone());

    let mock = world
        .get_mock_server()
        .publish_file(template_name.clone(), wasm_file_path.display().to_string())
        .await;


    // build the template registration request
    let request = CreateTemplateRegistrationRequest {

            template_name,
            template_version: 0,
            template_type: Some(TemplateType{ template_type: Some( template_type::TemplateType::Wasm(
                WasmInfo { abi_version: 1}
            ))}),
            build_info: Some(BuildInfo {
                repo_url: "".to_string(),
                commit_hash: vec![],
            }),
            // repo_url: String::new(),
            // commit_hash: vec![],
            binary_sha: binary_sha.to_vec(),
            binary_url: mock.url,
            sidechain_deployment_key: vec![],
        fee_per_gram: 1,
    };

    dbg!("here 2");
    // send the template registration request
    let wallet = world.get_wallet(&wallet_name);
    let mut client = wallet.create_client().await;

    dbg!("here3");
    // store the template address for future reference
    let resp = client.create_template_registration(request).await?.into_inner();
    dbg!("here4");
    Ok(TemplateAddress::try_from_vec(resp.template_address).unwrap())
}

pub fn compile_wasm_template(template_name: String) -> Result<Hash, anyhow::Error> {
    let mut template_path = get_template_root_path();

    template_path.push(template_name);
    let wasm_module = compile_template(template_path.as_path(), &[])?;
    let wasm_code = wasm_module.code();
    Ok(template_hasher32().chain(&wasm_code).result())
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
