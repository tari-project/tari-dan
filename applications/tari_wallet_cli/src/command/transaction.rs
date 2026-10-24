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
    fs,
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use clap::{Args, Subcommand};
use tari_bor::decode_exact;
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason, TransactionResult},
    parse_template_address,
    substate::{SubstateDiff, SubstateId, SubstateValue},
};
use tari_ootle_address::OotleAddress;
use tari_ootle_common_types::{Epoch, InputDeclaration, SubstateAddress};
use tari_ootle_transaction::{
    Instruction,
    Transaction,
    TransactionId,
    UnsignedTransaction,
    args,
    args::InstructionArg,
    call_arg,
};
use tari_ootle_wallet_sdk::{apis::confidential_transfer::UtxoInputSelection, crypto::memo::Memo};
use tari_ootle_walletd_client::{
    ComponentAddressOrName,
    WalletDaemonClient,
    types::{
        AccountGetResponse,
        AccountsTransferRequest,
        ConfidentialTransferRequest,
        SettingsGetResponse,
        TransactionGetResultRequest,
        TransactionSubmitDryRunRequest,
        TransactionSubmitRequest,
        TransactionWaitResultRequest,
        TransactionWaitResultResponse,
    },
};
use tari_template_lib::types::{
    NonFungibleAddress,
    NonFungibleId,
    ResourceAddress,
    TemplateAddress,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    crypto::RistrettoPublicKeyBytes,
};
use tari_transaction_manifest::{ManifestValue, parse_manifest};

use crate::from_hex::FromHex;

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
    transaction_id: FromHex<TransactionId>,
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
    /// Substates to declare as transaction inputs (`<id>[:<version>|:?][:read|:write]`). May be
    /// repeated. Suffixes are parsed from the right (`write` by default), so a string NFT id
    /// containing `:` requires an explicit `:?` or `:<version>` (e.g. `nft_<res>_str_a:7:?`).
    #[clap(long, short = 'i')]
    pub inputs: Vec<InputDeclaration>,
    #[clap(long, alias = "autofill")]
    pub detect_inputs: Option<bool>,
    #[clap(long, short = 'v')]
    pub version: Option<u8>,
    #[clap(long, short = 'd')]
    pub dump_outputs_into: Option<ComponentAddressOrName>,
    #[clap(long)]
    pub dry_run: bool,
    #[clap(long)]
    pub max_fee: u64,
    #[clap(long, short = 'f', alias = "fee-account")]
    pub fee_account: Option<ComponentAddressOrName>,
    #[clap(long)]
    pub min_epoch: Option<u64>,
    /// The last epoch this transaction may be sequenced in. Defaults to the wallet daemon's
    /// configured window past the current epoch.
    #[clap(long)]
    pub max_epoch: Option<u64>,
}

impl CommonSubmitArgs {
    /// The caller's explicit `max_epoch`, or the daemon's default window past the current epoch.
    /// Without either — the daemon could not reach its indexer and no `--max-epoch` was given —
    /// there is no window the network is known to accept, so fail rather than guess.
    fn resolve_max_epoch(&self, current_epoch: Option<Epoch>, default_validity_epochs: u64) -> anyhow::Result<Epoch> {
        if let Some(max_epoch) = self.max_epoch {
            return Ok(Epoch(max_epoch));
        }
        let current_epoch = current_epoch.ok_or_else(|| {
            anyhow!(
                "The wallet daemon could not reach its indexer, so the current epoch is unknown. Pass --max-epoch to \
                 choose the transaction's validity window explicitly."
            )
        })?;
        Ok(Epoch(current_epoch.as_u64().saturating_add(default_validity_epochs)))
    }
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
    amount: u64,
    resource_address: ResourceAddress,
    destination_public_key: FromHex<Vec<u8>>,
    #[clap(flatten)]
    common: CommonSubmitArgs,
    source_account_name: Option<ComponentAddressOrName>,
}

#[derive(Debug, Args, Clone)]
pub struct ConfidentialTransferArgs {
    amount: u64,
    destination_address: OotleAddress,
    #[clap(flatten)]
    common: CommonSubmitArgs,
    #[clap(long, short = 'a', alias = "account")]
    source_account: Option<ComponentAddressOrName>,
    /// The address of the resource to send. If not provided, use the default Tari confidential resource
    #[clap(long)]
    resource_address: Option<ResourceAddress>,
    /// An optional memo to include in the confidential output
    #[clap(long, short = 'm')]
    memo_message: Option<String>,
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
        component_address: SubstateId,
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
        transaction_id: args.transaction_id.into_inner(),
    };
    let resp = client.get_transaction_result(request).await?;

    if let Some(result) = resp.result {
        println!("Transaction {}", args.transaction_id);
        println!();

        summarize_finalize_result(&result);
    } else {
        println!("Transaction not finalized",);
    }

    Ok(())
}

pub async fn handle_submit(args: SubmitArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let SubmitArgs { instruction, common } = args;
    let instruction = match instruction {
        CliInstruction::CallFunction {
            template_address,
            function_name,
            args,
        } => Instruction::CallFunction {
            address: template_address.into_inner(),
            function: function_name.try_into()?,
            args: args.into_iter().map(|s| s.into_arg()).collect(),
        },
        CliInstruction::CallMethod {
            component_address,
            method_name,
            args,
        } => Instruction::CallMethod {
            call: component_address
                .as_component_address()
                .ok_or_else(|| anyhow!("Invalid component address: {}", component_address))?
                .into(),
            method: method_name.try_into()?,
            args: args.into_iter().map(|s| s.into_arg()).collect(),
        },
    };

    let fee_account;
    if let Some(fee_account_name) = common.fee_account.clone() {
        fee_account = client.accounts_get(fee_account_name).await?.account;
    } else {
        fee_account = client.accounts_get_default().await?.account;
    }

    let owner_key_id = fee_account
        .owner_key_id
        .ok_or_else(|| anyhow!("Fee account does not have an owner key ID"))?;

    let SettingsGetResponse {
        network,
        current_epoch,
        default_transaction_validity_epochs,
        ..
    } = client.get_settings().await?;
    let max_epoch = common.resolve_max_epoch(current_epoch, default_transaction_validity_epochs)?;

    let mut builder = Transaction::builder(network.byte, max_epoch)
        .call_method(fee_account.component_address, "withdraw", args![common.max_fee,])
        .put_last_instruction_output_on_workspace("fee_bucket")
        .pay_fee_from_bucket("fee_bucket")
        .add_instruction(instruction)
        .with_inputs(common.inputs)
        .with_min_epoch(common.min_epoch.map(Epoch));

    if let Some(dump_account) = common.dump_outputs_into {
        let AccountGetResponse { account, .. } = client.accounts_get(dump_account).await?;

        builder = builder.put_last_instruction_output_on_workspace("bucket").call_method(
            account.component_address,
            "deposit",
            args![Workspace("bucket")],
        );
    }

    let transaction = builder.build_unsigned();
    summarize_transaction(&transaction);

    if common.dry_run {
        println!("NOTE: Dry run is enabled. This transaction will not be processed by the network.");
        println!();
        let resp = client
            .submit_transaction_dry_run(TransactionSubmitDryRunRequest {
                transaction,
                seal_signer: owner_key_id,
                other_signers: vec![],
                signatures: vec![],
                detect_inputs: common.detect_inputs.unwrap_or(true),
                detect_inputs_use_unversioned: true,
                lock_ids: vec![],
            })
            .await?;
        wait_transaction_result(resp.transaction_id, client).await?;
    } else {
        let request = TransactionSubmitRequest {
            transaction,
            seal_signer: owner_key_id,
            other_signers: vec![],
            signatures: vec![],
            detect_inputs: common.detect_inputs.unwrap_or(true),
            detect_inputs_use_unversioned: true,
            lock_ids: vec![],
        };
        let resp = client.submit_transaction(&request).await?;
        wait_transaction_result(resp.transaction_id, client).await?;
    }
    Ok(())
}

async fn handle_submit_manifest(
    args: SubmitManifestArgs,
    client: &mut WalletDaemonClient,
) -> Result<(), anyhow::Error> {
    let timer = Instant::now();
    let contents = fs::read_to_string(&args.manifest).map_err(|e| anyhow!("Failed to read manifest: {}", e))?;
    let instructions = parse_manifest(
        &contents,
        parse_globals(args.input_variables)?,
        Default::default(),
        Default::default(),
    )?;
    let common = args.common;

    let fee_account;
    if let Some(fee_account_name) = common.fee_account.clone() {
        fee_account = client.accounts_get(fee_account_name).await?.account;
    } else {
        fee_account = client.accounts_get_default().await?.account;
    }

    let owner_key_id = fee_account
        .owner_key_id
        .ok_or_else(|| anyhow!("Fee account does not have an owner key ID"))?;

    let SettingsGetResponse {
        network,
        current_epoch,
        default_transaction_validity_epochs,
        ..
    } = client.get_settings().await?;
    let max_epoch = common.resolve_max_epoch(current_epoch, default_transaction_validity_epochs)?;

    let builder = Transaction::builder(network.byte, max_epoch)
        .with_fee_instructions_builder(|builder| {
            builder.with_instructions(instructions.fee_instructions).call_method(
                fee_account.component_address,
                "pay_fee",
                args![common.max_fee],
            )
        })
        .with_instructions(instructions.instructions)
        .with_inputs(common.inputs)
        .with_min_epoch(common.min_epoch.map(Epoch));

    let transaction = builder.build_unsigned();
    summarize_transaction(&transaction);

    if common.dry_run {
        println!("NOTE: Dry run is enabled. This transaction will not be processed by the network.");
        println!();

        let resp = client
            .submit_transaction_dry_run(TransactionSubmitDryRunRequest {
                transaction,
                seal_signer: owner_key_id,
                other_signers: vec![],
                signatures: vec![],
                detect_inputs: common.detect_inputs.unwrap_or(true),
                detect_inputs_use_unversioned: true,
                lock_ids: vec![],
            })
            .await?;
        summarize(&resp.result.finalize, timer.elapsed());
    } else {
        let request = TransactionSubmitRequest {
            transaction,
            seal_signer: owner_key_id,
            other_signers: vec![],
            signatures: vec![],
            detect_inputs: common.detect_inputs.unwrap_or(true),
            detect_inputs_use_unversioned: true,
            lock_ids: vec![],
        };

        let resp = client.submit_transaction(&request).await?;
        wait_transaction_result(resp.transaction_id, client).await?;
    }

    Ok(())
}

pub async fn handle_send(args: SendArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let SendArgs {
        source_account_name,
        amount,
        resource_address,
        destination_public_key,
        common,
    } = args;

    let destination_public_key =
        RistrettoPublicKeyBytes::from_bytes(&destination_public_key.into_inner()).map_err(anyhow::Error::msg)?;

    let fee = common.max_fee;
    let resp = client
        .accounts_transfer(AccountsTransferRequest {
            account: source_account_name,
            amount: amount.into(),
            resource_address,
            destination_public_key,
            max_fee: fee,
            proof_from_badge_resource: None,
            dry_run: false,
        })
        .await?;
    let resp = client
        .wait_transaction_result(TransactionWaitResultRequest {
            transaction_id: resp.transaction_id,
            timeout_secs: common.wait_for_result_timeout_secs,
        })
        .await?;
    if resp.timed_out {
        println!("❌ Transaction {} result timed out.", resp.transaction_id);
        println!();
        return Ok(());
    }

    println!("Transaction: {}", resp.transaction_id);
    println!(
        "Fee: {} ({} refunded)",
        resp.final_fee,
        resp.result.as_ref().unwrap().fee_receipt.total_refunded()
    );
    println!();
    summarize_finalize_result(resp.result.as_ref().unwrap());

    Ok(())
}

pub async fn handle_confidential_transfer(
    args: ConfidentialTransferArgs,
    client: &mut WalletDaemonClient,
) -> Result<(), anyhow::Error> {
    let ConfidentialTransferArgs {
        source_account,
        resource_address,
        amount,
        destination_address,
        common,
        memo_message,
    } = args;

    // let AccountByNameResponse { account, .. } = client.accounts_get_by_name(&source_account_name).await?;
    let resp = client
        .accounts_confidential_transfer(ConfidentialTransferRequest {
            account: source_account,
            input_selection: UtxoInputSelection::PreferConfidential,
            amount: amount.into(),
            resource_address: resource_address.unwrap_or(STEALTH_TARI_RESOURCE_ADDRESS),
            destination_address,
            max_fee: common.max_fee,
            output_to_revealed: false,
            proof_from_badge_resource: None,
            output_memo: memo_message
                .map(|s| Memo::new_message(s).ok_or_else(|| anyhow!("Invalid memo length")))
                .transpose()?,
            dry_run: false,
        })
        .await?;
    let resp = client
        .wait_transaction_result(TransactionWaitResultRequest {
            transaction_id: resp.transaction_id,
            timeout_secs: common.wait_for_result_timeout_secs,
        })
        .await?;
    if resp.timed_out {
        println!("❌ Transaction result timed out.",);
        println!();
        return Ok(());
    }

    println!("Transaction: {}", resp.transaction_id);
    println!("Fee: {}", resp.final_fee);
    println!();
    summarize_finalize_result(&resp.result.unwrap());

    Ok(())
}

pub async fn wait_transaction_result(
    transaction_id: TransactionId,
    client: &mut WalletDaemonClient,
) -> Result<TransactionWaitResultResponse, anyhow::Error> {
    let timer = Instant::now();

    println!();
    println!("✅ Transaction {} submitted.", transaction_id);
    println!();

    println!();
    println!("⏳️ Waiting for transaction result...");
    println!();
    let wait_resp = client
        .wait_transaction_result(TransactionWaitResultRequest {
            transaction_id,
            // Never timeout, you can ctrl+c to exit
            timeout_secs: None,
        })
        .await?;
    if wait_resp.timed_out {
        println!("⏳️ Transaction result timed out.",);
        println!();
    } else if let Some(ref result) = wait_resp.result {
        summarize(result, timer.elapsed());
    } else {
        println!("⚠️ Transaction not finalized");
    }

    Ok(wait_resp)
}

fn summarize_transaction(transaction: &UnsignedTransaction) {
    println!("Inputs:");
    if transaction.inputs().is_empty() {
        println!("  None");
    } else {
        for req in transaction.inputs() {
            println!("- {}", req);
        }
    }
    println!();
    println!("🌟 Submitting fee instructions:");
    for instruction in transaction.fee_instructions() {
        println!("- {}", instruction);
    }
    println!();
    println!("🌟 Submitting instructions:");
    for instruction in transaction.instructions() {
        println!("- {}", instruction);
    }
    println!();
}

fn summarize(result: &FinalizeResult, time_taken: Duration) {
    println!("✅️ Transaction complete");
    println!();
    // if let Some(qc) = resp.qcs.first() {
    //     println!("Epoch: {}", qc.epoch());
    //     println!("Payload height: {}", qc.payload_height());
    //     println!("Signed by: {} validator nodes", qc.validators_metadata().len());
    // } else {
    //     println!("No QC");
    // }
    // println!();

    summarize_finalize_result(result);

    println!();
    println!("Fee: {}", result.fee_receipt.total_fees_charged());
    println!("Time taken: {:?}", time_taken);
    println!();
    println!("OVERALL DECISION: {}", result.result);
}

pub fn print_substate_diff(diff: &SubstateDiff) {
    for (id, substate) in diff.up_iter() {
        println!("️🌲 UP substate {} (v{})", id, substate.version(),);
        println!(
            "      🧩 Substate address: {}",
            SubstateAddress::from_substate_id(id, substate.version())
        );
        match substate.substate_value() {
            SubstateValue::Component(_) => {
                println!("      ▶ component: {}", id);
            },
            SubstateValue::Resource(_) => {
                println!("      ▶ resource: {}", id);
            },
            SubstateValue::TransactionReceipt(_) => {
                println!("      ▶ transaction_receipt: {}", id);
            },
            SubstateValue::Vault(vault) => {
                println!("      ▶ vault: {} {}", id, vault.resource_address());
            },
            SubstateValue::NonFungible(_) => {
                println!("      ▶ NFT: {}", id);
            },
            SubstateValue::ClaimedOutputTombstone(_) => {
                println!("      ▶ Layer 1 commitment: {}", id);
            },
            SubstateValue::Template(_) => {
                println!("      ▶ Template: {}", id);
            },
            SubstateValue::ValidatorFeePool(pool) => {
                println!("      ▶ Validator Fee Pool: {}", id);
                println!("        ▶ Total fees: {}", pool.amount());
            },
            SubstateValue::Utxo(_) => {
                println!("      ▶ UTXO:");
                let utxo_address = id.as_utxo_address().unwrap();
                println!("        ▶ Resource: {}", utxo_address.resource_address());
                println!("        ▶ id: {}", utxo_address.id());
            },
            SubstateValue::ConfidentialOutput(_) => {
                println!("      ▶ Confidential output:");
                let output_address = id.as_confidential_output_address().unwrap();
                println!("        ▶ Resource: {}", output_address.resource_address());
                println!("        ▶ commitment: {}", output_address.commitment());
            },
        }
        println!();
    }
    for (address, version) in diff.down_iter() {
        println!("🗑️ DOWN substate {} v{}", address, version,);
        println!(
            "      🧩 Substate address: {}",
            SubstateAddress::from_substate_id(address, *version)
        );
        println!();
    }
}

fn print_reject_reason(reason: &RejectReason) {
    println!("❌️ Transaction rejected: {}", reason);
}

pub fn summarize_finalize_result(finalize: &FinalizeResult) {
    println!("========= Substates =========");
    match finalize.result {
        TransactionResult::Accept(ref diff) => print_substate_diff(diff),
        TransactionResult::AcceptFeeRejectRest(ref diff, ref reason) => {
            print_substate_diff(diff);
            print_reject_reason(reason);
        },
        TransactionResult::Reject(ref reason) => print_reject_reason(reason),
    }

    println!();
    println!("========= LOGS =========");
    for log in &finalize.logs {
        println!("{}", log);
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
    Blob(tari_bor::Value),
    NonFungibleId(NonFungibleId),
    SubstateId(SubstateId),
    TemplateAddress(TemplateAddress),
}

impl FromStr for CliArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(file) = s.strip_prefix('@') {
            let base64_data = fs::read_to_string(file).map_err(|e| anyhow!("Failed to read file {}: {}", file, e))?;
            return Ok(CliArg::Blob(decode_exact(&base64_decode(base64_data)?)?));
        }

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

        if let Ok(v) = s.parse::<SubstateId>() {
            return Ok(CliArg::SubstateId(v));
        }

        if let Some(v) = parse_template_address(s) {
            return Ok(CliArg::TemplateAddress(v));
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
    pub fn into_arg(self) -> InstructionArg {
        match self {
            CliArg::String(s) => call_arg!(s),
            CliArg::U64(v) => call_arg!(v),
            CliArg::U32(v) => call_arg!(v),
            CliArg::U16(v) => call_arg!(v),
            CliArg::U8(v) => call_arg!(v),
            CliArg::I64(v) => call_arg!(v),
            CliArg::I32(v) => call_arg!(v),
            CliArg::I16(v) => call_arg!(v),
            CliArg::I8(v) => call_arg!(v),
            CliArg::Bool(v) => call_arg!(v),
            CliArg::Blob(v) => InstructionArg::literal(v).unwrap(),
            CliArg::SubstateId(v) => match v {
                SubstateId::Component(v) => call_arg!(v),
                SubstateId::Resource(v) => call_arg!(v),
                SubstateId::Vault(v) => call_arg!(v),
                SubstateId::ClaimedOutputTombstone(v) => call_arg!(v),
                SubstateId::NonFungible(v) => call_arg!(v),
                SubstateId::TransactionReceipt(v) => call_arg!(v),
                SubstateId::Template(v) => call_arg!(v.as_template_address()),
                SubstateId::ValidatorFeePool(v) => call_arg!(v),
                SubstateId::Utxo(v) => call_arg!(v),
                SubstateId::ConfidentialOutput(v) => call_arg!(v),
            },
            CliArg::TemplateAddress(v) => call_arg!(v),
            CliArg::NonFungibleId(v) => call_arg!(v),
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
    pub fn to_substate_address(&self) -> SubstateId {
        SubstateId::NonFungible(NonFungibleAddress::new(
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
        let resource_address = SubstateId::from_str(resource_address)?;
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
        let resource_address = SubstateId::from_str(resource_address)?;
        let resource_address = resource_address
            .as_resource_address()
            .ok_or_else(|| anyhow!("Expected resource address but got {}", resource_address))?;
        Ok(NewNonFungibleMintOutput {
            resource_address,
            count: count_str.parse()?,
        })
    }
}

fn parse_globals(globals: Vec<String>) -> Result<HashMap<String, ManifestValue>, anyhow::Error> {
    let mut result = HashMap::with_capacity(globals.len());
    for global in globals {
        let (name, value) = global
            .split_once('=')
            .ok_or_else(|| anyhow!("Invalid global: {}", global))?;
        if let Ok(url) = url::Url::parse(value) {
            let blob = match url.scheme() {
                "file" => {
                    let contents = fs::read_to_string(url.path())
                        .map_err(|err| anyhow!("Failed to read file '{}': {}", url, err))?;

                    base64_decode(contents.trim())
                        .map_err(|err| anyhow!("Failed to decode base64 file '{}': {}", url, err))?
                },
                "data" => {
                    base64_decode(url.path()).map_err(|err| anyhow!("Failed to decode base64 '{}': {}", url, err))?
                },
                scheme => anyhow::bail!("Unsupported scheme '{}'", scheme),
            };
            result.insert(name.to_string(), ManifestValue::Value(decode_exact(&blob)?));
        } else {
            let value = value
                .parse()
                .map_err(|err| anyhow!("Failed to parse global '{}': {}", name, err))?;
            result.insert(name.to_string(), value);
        }
    }
    Ok(result)
}

fn base64_decode<T: AsRef<[u8]>>(s: T) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::prelude::BASE64_STANDARD.decode(s)
}
