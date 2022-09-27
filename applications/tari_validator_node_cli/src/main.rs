// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

mod cli;
mod client;
mod command;
mod prompt;

use std::error::Error;

use anyhow::anyhow;
use command::{RegisterSubcommand, RegisterTemplateArgs};
use multiaddr::{Multiaddr, Protocol};
use reqwest::Url;
use tari_dan_engine::{hashing::hasher, wasm::compile::compile_template};

use crate::{
    cli::Cli,
    client::{TemplateRegistrationRequest, ValidatorNodeClient},
    command::Command,
    prompt::{Prompt, YesNo},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::init();

    let endpoint = cli
        .vn_daemon_jrpc_endpoint
        .map(multiaddr_to_http_url)
        .transpose()?
        .ok_or_else(|| {
            anyhow!("For now, please provide a daemon endpoint using e.g. `--endpoint /ip4/127.0.0.1/tcp/xxxx`")
        })?;

    log::info!("🌍️ Connecting to {}", endpoint);
    let client = ValidatorNodeClient::connect(endpoint)?;

    handle_command(cli.command, client).await?;

    Ok(())
}

fn multiaddr_to_http_url(multiaddr: Multiaddr) -> anyhow::Result<Url> {
    let mut iter = multiaddr.iter();
    let ip = iter.next().ok_or_else(|| anyhow!("Invalid multiaddr"))?;
    let port = iter.next().ok_or_else(|| anyhow!("Invalid multiaddr"))?;

    let ip = match ip {
        Protocol::Ip4(ip) => ip.to_string(),
        Protocol::Ip6(ip) => ip.to_string(),
        _ => return Err(anyhow!("Invalid multiaddr")),
    };

    let port = match port {
        Protocol::Tcp(port) => port,
        _ => return Err(anyhow!("Invalid multiaddr")),
    };

    let url = Url::parse(&format!("http://{}:{}", ip, port))?;
    Ok(url)
}

async fn handle_command(command: Command, client: ValidatorNodeClient) -> anyhow::Result<()> {
    match command {
        Command::Register(command) => match command.subcommand {
            RegisterSubcommand::Validator => {
                handle_register_node(client).await?;
            },
            RegisterSubcommand::Template(args) => {
                handle_register_template(args, client).await?;
            },
        },
    }

    Ok(())
}

async fn handle_register_node(mut client: ValidatorNodeClient) -> anyhow::Result<()> {
    let tx_id = client.register_validator_node().await?;
    println!("✅ Validator node registration submitted (tx_id: {})", tx_id);

    Ok(())
}

async fn handle_register_template(args: RegisterTemplateArgs, mut client: ValidatorNodeClient) -> anyhow::Result<()> {
    // retrieve the root folder of the template
    let root_folder = args.template_code_path;
    println!("Template code path {}", root_folder.display());

    // compile the code and retrieve the binary content of the wasm
    let wasm_module = compile_template(root_folder.as_path(), &[]).unwrap();
    let wasm_code = wasm_module.code();
    println!(
        "✅ Template compilation succesful (WASM file size: {} bytes)",
        wasm_code.len()
    );

    // calculate the hash of the WASM binary
    let binary_sha = hasher("template").chain(&wasm_code).result().to_vec();

    // get the local path of the compiled wasm
    // note that the file name will be the same as the root folder name
    let file_name = root_folder.file_name().unwrap().to_str().unwrap();
    let mut wasm_path = root_folder.clone();
    wasm_path.push(format!("target/wasm32-unknown-unknown/release/{}.wasm", file_name));

    // ask the template name (skip if already passed as a CLI argument)
    let template_name: String = match args.template_name {
        Some(value) => value,
        None => Prompt::new("Template name (max 32 characters):").ask()?,
    };

    // ask the template version (skip if already passed as a CLI argument)
    let template_version: u16 = match args.template_version {
        Some(value) => value,
        None => Prompt::new("Template version:")
            .with_default(0.to_string())
            .ask_parsed()?,
    };

    // ask repository info (optional)
    let mut repo_url = String::new();
    let mut commit_hash = String::new();
    let add_repo_info: YesNo = Prompt::new("Add repository info? Y/N")
        .with_default("no")
        .ask_parsed()?;
    if add_repo_info.as_bool() {
        repo_url = match args.repo_url {
            Some(value) => value,
            None => Prompt::new("Repository URL (max 255 characters):").ask()?,
        };
        commit_hash = match args.commit_hash {
            Some(value) => value,
            None => Prompt::new("Commit hash (max 32 characters):").ask()?,
        };
    }

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
        binary_sha,
        binary_url,
    };
    client.register_template(request).await?;
    println!("✅ Template registration submitted");

    Ok(())
}
