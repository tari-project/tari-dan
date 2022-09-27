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

use std::error::Error;

use anyhow::anyhow;
use command::{RegisterSubcommand, RegisterTemplateArgs};
use multiaddr::{Multiaddr, Protocol};
use reqwest::Url;
use tari_dan_engine::wasm::compile::compile_template;

use crate::{cli::Cli, client::ValidatorNodeClient, command::Command};

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
            RegisterSubcommand::Node => {
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
    let tx_id = client.register().await?;
    println!("✅ Validator node registration submitted (tx_id: {})", tx_id);

    Ok(())
}

async fn handle_register_template(args: RegisterTemplateArgs, mut _client: ValidatorNodeClient) -> anyhow::Result<()> {
    // retrieve the root folder of the template
    let root_folder = args.template_code_path;
    println!("Template code path {}", root_folder.display());

    // compile the code and retrieve the binary content of the wasm
    let wasm_module = compile_template(root_folder.as_path(), &[]).unwrap();
    let wasm_code = wasm_module.code();

    // get the local path of the compiled wasm
    // note that the file name will be the same as the root folder name
    let file_name = root_folder.file_name().unwrap().to_str().unwrap();
    let mut wasm_path = root_folder.clone();
    wasm_path.push(format!("target/wasm32-unknown-unknown/release/{}.wasm", file_name));

    // display the results
    println!("Template wasm file {}", wasm_path.display());
    println!("Template code size {}", wasm_code.len());

    Ok(())
}
