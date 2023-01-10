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
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use clap::{Args, Subcommand};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::ShardId;
use tari_dan_engine::transaction::Transaction;
use tari_engine_types::{
    commit_result::{FinalizeResult, TransactionResult},
    execution_result::Type,
    instruction::Instruction,
    substate::{SubstateAddress, SubstateValue},
    TemplateAddress,
};
use tari_template_lib::{
    arg,
    args::Arg,
    models::{Amount, ComponentAddress},
};
use tari_transaction_manifest::parse_manifest;
use tari_utilities::hex::to_hex;
use tari_validator_node_client::{
    types::{GetTransactionRequest, SubmitTransactionRequest, SubmitTransactionResponse, TransactionFinalizeResult},
    ValidatorNodeClient,
};

use crate::{
    account_manager::AccountFileManager,
    command::manifest,
    component_manager::ComponentManager,
    from_hex::FromHex,
    versioned_substate_address::VersionedSubstateAddress,
};

#[derive(Debug, Subcommand, Clone)]
pub enum TransactionSubcommand {
    Get(GetArgs),
    Submit(SubmitArgs),
    SubmitManifest(SubmitManifestArgs),
}

#[derive(Debug, Args, Clone)]
pub struct GetArgs {
    transaction_hash: FromHex<FixedHash>,
}

#[derive(Debug, Args, Clone)]
pub struct SubmitArgs {
    #[clap(subcommand)]
    pub instruction: CliInstruction,
    #[clap(flatten)]
    pub common: CommonSubmitArgs,
}

#[derive(Debug, Args, Clone)]
pub struct CommonSubmitArgs {
    #[clap(long, short = 'w')]
    pub wait_for_result: bool,
    /// Timeout in seconds
    #[clap(long, short = 't')]
    pub wait_for_result_timeout: Option<u64>,
    #[clap(long, short = 'n')]
    pub num_outputs: Option<u8>,
    #[clap(long, short = 'i')]
    pub inputs: Vec<VersionedSubstateAddress>,
    #[clap(long, short = 'v')]
    pub version: Option<u8>,
    #[clap(long, short = 'd')]
    pub dump_outputs_into: Option<String>,
    #[clap(long, short = 'a')]
    pub account_template_address: Option<String>,
    #[clap(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args, Clone)]
pub struct SubmitManifestArgs {
    manifest: PathBuf,
    #[clap(long, short = 'g')]
    input_variables: Vec<String>,
    #[clap(flatten)]
    common: CommonSubmitArgs,
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
            TransactionSubcommand::Submit(args) => {
                handle_submit(args, base_dir, &mut client).await?;
            },
            TransactionSubcommand::SubmitManifest(args) => {
                handle_submit_manifest(args, base_dir, &mut client).await?;
            },
            TransactionSubcommand::Get(args) => handle_get(args, &mut client).await?,
        }
        Ok(())
    }
}

async fn handle_get(args: GetArgs, client: &mut ValidatorNodeClient) -> Result<(), anyhow::Error> {
    let request = GetTransactionRequest {
        hash: args.transaction_hash.into_inner(),
    };
    let resp = client.get_transaction_result(request).await?;

    if let Some(result) = resp.result {
        println!("Transaction {}", args.transaction_hash);
        println!();

        summarize_finalize_result(&result);
    } else {
        println!("Transaction not finalized",);
    }

    Ok(())
}

pub async fn handle_submit(
    args: SubmitArgs,
    base_dir: impl AsRef<Path>,
    client: &mut ValidatorNodeClient,
) -> Result<Option<SubmitTransactionResponse>, anyhow::Error> {
    let SubmitArgs { instruction, common } = args;
    let instruction = match instruction {
        CliInstruction::CallFunction {
            template_address,
            function_name,
            args,
        } => Instruction::CallFunction {
            template_address: template_address.into_inner(),
            function: function_name,
            args: args.iter().map(|s| s.to_arg()).collect(),
        },
        CliInstruction::CallMethod {
            component_address,
            method_name,
            args,
        } => Instruction::CallMethod {
            component_address: component_address.into_inner(),
            method: method_name,
            args: args.iter().map(|s| s.to_arg()).collect(),
        },
    };

    submit_transaction(vec![instruction], common, base_dir, client).await
}

async fn handle_submit_manifest(
    args: SubmitManifestArgs,
    base_dir: impl AsRef<Path>,
    client: &mut ValidatorNodeClient,
) -> Result<Option<SubmitTransactionResponse>, anyhow::Error> {
    let contents = std::fs::read_to_string(&args.manifest).map_err(|e| anyhow!("Failed to read manifest: {}", e))?;
    let instructions = parse_manifest(&contents, manifest::parse_globals(args.input_variables)?)?;
    submit_transaction(instructions, args.common, base_dir, client).await
}

async fn submit_transaction(
    instructions: Vec<Instruction>,
    common: CommonSubmitArgs,
    base_dir: impl AsRef<Path>,
    client: &mut ValidatorNodeClient,
) -> Result<Option<SubmitTransactionResponse>, anyhow::Error> {
    let component_manager = ComponentManager::init(base_dir.as_ref())?;
    let account_manager = AccountFileManager::init(base_dir.as_ref().to_path_buf())?;
    let account = account_manager
        .get_active_account()
        .ok_or_else(|| anyhow::anyhow!("No active account. Use `accounts use [public key hex]` to set one."))?;

    let inputs = if common.inputs.is_empty() {
        load_inputs(&instructions, &component_manager)?
    } else {
        common.inputs
    };

    // TODO: we assume that all inputs will be consumed and produce a new output however this is only the case when the
    //       object is mutated
    let outputs = inputs
        .iter()
        .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version + 1))
        .collect();

    // Convert to shard id
    let inputs = inputs
        .into_iter()
        .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version))
        .collect();

    let mut builder = Transaction::builder();

    builder
        .with_instructions(instructions)
        .with_inputs(inputs)
        .with_new_outputs(common.num_outputs.unwrap_or(0))
        .with_outputs(outputs)
        .with_fee(1)
        .sign(&account.secret_key);

    let transaction = builder.build();

    let inputs = transaction
        .meta()
        .involved_objects_iter()
        .map(|(shard_id, (change, _))| (*shard_id, *change))
        .collect();

    let request = SubmitTransactionRequest {
        // TODO: just pass the whole transaction in this request
        // transaction,
        instructions: transaction.instructions().to_vec(),
        signature: transaction.signature().clone(),
        fee: transaction.fee(),
        sender_public_key: transaction.sender_public_key().clone(),
        inputs,
        num_outputs: common.num_outputs.unwrap_or(0),
        wait_for_result: common.wait_for_result,
        is_dry_run: common.dry_run,
        wait_for_result_timeout: common.wait_for_result_timeout,
    };

    if request.inputs.is_empty() && request.num_outputs == 0 {
        println!("No inputs or outputs. This transaction will not be processed by the network.");
        return Ok(None);
    }
    println!("Request:");
    println!("{}", serde_json::to_string_pretty(&request).unwrap());
    println!();

    println!("🌟 Submitting instructions:");
    for instruction in &request.instructions {
        println!("- {}", instruction);
    }
    println!();

    println!("✅ Transaction {} submitted.", transaction.hash());
    let timer = Instant::now();
    if common.wait_for_result {
        println!("⏳️ Waiting for transaction result...");
        println!();
    }

    // dbg!(&request);
    let resp = client.submit_transaction(request).await?;
    if let Some(result) = &resp.result {
        if let Some(diff) = result.finalize.result.accept() {
            component_manager.commit_diff(diff)?;
        }
        summarize(result, timer.elapsed());
    }
    Ok(Some(resp))
}

#[allow(clippy::too_many_lines)]
fn summarize(result: &TransactionFinalizeResult, time_taken: Duration) {
    println!("✅️ Transaction finalized",);
    println!();
    println!("Epoch: {}", result.qc.epoch());
    println!("Payload height: {}", result.qc.payload_height());
    println!("Signed by: {} validator nodes", result.qc.validators_metadata().len());
    println!();

    summarize_finalize_result(&result.finalize);

    println!();
    println!("========= Pledges =========");
    for p in result.qc.all_shard_pledges().iter() {
        println!("Shard:{} Pledge:{}", p.shard_id, p.pledge.current_state.as_str());
    }

    println!();
    println!("Time taken: {:?}", time_taken);
    println!();
    println!("OVERALL DECISION: {:?}", result.decision);
}

fn summarize_finalize_result(finalize: &FinalizeResult) {
    println!("========= Substates =========");
    match finalize.result {
        TransactionResult::Accept(ref diff) => {
            for (address, substate) in diff.up_iter() {
                println!("️🌲 UP substate {} (v{})", address, substate.version());
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
            for (address, version) in diff.down_iter() {
                println!("🗑️ DOWN substate {} v{}", address, version);
                println!();
            }
        },
        TransactionResult::Reject(ref reason) => {
            println!("❌️ Transaction rejected: {}", reason);
        },
    }

    println!("========= Return Values =========");
    for result in &finalize.execution_results {
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
            Type::Other { ref name } if name == "Amount" => {
                println!("{}: {}", name, result.decode::<Amount>().unwrap());
            },
            Type::Other { ref name } => {
                println!("{}: {}", name, to_hex(&result.raw));
            },
        }
    }

    println!();
    println!("========= LOGS =========");
    for log in &finalize.logs {
        println!("{}", log);
    }
}

fn load_inputs(
    instructions: &[Instruction],
    component_manager: &ComponentManager,
) -> Result<Vec<VersionedSubstateAddress>, anyhow::Error> {
    let mut inputs = Vec::new();
    for instruction in instructions {
        if let Instruction::CallMethod { component_address, .. } = instruction {
            let addr = SubstateAddress::Component(*component_address);
            if inputs.iter().any(|a: &VersionedSubstateAddress| a.address == addr) {
                continue;
            }
            let component = component_manager
                .get_root_substate(&addr)?
                .ok_or_else(|| anyhow!("Component {} not found", component_address))?;
            println!("Loaded inputs");
            println!("- {} v{}", addr, component.latest_version());
            inputs.push(VersionedSubstateAddress {
                address: addr,
                version: component.latest_version(),
            });
            for child in component.get_children() {
                println!("  - {} v{}", child.address, child.version);
            }
            inputs.extend(component.get_children());
        }
    }
    Ok(inputs)
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
    pub fn to_arg(&self) -> Arg {
        match self {
            CliArg::String(s) => arg!(s),
            CliArg::U64(v) => arg!(*v),
            CliArg::U32(v) => arg!(*v),
            CliArg::U16(v) => arg!(*v),
            CliArg::U8(v) => arg!(*v),
            CliArg::I64(v) => arg!(*v),
            CliArg::I32(v) => arg!(*v),
            CliArg::I16(v) => arg!(*v),
            CliArg::I8(v) => arg!(*v),
            CliArg::Bool(v) => arg!(*v),
        }
    }
}
