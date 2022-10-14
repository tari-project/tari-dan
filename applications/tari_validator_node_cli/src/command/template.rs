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

use std::{convert::TryFrom, path::PathBuf};

use clap::{Args, Subcommand};
use tari_dan_engine::wasm::compile::compile_template;
use tari_engine_types::{hashing::hasher, TemplateAddress};
use tari_validator_node_client::{types::TemplateRegistrationRequest, ValidatorNodeClient};

use crate::Prompt;

#[derive(Debug, Subcommand, Clone)]
pub enum TemplateSubcommand {
    Publish(PublishTemplateArgs),
}

#[derive(Debug, Args, Clone)]
pub struct PublishTemplateArgs {
    #[clap(long, short = 'p', alias = "path")]
    pub template_code_path: PathBuf,

    #[clap(long, alias = "template-name")]
    pub template_name: Option<String>,

    #[clap(long, alias = "template-version")]
    pub template_version: Option<u16>,

    #[clap(long, alias = "binary-url")]
    pub binary_url: Option<String>,
}

impl TemplateSubcommand {
    pub async fn handle(self, client: ValidatorNodeClient) -> Result<(), anyhow::Error> {
        match self {
            TemplateSubcommand::Publish(args) => handle_publish(args, client).await?,
        }
        Ok(())
    }
}

async fn handle_publish(args: PublishTemplateArgs, mut client: ValidatorNodeClient) -> anyhow::Result<()> {
    // retrieve the root folder of the template
    let root_folder = args.template_code_path;
    println!("Template code path {}", root_folder.display());
    println!("⏳️ Compiling template...");

    // compile the code and retrieve the binary content of the wasm
    let wasm_module = compile_template(root_folder.as_path(), &[]).unwrap();
    let wasm_code = wasm_module.code();
    println!(
        "✅ Template compilation successful (WASM file size: {} bytes)",
        wasm_code.len()
    );
    println!();

    // calculate the hash of the WASM binary
    let binary_sha = hasher("template").chain(&wasm_code).result();

    // get the local path of the compiled wasm
    // note that the file name will be the same as the root folder name
    let file_name = root_folder.file_name().unwrap().to_str().unwrap();
    let mut wasm_path = root_folder.clone();
    wasm_path.push(format!("target/wasm32-unknown-unknown/release/{}.wasm", file_name));

    // ask the template name (skip if already passed as a CLI argument)
    let template_name = Prompt::new("Choose an user-friendly name for the template (max 32 characters):")
        .with_value(args.template_name)
        .ask()?;

    // ask the template version (skip if already passed as a CLI argument)
    let template_version: u16 = Prompt::new("Template version:")
        .with_default(0)
        .with_value(args.template_version)
        .ask_parsed()?;

    // TODO: ask repository info
    let repo_url = String::new();
    let commit_hash = vec![];

    // Show the wasm file path and ask to upload it to the web
    let binary_url = match args.binary_url {
        Some(value) => value,
        None => {
            println!("Compiled template WASM file location: {}", wasm_path.display());
            println!("Please upload the file to a public web location and then paste the URL");
            Prompt::new("WASM public URL (max 255 characters):").ask()?
        },
    };

    // build and send the template registration request
    let request = TemplateRegistrationRequest {
        template_name,
        template_version,
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
        TemplateAddress::try_from(resp.template_address.as_slice()).unwrap()
    );

    Ok(())
}
