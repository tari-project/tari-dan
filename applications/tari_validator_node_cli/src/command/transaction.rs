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
use tari_dan_common_types::ShardId;
use tari_dan_engine::transaction::Transaction;
use tari_engine_types::{
    commit_result::{FinalizeResult, TransactionResult},
    execution_result::Type,
    instruction::Instruction,
    substate::SubstateValue,
    TemplateAddress,
};
use tari_template_lib::models::ComponentAddress;
use tari_utilities::hex::to_hex;
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
    let mut inputs = vec![];
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
            inputs.push(component_address.into_inner().into_array().into());
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
        inputs,
        // TODO:
        num_new_components: 1,
        wait_for_result: args.wait_for_result,
    };

    if args.wait_for_result {
        println!("⏳️ Waiting for transaction result...");
    }
    let resp = client.submit_transaction(request).await?;
    println!("✅ Transaction {} submitted.", resp.hash);
    println!();
    if let Some(result) = resp.result {
        summarize(&result);
    }
    Ok(())
}

fn summarize(result: &FinalizeResult) {
    match result.result {
        TransactionResult::Accept(ref diff) => {
            for (address, substate) in diff.up_iter() {
                println!("️🌲 New substate {}", ShardId::from(address.into_shard_id()));
                match substate.substate_value() {
                    SubstateValue::Component(component) => {
                        println!(
                            "           component ({}): {}",
                            component.module_name, component.component_address
                        );
                    },
                    SubstateValue::Resource(resource) => {
                        println!("           resource: {}", resource.address());
                    },
                }
                println!();
            }
            for address in diff.down_iter() {
                println!("🗑️ Destroyed substate {}", ShardId::from(address.into_shard_id()));
                println!();
            }
        },
        TransactionResult::Reject(ref reject) => {
            println!("❌️ Transaction rejected: {}", reject.reason);
        },
    }
    println!("========= Return Values =========");
    for result in &result.execution_results {
        match result.return_type {
            Type::Unit => {},
            Type::Bool => {
                println!("bool: {}", result.decode::<bool>().unwrap());
            },
            Type::I8 => {
                println!("i8: {}", result.decode::<i8>().unwrap());
            },
            Type::I16 => {
                println!("i16: {}", result.decode::<i16>().unwrap());
            },
            Type::I32 => {
                println!("i32: {}", result.decode::<i32>().unwrap());
            },
            Type::I64 => {
                println!("i64: {}", result.decode::<i64>().unwrap());
            },
            Type::I128 => {
                println!("i128: {}", result.decode::<i128>().unwrap());
            },
            Type::U8 => {
                println!("u8: {}", result.decode::<u8>().unwrap());
            },
            Type::U16 => {
                println!("u16: {}", result.decode::<u16>().unwrap());
            },
            Type::U32 => {
                println!("u32: {}", result.decode::<u32>().unwrap());
            },
            Type::U64 => {
                println!("u64: {}", result.decode::<u64>().unwrap());
            },
            Type::U128 => {
                println!("u128: {}", result.decode::<u128>().unwrap());
            },
            Type::String => {
                println!("string: {}", result.decode::<String>().unwrap());
            },
            Type::Other { ref name } => {
                println!("{}: {}", name, to_hex(&result.raw));
            },
        }
    }

    println!();
    println!("========= LOGS =========");
    for log in &result.logs {
        println!("{}", log);
    }
}
