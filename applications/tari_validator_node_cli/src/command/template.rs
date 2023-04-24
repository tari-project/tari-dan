//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    convert::{TryFrom, TryInto},
    path::{Path, PathBuf},
};

use cargo_metadata::MetadataCommand;
use clap::{Args, Subcommand};
use reqwest::Url;
use tari_dan_engine::wasm::compile::compile_template;
use tari_engine_types::{calculate_template_binary_hash, TemplateAddress};
use tari_validator_node_client::{
    types::{GetTemplateRequest, GetTemplateResponse, GetTemplatesRequest, TemplateRegistrationRequest},
    ValidatorNodeClient,
};

use crate::{from_hex::FromHex, prompt::Prompt, table::Table, table_row};

#[derive(Debug, Subcommand, Clone)]
pub enum TemplateSubcommand {
    Get { template_address: FromHex<TemplateAddress> },
    List,
    Publish(PublishTemplateArgs),
}

#[derive(Debug, Args, Clone)]
pub struct PublishTemplateArgs {
    #[clap(long, short = 'p', alias = "path")]
    pub template_code_path: Option<PathBuf>,

    #[clap(long, alias = "template-name")]
    pub template_name: Option<String>,

    #[clap(long, alias = "template-version")]
    pub template_version: Option<u16>,

    #[clap(long, alias = "template-type")]
    pub template_type: Option<String>,

    #[clap(long, short = 'u', alias = "binary-url", alias = "url")]
    pub binary_url: Option<String>,
}

impl TemplateSubcommand {
    pub async fn handle(self, client: ValidatorNodeClient) -> Result<(), anyhow::Error> {
        #[allow(clippy::enum_glob_use)]
        use TemplateSubcommand::*;
        match self {
            Get { template_address } => handle_get(template_address.into_inner(), client).await?,
            List => handle_list(client).await?,
            Publish(args) => handle_publish(args, client).await?,
        }
        Ok(())
    }
}

async fn handle_get(template_address: TemplateAddress, mut client: ValidatorNodeClient) -> Result<(), anyhow::Error> {
    let GetTemplateResponse {
        registration_metadata,
        abi,
    } = client.get_template(GetTemplateRequest { template_address }).await?;
    println!(
        "Template {} | Mined at {}",
        registration_metadata.address, registration_metadata.height
    );
    println!();

    let mut table = Table::new();
    table.set_titles(vec!["Function", "Args", "Returns"]);
    for f in abi.functions {
        table.add_row(table_row![
            format!("{}::{}", abi.template_name, f.name),
            f.arguments
                .iter()
                .map(|a| format!("{}:{}", a.name, a.arg_type))
                .collect::<Vec<_>>()
                .join(","),
            f.output
        ]);
    }
    table.print_stdout();

    Ok(())
}

async fn handle_list(mut client: ValidatorNodeClient) -> Result<(), anyhow::Error> {
    let templates = client.get_active_templates(GetTemplatesRequest { limit: 10 }).await?;

    let mut table = Table::new();
    table
        .set_titles(vec!["Name", "Address", "Download Url", "Mined Height", "Status"])
        .enable_row_count();
    for template in templates.templates {
        table.add_row(table_row![
            template.name,
            template.address,
            template.url,
            template.height,
            "Active"
        ]);
    }
    table.print_stdout();
    Ok(())
}

async fn handle_publish(args: PublishTemplateArgs, mut client: ValidatorNodeClient) -> anyhow::Result<()> {
    let version;
    let name;
    let binary_sha;
    let binary_url;
    let template_type;
    if let Some(root_folder) = args.template_code_path {
        // retrieve the root folder of the template
        println!("Template code path {}", root_folder.display());
        println!("⏳️ Compiling template...");

        // compile the code and retrieve the binary content of the wasm
        let wasm_module = compile_template(root_folder.as_path(), &[])?;
        let wasm_code = wasm_module.code();
        println!(
            "✅ Template compilation successful (WASM file size: {} bytes)",
            wasm_code.len()
        );
        println!();

        // calculate the hash of the WASM binary
        binary_sha = calculate_template_binary_hash(wasm_code);

        // get the local path of the compiled wasm
        // note that the file name will be the same as the root folder name
        let file_name = root_folder.file_name().unwrap().to_str().unwrap();
        let mut wasm_path = root_folder.clone();
        wasm_path.push(format!("target/wasm32-unknown-unknown/release/{}.wasm", file_name));

        // Show the wasm file path and ask to upload it to the web
        let (cargo_version, cargo_name, default_binary_url) = parse_cargo_file(root_folder.as_path())?;
        binary_url = match args.binary_url {
            Some(value) => value,
            None => {
                println!("Compiled template WASM file location: {}", wasm_path.display());
                println!("Please upload the file to a public web location and then paste the URL");
                Prompt::new("WASM public URL (max 255 characters):")
                    .with_default(default_binary_url)
                    .ask()?
            },
        };

        version = cargo_version;
        name = cargo_name;
        template_type = "wasm";
    } else if let Some(arg_binary_url) = args.binary_url {
        let parsed_url = Url::parse(&arg_binary_url)?;
        let file_name = parsed_url.path().split('/').last().unwrap();
        version = file_name.split('-').nth(1).unwrap_or("0").parse::<u16>()?;
        name = file_name.split('-').next().unwrap_or(file_name).to_string();
        template_type = match file_name.split('.').last().unwrap_or("wasm") {
            "flow" => "flow",
            _ => "wasm",
        };

        binary_sha = calculate_template_binary_hash(reqwest::get(&arg_binary_url).await?.bytes().await?.as_ref());
        binary_url = arg_binary_url;
    } else {
        println!("Please specify a template code path or a binary url");
        return Ok(());
    }

    // ask the template name (skip if already passed as a CLI argument)
    let template_name = Prompt::new("Choose an user-friendly name for the template (max 32 characters):")
        .with_default(name)
        .with_value(args.template_name)
        .ask()?;

    // ask the template version (skip if already passed as a CLI argument)
    let template_version: u16 = Prompt::new("Template version:")
        .with_default(version)
        .with_value(args.template_version)
        .ask_parsed()?;

    let template_type = Prompt::new("Choose a template type (wasm, flow):")
        .with_default(template_type)
        .with_value(args.template_type)
        .ask()?;

    // TODO: ask repository info
    let repo_url = String::new();
    let commit_hash = vec![];

    // build and send the template registration request
    let request = TemplateRegistrationRequest {
        template_name,
        template_version,
        template_type,
        repo_url,
        commit_hash,
        binary_sha: binary_sha.to_vec(),
        binary_url,
    };
    let resp = client.register_template(request).await?;
    println!("✅ Template registration submitted");
    println!();
    println!(
        "The template address will be {}",
        TemplateAddress::try_from(resp.template_address.as_slice())?
    );

    Ok(())
}

fn parse_cargo_file(root_path: &Path) -> anyhow::Result<(u16, String, String)> {
    let metadata = MetadataCommand::new()
        .manifest_path(root_path.join("Cargo.toml"))
        .exec()?;

    if let Some(root) = metadata.root_package() {
        let mut version = root.version.major.try_into()?;
        if version == 0 {
            version = root.version.minor.try_into()?;
        }
        Ok((
            version,
            root.name.to_string(),
            root.metadata["tari"]["binary_url"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        ))
    } else {
        Ok((0, "".to_string(), "".to_string()))
    }
}
