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
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use clap::{Args, Subcommand};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::ShardId;
use tari_dan_engine::abi::Type;
use tari_engine_types::{
    commit_result::{FinalizeResult, TransactionResult},
    instruction::Instruction,
    instruction_result::InstructionResult,
    substate::{SubstateAddress, SubstateValue},
    TemplateAddress,
};
use tari_template_lib::{
    arg,
    args::Arg,
    models::{Amount, NonFungibleAddress, NonFungibleId},
    prelude::{ComponentAddress, ResourceAddress},
};
use tari_transaction::Transaction;
use tari_transaction_manifest::parse_manifest;
use tari_utilities::hex::to_hex;
use tari_validator_node_client::{
    types::{
        GetTransactionResultRequest,
        SubmitTransactionRequest,
        SubmitTransactionResponse,
        TransactionFinalizeResult,
    },
    ValidatorNodeClient,
};

use crate::{
    command::manifest,
    component_manager::ComponentManager,
    from_hex::FromHex,
    key_manager::KeyManager,
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
    #[clap(long, short = 'r', alias = "resource")]
    pub new_resources: Vec<NewResourceOutput>,
    #[clap(long, short = 'm', alias = "mint-specific")]
    pub non_fungible_mint_outputs: Vec<SpecificNonFungibleMintOutput>,
    #[clap(long, alias = "mint-new")]
    pub new_non_fungible_outputs: Vec<NewNonFungibleMintOutput>,
    #[clap(long, alias = "new-nft-index")]
    pub new_non_fungible_index_outputs: Vec<NewNonFungibleIndexOutput>,
    #[clap(long, alias = "new-components")]
    pub new_component_outputs: Vec<NewComponentOutput>,
}

#[derive(Debug, Args, Clone)]
pub struct SubmitManifestArgs {
    manifest: PathBuf,
    /// A list of globals to be used by the manifest in the format `name=value`
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
        component_address: SubstateAddress,
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
    let request = GetTransactionResultRequest {
        hash: args.transaction_hash.into_inner(),
    };
    let resp = client.get_transaction_result(request).await?;

    if let Some(result) = resp.result {
        println!("Transaction {}", args.transaction_hash);
        println!();

        summarize_finalize_result(&result.finalize);
    } else {
        println!("Transaction not finalized",);
    }

    Ok(())
}

pub async fn handle_submit(
    args: SubmitArgs,
    base_dir: impl AsRef<Path>,
    client: &mut ValidatorNodeClient,
) -> Result<SubmitTransactionResponse, anyhow::Error> {
    let SubmitArgs { instruction, common } = args;
    let instruction = match instruction {
        CliInstruction::CallFunction {
            template_address,
            function_name,
            args,
        } => Instruction::CallFunction {
            template_address: template_address.into_inner(),
            function: function_name,
            args: args.into_iter().map(|s| s.into_arg()).collect(),
        },
        CliInstruction::CallMethod {
            component_address,
            method_name,
            args,
        } => Instruction::CallMethod {
            component_address: component_address
                .as_component_address()
                .ok_or_else(|| anyhow!("Invalid component address: {}", component_address))?,
            method: method_name,
            args: args.into_iter().map(|s| s.into_arg()).collect(),
        },
    };
    submit_transaction(vec![instruction], common, base_dir, client).await
}

async fn handle_submit_manifest(
    args: SubmitManifestArgs,
    base_dir: impl AsRef<Path>,
    client: &mut ValidatorNodeClient,
) -> Result<SubmitTransactionResponse, anyhow::Error> {
    let contents = std::fs::read_to_string(&args.manifest).map_err(|e| anyhow!("Failed to read manifest: {}", e))?;
    let instructions = parse_manifest(&contents, manifest::parse_globals(args.input_variables)?)?;
    submit_transaction(instructions, args.common, base_dir, client).await
}

pub async fn submit_transaction(
    instructions: Vec<Instruction>,
    common: CommonSubmitArgs,
    base_dir: impl AsRef<Path>,
    client: &mut ValidatorNodeClient,
) -> Result<SubmitTransactionResponse, anyhow::Error> {
    let component_manager = ComponentManager::init(base_dir.as_ref())?;
    let key_manager = KeyManager::init(base_dir)?;
    let key = key_manager
        .get_active_key()
        .ok_or_else(|| anyhow::anyhow!("No active key. Use `keys use [public key hex]` to set one."))?;

    let inputs = if common.inputs.is_empty() {
        load_inputs(&instructions, &component_manager)?
    } else {
        common.inputs
    };

    // TODO: we assume that all inputs will be consumed and produce a new output however this is only the case when the
    //       object is mutated
    let mut outputs = inputs
        .iter()
        .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version + 1))
        .collect::<Vec<_>>();

    outputs.extend(
        common
            .non_fungible_mint_outputs
            .into_iter()
            .map(|m| ShardId::from_address(&m.to_substate_address(), 0)),
    );

    outputs.extend(
        common
            .new_component_outputs
            .iter()
            .map(|c| ShardId::from_address(&c.to_substate_address(), 0)),
    );

    // Convert to shard id
    let inputs = inputs
        .into_iter()
        .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version))
        .collect::<Vec<_>>();

    summarize_request(&instructions, &inputs, &outputs, 1, common.dry_run);
    println!();

    let transaction = Transaction::builder()
        .with_instructions(instructions)
        .with_inputs(inputs)
        .with_new_outputs(common.num_outputs.unwrap_or(0))
        .with_outputs(outputs)
        .with_new_resources(
            common
                .new_resources
                .into_iter()
                .map(|r| (r.template_address, r.token_symbol))
                .collect(),
        )
        .with_new_non_fungible_outputs(
            common
                .new_non_fungible_outputs
                .into_iter()
                .map(|m| (m.resource_address, m.count))
                .collect(),
        )
        .with_new_non_fungible_index_outputs(
            common
                .new_non_fungible_index_outputs
                .into_iter()
                .map(|i| (i.parent_address, i.index))
                .collect(),
        )
        .with_new_components(
            common
                .new_component_outputs
                .into_iter()
                .map(|i| ComponentAddress::new(i.template_address, i.index))
                .collect(),
        )
        .sign(&key.secret_key)
        .build();

    if transaction.meta().involved_shards().is_empty() {
        return Err(anyhow::anyhow!(
            "No inputs or outputs, transaction will not be processed by the network"
        ));
    }

    let tx_hash = *transaction.hash();
    let request = SubmitTransactionRequest {
        transaction,
        wait_for_result: common.wait_for_result,
        is_dry_run: common.dry_run,
        wait_for_result_timeout: common.wait_for_result_timeout,
    };

    println!("✅ Transaction {} submitted.", tx_hash);
    println!();

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

    Ok(resp)
}

fn summarize_request(
    instructions: &[Instruction],
    inputs: &[ShardId],
    outputs: &[ShardId],
    fee: u64,
    is_dry_run: bool,
) {
    if is_dry_run {
        println!("NOTE: Dry run is enabled. This transaction will not be processed by the network.");
        println!();
    }
    println!("Fee: {}", fee);
    println!("Inputs:");
    if inputs.is_empty() {
        println!("  None");
    } else {
        for shard_id in inputs {
            println!("- {}", shard_id);
        }
    }
    println!();
    println!("Outputs:");
    if outputs.is_empty() {
        println!("  None");
    } else {
        for shard_id in outputs {
            println!("- {}", shard_id);
        }
    }
    println!();
    println!("🌟 Submitting instructions:");
    for instruction in instructions {
        println!("- {}", instruction);
    }
    println!();
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
    if let Some(tx_failure) = &result.transaction_failure {
        println!("Transaction failure: {:?}", tx_failure);
    }
    println!("OVERALL DECISION: {:?}", result.decision);
}

#[allow(clippy::too_many_lines)]
fn summarize_finalize_result(finalize: &FinalizeResult) {
    println!("========= Substates =========");
    match finalize.result {
        TransactionResult::Accept(ref diff) => {
            for (address, substate) in diff.up_iter() {
                println!("️🌲 UP substate {} (v{})", address, substate.version(),);
                println!("      🧩 Shard: {}", ShardId::from_address(address, substate.version()));
                match substate.substate_value() {
                    SubstateValue::Component(component) => {
                        println!("      ▶ component ({}): {}", component.module_name, address,);
                    },
                    SubstateValue::Resource(_) => {
                        println!("      ▶ resource: {}", address);
                    },
                    SubstateValue::Vault(vault) => {
                        println!("      ▶ vault: {} {}", address, vault.resource_address());
                    },
                    SubstateValue::NonFungible(_) => {
                        println!("      ▶ NFT: {}", address);
                    },
                    SubstateValue::UnclaimedConfidentialOutput(_hash) => {
                        println!("     ! layer one commitment: Should never happen");
                    },
                    SubstateValue::NonFungibleIndex(index) => {
                        let referenced_address = SubstateAddress::from(index.referenced_address().clone());
                        println!("      ▶ NFT index {} referencing {}", address, referenced_address);
                    },
                }
                println!();
            }
            for (address, version) in diff.down_iter() {
                println!("🗑️ DOWN substate {} v{}", address, version,);
                println!("      🧩 Shard: {}", ShardId::from_address(address, *version));
                println!();
            }
        },
        TransactionResult::Reject(ref reason) => {
            println!("❌️ Transaction rejected: {}", reason);
        },
    }

    println!("========= Return Values =========");
    for result in &finalize.execution_results {
        match &result.return_type {
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
            Type::Vec(ty) => {
                let mut vec_ty = String::new();
                display_vec(&mut vec_ty, ty, result).unwrap();
                match &**ty {
                    Type::Other { name } => {
                        println!("Vec<{}>: {}", name, vec_ty);
                    },
                    _ => {
                        println!("Vec<{:?}>: {}", ty, vec_ty);
                    },
                }
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

fn display_vec<W: fmt::Write>(writer: &mut W, ty: &Type, result: &InstructionResult) -> fmt::Result {
    fn stringify_slice<T: fmt::Display>(slice: &[T]) -> String {
        slice.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
    }

    match &ty {
        Type::Unit => {},
        Type::Bool => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<bool>>().unwrap()))?;
        },
        Type::I8 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<i8>>().unwrap()))?;
        },
        Type::I16 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<i16>>().unwrap()))?;
        },
        Type::I32 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<i32>>().unwrap()))?;
        },
        Type::I64 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<i64>>().unwrap()))?;
        },
        Type::I128 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<i128>>().unwrap()))?;
        },
        Type::U8 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<u8>>().unwrap()))?;
        },
        Type::U16 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<u16>>().unwrap()))?;
        },
        Type::U32 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<u32>>().unwrap()))?;
        },
        Type::U64 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<u64>>().unwrap()))?;
        },
        Type::U128 => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<u128>>().unwrap()))?;
        },
        Type::String => {
            write!(writer, "{}", result.decode::<Vec<String>>().unwrap().join(", "))?;
        },
        Type::Vec(ty) => {
            let mut vec_ty = String::new();
            display_vec(&mut vec_ty, ty, result)?;
            match &**ty {
                Type::Other { name } => {
                    write!(writer, "Vec<{}>: {}", name, vec_ty)?;
                },
                _ => {
                    write!(writer, "Vec<{:?}>: {}", ty, vec_ty)?;
                },
            }
        },
        Type::Other { name } if name == "Amount" => {
            write!(writer, "{}", stringify_slice(&result.decode::<Vec<Amount>>().unwrap()))?;
        },
        Type::Other { name } if name == "NonFungibleId" => {
            write!(
                writer,
                "{}",
                stringify_slice(&result.decode::<Vec<NonFungibleId>>().unwrap())
            )?;
        },
        Type::Other { .. } => {
            write!(writer, "{}", to_hex(&result.raw))?;
        },
    }
    Ok(())
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
    Amount(i64),
    NonFungibleId(NonFungibleId),
    SubstateAddress(SubstateAddress),
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

        if let Ok(v) = s.parse::<SubstateAddress>() {
            return Ok(CliArg::SubstateAddress(v));
        }

        if let Some(("nft", nft_id)) = s.split_once('_') {
            match NonFungibleId::try_from_canonical_string(nft_id) {
                Ok(v) => {
                    return Ok(CliArg::NonFungibleId(v));
                },
                Err(e) => {
                    eprintln!(
                        "WARN: '{}' is not a valid NonFungibleId ({:?}) and will be interpreted as a string",
                        s, e
                    );
                },
            }
        }

        if let Some(("amount", amount)) = s.split_once('_') {
            match amount.parse::<i64>() {
                Ok(number) => {
                    return Ok(CliArg::Amount(number));
                },
                Err(e) => {
                    eprintln!(
                        "WARN: '{}' is not a valid Amount ({:?}) and will be interpreted as a string",
                        s, e
                    );
                },
            }
        }

        Ok(CliArg::String(s.to_string()))
    }
}

impl CliArg {
    pub fn into_arg(self) -> Arg {
        match self {
            CliArg::String(s) => arg!(s),
            CliArg::U64(v) => arg!(v),
            CliArg::U32(v) => arg!(v),
            CliArg::U16(v) => arg!(v),
            CliArg::U8(v) => arg!(v),
            CliArg::I64(v) => arg!(v),
            CliArg::I32(v) => arg!(v),
            CliArg::I16(v) => arg!(v),
            CliArg::I8(v) => arg!(v),
            CliArg::Bool(v) => arg!(v),
            CliArg::SubstateAddress(v) => arg!(v.to_canonical_hash()),
            CliArg::NonFungibleId(v) => arg!(v),
            CliArg::Amount(v) => arg!(Amount(v)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewResourceOutput {
    pub template_address: TemplateAddress,
    pub token_symbol: String,
}

impl FromStr for NewResourceOutput {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (template_address, token_symbol) = s
            .split_once(':')
            .ok_or_else(|| anyhow!("Expected template address and token symbol"))?;
        let template_address = TemplateAddress::from_hex(template_address)?;
        Ok(NewResourceOutput {
            template_address,
            token_symbol: token_symbol.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SpecificNonFungibleMintOutput {
    pub resource_address: ResourceAddress,
    pub non_fungible_id: NonFungibleId,
}

impl SpecificNonFungibleMintOutput {
    pub fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::NonFungible(NonFungibleAddress::new(
            self.resource_address,
            self.non_fungible_id.clone(),
        ))
    }
}

impl FromStr for SpecificNonFungibleMintOutput {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (resource_address, non_fungible_id) = s
            .split_once(',')
            .ok_or_else(|| anyhow!("Expected resource address and non-fungible id"))?;
        let resource_address = SubstateAddress::from_str(resource_address)?;
        let resource_address = resource_address
            .as_resource_address()
            .ok_or_else(|| anyhow!("Expected resource address but got {}", resource_address))?;
        let non_fungible_id = non_fungible_id
            .split_once('_')
            .map(|(_, b)| b)
            .unwrap_or(non_fungible_id);
        let non_fungible_id =
            NonFungibleId::try_from_canonical_string(non_fungible_id).map_err(|e| anyhow!("{:?}", e))?;
        Ok(SpecificNonFungibleMintOutput {
            resource_address,
            non_fungible_id,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NewNonFungibleMintOutput {
    pub resource_address: ResourceAddress,
    pub count: u8,
}

impl FromStr for NewNonFungibleMintOutput {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (resource_address, count_str) = s.split_once(',').unwrap_or((s, "1"));
        let resource_address = SubstateAddress::from_str(resource_address)?;
        let resource_address = resource_address
            .as_resource_address()
            .ok_or_else(|| anyhow!("Expected resource address but got {}", resource_address))?;
        Ok(NewNonFungibleMintOutput {
            resource_address,
            count: count_str.parse()?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NewNonFungibleIndexOutput {
    pub parent_address: ResourceAddress,
    pub index: u64,
}

impl FromStr for NewNonFungibleIndexOutput {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (parent_address, index_str) = s.split_once(',').unwrap_or((s, "0"));
        let parent_address = SubstateAddress::from_str(parent_address)?;
        let parent_address = parent_address
            .as_resource_address()
            .ok_or_else(|| anyhow!("Expected resource address but got {}", parent_address))?;
        Ok(NewNonFungibleIndexOutput {
            parent_address,
            index: index_str.parse()?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NewComponentOutput {
    pub template_address: TemplateAddress,
    pub index: u64,
}

impl NewComponentOutput {
    pub fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::Component(ComponentAddress::new(self.template_address, self.index))
    }
}

impl FromStr for NewComponentOutput {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (template_address, index_str) = s
            .split_once(',')
            .ok_or_else(|| anyhow!("Expected template address and index"))?;
        let template_address = TemplateAddress::from_hex(template_address)?;
        Ok(NewComponentOutput {
            template_address,
            index: index_str.parse()?,
        })
    }
}
