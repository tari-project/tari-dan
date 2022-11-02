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

use std::{convert::TryFrom, path::Path, str::FromStr};

use clap::{Args, Subcommand};
use tari_dan_common_types::{ShardId, SubstateChange};
use tari_dan_engine::transaction::Transaction;
use tari_engine_types::{
    commit_result::{FinalizeResult, TransactionResult},
    execution_result::Type,
    instruction::Instruction,
    substate::SubstateValue,
    TemplateAddress,
};
use tari_template_lib::{args::Arg, models::ComponentAddress};
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
    #[clap(long, short = 'n')]
    num_outputs: Option<u8>,
    #[clap(long, short = 'v')]
    version: Option<u8>,
}

#[derive(Debug, Clone)]
pub enum CliArg {
    String(String),
    U64(u64),
    U32(u32),
    U16(u16),
    U8(u8),
    I64(i64),
    I32(i32),
    I16(i16),
    I8(i8),
    Bool(bool),
}

impl FromStr for CliArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(v) = s.parse::<u64>() {
            return Ok(CliArg::U64(v));
        }
        if let Ok(v) = s.parse::<u32>() {
            return Ok(CliArg::U32(v));
        }
        if let Ok(v) = s.parse::<u16>() {
            return Ok(CliArg::U16(v));
        }
        if let Ok(v) = s.parse::<u8>() {
            return Ok(CliArg::U8(v));
        }
        if let Ok(v) = s.parse::<i64>() {
            return Ok(CliArg::I64(v));
        }
        if let Ok(v) = s.parse::<i32>() {
            return Ok(CliArg::I32(v));
        }
        if let Ok(v) = s.parse::<i16>() {
            return Ok(CliArg::I16(v));
        }
        if let Ok(v) = s.parse::<i8>() {
            return Ok(CliArg::I8(v));
        }
        if let Ok(v) = s.parse::<bool>() {
            return Ok(CliArg::Bool(v));
        }
        Ok(CliArg::String(s.to_string()))
    }
}

impl CliArg {
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            CliArg::String(s) => s.as_bytes().to_vec(),
            CliArg::U64(v) => i64::try_from(*v)
                .expect("Not a valid i64 number")
                .to_le_bytes()
                .to_vec(),
            CliArg::U32(v) => i64::from(*v).to_le_bytes().to_vec(),
            CliArg::U16(v) => i64::from(*v).to_le_bytes().to_vec(),
            CliArg::U8(v) => i64::from(*v).to_le_bytes().to_vec(),
            CliArg::I64(v) => v.to_le_bytes().to_vec(),
            CliArg::I32(v) => i64::from(*v).to_le_bytes().to_vec(),
            CliArg::I16(v) => i64::from(*v).to_le_bytes().to_vec(),
            CliArg::I8(v) => i64::from(*v).to_le_bytes().to_vec(),
            CliArg::Bool(v) => i64::from(*v).to_le_bytes().to_vec(),
        }
    }
}

#[derive(Debug, Subcommand, Clone)]
pub enum CliInstruction {
    CallFunction {
        template_address: FromHex<TemplateAddress>,
        function_name: String,
        #[clap(long, short = 'a')]
        args: Vec<CliArg>,
    },
    CallMethod {
        template_address: FromHex<TemplateAddress>,
        component_address: FromHex<ComponentAddress>,
        method_name: String,
        #[clap(long, short = 'a')]
        args: Vec<CliArg>,
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
    let mut input_refs = vec![];
    let mut inputs = vec![];
    let instruction = match args.instruction {
        CliInstruction::CallFunction {
            template_address,
            function_name,
            args,
        } => Instruction::CallFunction {
            template_address: template_address.into_inner(),
            function: function_name,
            args: args.iter().map(|s| Arg::literal(s.to_bytes())).collect(),
        },
        CliInstruction::CallMethod {
            template_address,
            component_address,
            method_name,
            args,
        } => {
            input_refs.push(component_address.into_inner().into_array().into());
            // inputs.push(component_address.into_inner().into_array().into());
            Instruction::CallMethod {
                template_address: template_address.into_inner(),
                component_address: component_address.into_inner(),
                method: method_name,
                args: args.iter().map(|s| Arg::literal(s.to_bytes())).collect(),
            }
        },
    };
    let account_manager = AccountFileManager::init(base_dir.as_ref().to_path_buf())?;
    let account = account_manager
        .get_active_account()
        .ok_or_else(|| anyhow::anyhow!("No active account. Use `accounts use [public key hex]` to set one."))?;

    // TODO: this is a little clunky
    let mut builder = Transaction::builder();
    builder
        .with_input_refs(input_refs.clone())
        .with_inputs(inputs.clone())
        .with_num_outputs(args.num_outputs.unwrap_or(0))
        .add_instruction(instruction)
        .sign(&account.secret_key)
        .fee(1);
    let transaction = builder.build();
    let tx_hash = *transaction.hash();

    let mut input_data: Vec<(ShardId, SubstateChange)> =
        input_refs.iter().map(|i| (*i, SubstateChange::Exists)).collect();
    input_data.extend(inputs.iter().map(|i| (*i, SubstateChange::Destroy)));
    let request = SubmitTransactionRequest {
        instructions: transaction.instructions().to_vec(),
        signature: transaction.signature().clone(),
        fee: transaction.fee(),
        sender_public_key: transaction.sender_public_key().clone(),
        inputs: input_data,
        num_outputs: args.num_outputs.unwrap_or(0),
        wait_for_result: args.wait_for_result,
    };

    if request.inputs.is_empty() && request.num_outputs == 0 {
        println!("No inputs or outputs. This transaction will not be processed by the network.");
        return Ok(());
    }
    println!("✅ Transaction {} submitted.", tx_hash);
    if args.wait_for_result {
        println!("⏳️ Waiting for transaction result...");
        println!();
    }

    dbg!(&request);
    let resp = client.submit_transaction(request).await?;
    if let Some(result) = resp.result {
        summarize(&result);
    }
    Ok(())
}

fn summarize(result: &FinalizeResult) {
    match result.result {
        TransactionResult::Accept(ref diff) => {
            for (address, substate) in diff.up_iter() {
                println!(
                    "️🌲 New substate {} (v{})",
                    ShardId::from_address(address),
                    substate.version()
                );
                match substate.substate_value() {
                    SubstateValue::Component(component) => {
                        println!(
                            "       ▶ component ({}): {}",
                            component.module_name, component.component_address
                        );
                    },
                    SubstateValue::Resource(resource) => {
                        println!("       ▶ resource: {}", resource.address());
                    },
                    SubstateValue::Vault(vault) => {
                        println!("       ▶ vault: {} {}", vault.id(), vault.resource_address());
                    },
                }
                println!();
            }
            for address in diff.down_iter() {
                println!("🗑️ Destroyed substate {}", ShardId::from_address(address));
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
