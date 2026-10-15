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

use std::str::FromStr;

use anyhow::anyhow;
use base64::{Engine, prelude::BASE64_STANDARD};
use clap::{Args, Subcommand};
use tari_bor::encode;
use tari_ootle_address::OotleAddress;
use tari_ootle_wallet_sdk::crypto::memo::Memo;
use tari_ootle_walletd_client::{WalletDaemonClient, types::ConfidentialCreateOutputProofRequest};
use tari_template_lib::types::ResourceAddress;

#[derive(Debug, Subcommand, Clone)]
pub enum ProofsSubcommand {
    #[clap(alias = "create")]
    Generate(GenerateArgs),
}

#[derive(Debug, Args, Clone)]
pub struct GenerateArgs {
    pub amount: u64,
    pub resource_address: ResourceAddress,
    /// The address that can recover and spend the output
    pub destination_address: OotleAddress,
    /// An optional memo to include in the confidential output
    #[clap(long, short = 'm')]
    pub memo_message: Option<String>,
    #[clap(short = 'o', long)]
    pub output_type: OutputType,
}

#[derive(Debug, Clone, Default)]
pub enum OutputType {
    #[default]
    Json,
    Base64,
}

impl FromStr for OutputType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().trim() {
            "json" => Ok(OutputType::Json),
            "base64" => Ok(OutputType::Base64),
            _ => Err(anyhow::anyhow!("Invalid output type {}", s)),
        }
    }
}

impl ProofsSubcommand {
    pub async fn handle(self, mut client: WalletDaemonClient) -> anyhow::Result<()> {
        #[allow(clippy::enum_glob_use)]
        use ProofsSubcommand::*;
        match self {
            Generate(args) => {
                let resp = client
                    .create_confidential_output_proof(ConfidentialCreateOutputProofRequest {
                        amount: args.amount,
                        resource_address: args.resource_address,
                        destination_address: args.destination_address,
                        output_memo: args
                            .memo_message
                            .map(|s| Memo::new_message(s).ok_or_else(|| anyhow!("Invalid memo length")))
                            .transpose()?,
                    })
                    .await?;

                match args.output_type {
                    OutputType::Json => {
                        println!("{}", serde_json::to_string_pretty(&resp.proof)?);
                    },
                    OutputType::Base64 => {
                        let encode_proof = encode(&resp.proof)?;
                        println!("{}", BASE64_STANDARD.encode(encode_proof));
                    },
                }
            },
        }
        Ok(())
    }
}
