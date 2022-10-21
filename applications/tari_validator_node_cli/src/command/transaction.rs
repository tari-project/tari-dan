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

use std::path::Path;

use clap::{Args, Subcommand};
use tari_dan_engine::transaction::Transaction;
use tari_engine_types::{instruction::Instruction, TemplateAddress};
use tari_template_lib::models::ComponentAddress;
use tari_validator_node_client::{types::SubmitTransactionRequest, ValidatorNodeClient};

use crate::{account_manager::AccountFileManager, from_hex::FromHex};

#[derive(Debug, Subcommand, Clone)]
pub enum TransactionSubcommand {
    Submit(SubmitArgs),
}

#[derive(Debug, Args, Clone)]
pub struct SubmitArgs {
    #[clap(subcommand)]
    instruction: CliInstruction,
    #[clap(long, short = 'w')]
    wait_for_result: bool,
}

#[derive(Debug, Subcommand, Clone)]
pub enum CliInstruction {
    CallFunction {
        template_address: FromHex<TemplateAddress>,
        function_name: String,
    },
    CallMethod {
        template_address: FromHex<TemplateAddress>,
        component_address: FromHex<ComponentAddress>,
        method_name: String,
    },
}

impl TransactionSubcommand {
    pub async fn handle<P: AsRef<Path>>(
        self,
        base_dir: P,
        mut client: ValidatorNodeClient,
    ) -> Result<(), anyhow::Error> {
        match self {
            TransactionSubcommand::Submit(args) => handle_submit(args, base_dir, &mut client).await?,
        }
        Ok(())
    }
}

async fn handle_submit(
    args: SubmitArgs,
    base_dir: impl AsRef<Path>,
    client: &mut ValidatorNodeClient,
) -> Result<(), anyhow::Error> {
    let instruction = match args.instruction {
        CliInstruction::CallFunction {
            template_address,
            function_name,
        } => {
            Instruction::CallFunction {
                template_address: template_address.into_inner(),
                function: function_name,
                // TODO
                args: vec![],
            }
        },
        CliInstruction::CallMethod {
            template_address,
            component_address,
            method_name,
        } => {
            Instruction::CallMethod {
                template_address: template_address.into_inner(),
                component_address: component_address.into_inner(),
                method: method_name,
                // TODO
                args: vec![],
            }
        },
    };
    let account_manager = AccountFileManager::init(base_dir.as_ref().to_path_buf())?;
    let account = account_manager
        .get_active_account()
        .ok_or_else(|| anyhow::anyhow!("No active account. Use `accounts use [public key hex]` to set one."))?;

    // TODO: this is a little clunky
    let mut builder = Transaction::builder();
    builder.add_instruction(instruction).sign(&account.secret_key).fee(1);
    let transaction = builder.build();

    let request = SubmitTransactionRequest {
        instructions: transaction.instructions().to_vec(),
        signature: transaction.signature().clone(),
        fee: transaction.fee(),
        sender_public_key: transaction.sender_public_key().clone(),
        // TODO:
        num_new_components: 1,
        wait_for_result: args.wait_for_result,
    };

    if args.wait_for_result {
        println!("⏳️ Waiting for transaction result...");
    }
    let resp = client.submit_transaction(request).await?;
    println!("✅ Transaction {} submitted.", resp.hash);
    for (shard_id, _) in resp.changes {
        println!("🌼️ New component: {}", shard_id);
    }
    Ok(())
}
