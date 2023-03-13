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
    collections::HashMap,
    fmt,
    fs,
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use clap::{Args, Subcommand};
use tari_common_types::types::FixedHash;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_wallet_sdk::models::{ConfidentialProofId, VersionedSubstateAddress};
use tari_engine_types::{
    commit_result::{FinalizeResult, TransactionResult},
    execution_result::{ExecutionResult, Type},
    instruction::Instruction,
    substate::{SubstateAddress, SubstateValue},
    TemplateAddress,
};
use tari_template_lib::{
    arg,
    args,
    args::Arg,
    models::{Amount, NonFungibleAddress, NonFungibleId},
    prelude::{ComponentAddress, ResourceAddress},
};
use tari_transaction_manifest::{parse_manifest, ManifestValue};
use tari_utilities::{hex::to_hex, ByteArray};
use tari_wallet_daemon_client::{
    types::{
        ProofsGenerateRequest,
        TransactionGetResultRequest,
        TransactionSubmitRequest,
        TransactionSubmitResponse,
        TransactionWaitResultRequest,
        TransactionWaitResultResponse,
    },
    WalletDaemonClient,
};

use crate::{from_base64::FromBase64, from_hex::FromHex};

#[derive(Debug, Subcommand, Clone)]
pub enum TransactionSubcommand {
    Get(GetArgs),
    Submit(SubmitArgs),
    SubmitManifest(SubmitManifestArgs),
    Send(SendArgs),
    ConfidentialTransfer(ConfidentialTransferArgs),
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
    /// Timeout in seconds
    #[clap(long, short = 't', alias = "wait-timeout")]
    pub wait_for_result_timeout_secs: Option<u64>,
    #[clap(long, short = 'n')]
    pub num_outputs: Option<u8>,
    #[clap(long, short = 'i')]
    pub inputs: Vec<VersionedSubstateAddress>,
    #[clap(long, short = 'o')]
    pub override_inputs: Option<bool>,
    #[clap(long, short = 'v')]
    pub version: Option<u8>,
    #[clap(long, short = 'd')]
    pub dump_outputs_into: Option<String>,
    #[clap(long)]
    pub dry_run: bool,
    #[clap(long, short = 'm', alias = "mint-specific")]
    pub non_fungible_mint_outputs: Vec<SpecificNonFungibleMintOutput>,
    #[clap(long, alias = "mint-new")]
    pub new_non_fungible_outputs: Vec<NewNonFungibleMintOutput>,
    #[clap(long, alias = "new-nft-index")]
    pub new_non_fungible_index_outputs: Vec<NewNonFungibleIndexOutput>,
    #[clap(long)]
    pub fee: Option<u64>,
}

#[derive(Debug, Args, Clone)]
pub struct SubmitManifestArgs {
    manifest: PathBuf,
    #[clap(long, short = 'g')]
    input_variables: Vec<String>,
    #[clap(flatten)]
    common: CommonSubmitArgs,
}

#[derive(Debug, Args, Clone)]
pub struct SendArgs {
    source_account_name: String,
    amount: u32,
    resource_address: ResourceAddress,
    dest_address: ComponentAddress,
    #[clap(flatten)]
    common: CommonSubmitArgs,
}

#[derive(Debug, Args, Clone)]
pub struct ConfidentialTransferArgs {
    source_account_name: String,
    amount: u32,
    destination_account: ComponentAddress,
    destination_stealth_public_key: FromBase64<Vec<u8>>,
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
    pub async fn handle(self, mut client: WalletDaemonClient) -> Result<(), anyhow::Error> {
        match self {
            TransactionSubcommand::Submit(args) => {
                handle_submit(args, &mut client).await?;
            },
            TransactionSubcommand::SubmitManifest(args) => {
                handle_submit_manifest(args, &mut client).await?;
            },
            TransactionSubcommand::Get(args) => handle_get(args, &mut client).await?,
            TransactionSubcommand::Send(args) => {
                handle_send(args, &mut client).await?;
            },
            TransactionSubcommand::ConfidentialTransfer(args) => {
                handle_confidential_transfer(args, &mut client).await?;
            },
        }
        Ok(())
    }
}

async fn handle_get(args: GetArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let request = TransactionGetResultRequest {
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
    client: &mut WalletDaemonClient,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
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

    submit_transaction(vec![instruction], common, None, client).await
}

async fn handle_submit_manifest(
    args: SubmitManifestArgs,
    client: &mut WalletDaemonClient,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let contents = fs::read_to_string(&args.manifest).map_err(|e| anyhow!("Failed to read manifest: {}", e))?;
    let instructions = parse_manifest(&contents, parse_globals(args.input_variables)?)?;
    submit_transaction(instructions, args.common, None, client).await
}

pub async fn submit_transaction(
    instructions: Vec<Instruction>,
    common: CommonSubmitArgs,
    proof_id: Option<ConfidentialProofId>,
    client: &mut WalletDaemonClient,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let request = TransactionSubmitRequest {
        // Sign with the active key
        signing_key_index: None,
        instructions,
        fee: common.fee.unwrap_or(1),
        inputs: common.inputs,
        override_inputs: common.override_inputs.unwrap_or_default(),
        new_outputs: common.num_outputs.unwrap_or(0),
        specific_non_fungible_outputs: common
            .non_fungible_mint_outputs
            .into_iter()
            .map(|m| (m.resource_address, m.non_fungible_id))
            .collect(),
        new_non_fungible_outputs: common
            .new_non_fungible_outputs
            .into_iter()
            .map(|m| (m.resource_address, m.count))
            .collect(),
        new_non_fungible_index_outputs: common
            .new_non_fungible_index_outputs
            .into_iter()
            .map(|i| (i.parent_address, i.index))
            .collect(),
        is_dry_run: common.dry_run,
        proof_id,
    };

    if request.inputs.is_empty() &&
        request.new_outputs == 0 &&
        request.specific_non_fungible_outputs.is_empty() &&
        request.new_non_fungible_outputs.is_empty() &&
        request.new_non_fungible_index_outputs.is_empty()
    {
        return Err(anyhow::anyhow!(
            "No inputs or outputs, transaction will not be processed by the network"
        ));
    }

    let timer = Instant::now();

    let resp = client.submit_transaction(&request).await?;
    println!();
    println!("✅ Transaction {} submitted.", resp.hash);
    println!();
    // TODO: Would be great if we could display the Substate addresses as well as shard ids
    summarize_request(&request, &resp.inputs, &resp.outputs);

    println!();
    println!("⏳️ Waiting for transaction result...");
    println!();
    let wait_resp = client
        .wait_transaction_result(TransactionWaitResultRequest {
            hash: resp.hash,
            timeout_secs: common.wait_for_result_timeout_secs,
        })
        .await?;
    if wait_resp.timed_out {
        println!(
            "⏳️ Transaction result not available after {} seconds.",
            common.wait_for_result_timeout_secs.unwrap_or(0)
        );
        println!();
    } else {
        summarize(&wait_resp, timer.elapsed());
    }

    Ok(resp)
}

pub async fn handle_send(
    args: SendArgs,
    client: &mut WalletDaemonClient,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let SendArgs {
        source_account_name,
        amount,
        resource_address,
        dest_address,
        common,
    } = args;

    let source_address = client.get_by_name(source_account_name).await?;
    let source_component_address = source_address
        .account_address
        .as_component_address()
        .ok_or_else(|| anyhow!("Invalid component address for source address"))?;

    let instructions = vec![
        Instruction::CallMethod {
            component_address: source_component_address,
            method: String::from("withdraw"),
            args: args![resource_address, Amount::from(amount)], // amount is u32
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"bucket".to_vec(),
        },
        Instruction::CallMethod {
            component_address: dest_address,
            method: String::from("deposit"),
            args: args![Variable("bucket")],
        },
    ];

    submit_transaction(instructions, common, None, client).await
}

pub async fn handle_confidential_transfer(
    args: ConfidentialTransferArgs,
    client: &mut WalletDaemonClient,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let ConfidentialTransferArgs {
        source_account_name,
        amount,
        destination_account,
        destination_stealth_public_key,
        common,
    } = args;

    let source_address = client.get_by_name(source_account_name.clone()).await?;
    let source_component_address = source_address
        .account_address
        .as_component_address()
        .ok_or_else(|| anyhow!("Invalid component address for source address"))?;
    let destination_stealth_public_key = RistrettoPublicKey::from_bytes(&destination_stealth_public_key.into_inner())?;

    let proof_generate_req = ProofsGenerateRequest {
        amount: Amount::from(amount),
        source_account_name,
        destination_account,
        destination_stealth_public_key,
    };
    let proof_generate_resp = client.create_transfer_proof(proof_generate_req).await?;
    let withdraw_proof = proof_generate_resp.proof;
    let proof_id = proof_generate_resp.proof_id;

    let instructions = vec![
        Instruction::CallMethod {
            component_address: source_component_address,
            method: String::from("withdraw_confidential"),
            args: args![withdraw_proof],
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"bucket".to_vec(),
        },
        Instruction::CallMethod {
            component_address: destination_account,
            method: String::from("deposit"),
            args: args![Variable("bucket")],
        },
    ];

    submit_transaction(instructions, common, Some(proof_id), client).await
}

fn summarize_request(request: &TransactionSubmitRequest, inputs: &[ShardId], outputs: &[ShardId]) {
    if request.is_dry_run {
        println!("NOTE: Dry run is enabled. This transaction will not be processed by the network.");
        println!();
    }
    println!("Fee: {}", request.fee);
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
    if outputs.is_empty() && request.new_outputs == 0 {
        println!("  None");
    } else {
        for shard_id in outputs {
            println!("- {}", shard_id);
        }
        println!("- {} new output(s)", request.new_outputs);
    }
    println!();
    println!("🌟 Submitting instructions:");
    for instruction in &request.instructions {
        println!("- {}", instruction);
    }
    println!();
}

#[allow(clippy::too_many_lines)]
fn summarize(result: &TransactionWaitResultResponse, time_taken: Duration) {
    println!("✅️ Transaction finalized");
    println!();
    if let Some(qc) = result.qcs.first() {
        println!("Epoch: {}", qc.epoch());
        println!("Payload height: {}", qc.payload_height());
        println!("Signed by: {} validator nodes", qc.validators_metadata().len());
    } else {
        println!("No QC");
    }
    println!();

    summarize_finalize_result(result.result.as_ref().unwrap());

    if let Some(qc) = result.qcs.first() {
        println!();
        println!("========= Pledges =========");
        for p in qc.all_shard_pledges().iter() {
            println!("Shard:{} Pledge:{}", p.shard_id, p.pledge.current_state.as_str());
        }
    }

    println!();
    println!("Time taken: {:?}", time_taken);
    println!();
    if let Some(qc) = result.qcs.first() {
        println!("OVERALL DECISION: {:?}", qc.decision());
    } else {
        println!("STATUS: {:?}", result.status);
    }
}

pub fn summarize_finalize_result(finalize: &FinalizeResult) {
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
                    SubstateValue::UnclaimedConfidentialOutput(_) => {
                        println!("      ▶ Layer 1 commitment: {}", address);
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
    print_execution_results(&finalize.execution_results);

    println!();
    println!("========= LOGS =========");
    for log in &finalize.logs {
        println!("{}", log);
    }
}

fn display_vec<W: fmt::Write>(writer: &mut W, ty: &Type, result: &ExecutionResult) -> fmt::Result {
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

pub fn print_execution_results(results: &[ExecutionResult]) {
    for result in results {
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
        }
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

fn parse_globals(globals: Vec<String>) -> Result<HashMap<String, ManifestValue>, anyhow::Error> {
    let mut result = HashMap::with_capacity(globals.len());
    for global in globals {
        let (name, value) = global
            .split_once('=')
            .ok_or_else(|| anyhow!("Invalid global: {}", global))?;
        let value = value
            .parse()
            .map_err(|err| anyhow!("Failed to parse global '{}': {}", name, err))?;
        result.insert(name.to_string(), value);
    }
    Ok(result)
}
