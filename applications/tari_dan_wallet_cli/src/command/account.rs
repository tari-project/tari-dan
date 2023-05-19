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
    convert::{TryFrom, TryInto},
    fs,
    io,
    io::Read,
    path::PathBuf,
};

use anyhow::anyhow;
use clap::{Args, Subcommand};
use serde_json as json;
use tari_template_lib::models::Amount;
use tari_utilities::ByteArray;
use tari_wallet_daemon_client::{
    types::{
        AccountInfo,
        AccountsCreateFreeTestCoinsRequest,
        AccountsCreateRequest,
        AccountsGetBalancesRequest,
        AccountsInvokeRequest,
        ClaimBurnRequest,
        RevealFundsRequest,
    },
    ComponentAddressOrName,
    WalletDaemonClient,
};

use crate::{
    command::transaction::{print_execution_results, summarize_finalize_result, CliArg},
    table::Table,
    table_row,
};

#[derive(Debug, Subcommand, Clone)]
pub enum AccountsSubcommand {
    #[clap(alias = "new")]
    Create(CreateArgs),
    #[clap(alias = "get-balance", alias = "balance")]
    GetBalances(GetBalancesArgs),
    List,
    Invoke {
        #[clap(long, alias = "name", short = 'n')]
        account: Option<ComponentAddressOrName>,
        method: String,
        #[clap(long, short = 'a')]
        args: Vec<CliArg>,
        fee: Option<u32>,
    },
    Get(GetArgs),
    ClaimBurn(ClaimBurnArgs),
    #[clap(alias = "reveal")]
    RevealFunds(RevealFundsArgs),
    #[clap(alias = "faucet")]
    CreateFreeTestCoins(CreateFreeTestCoinsArgs),
    #[clap(alias = "default")]
    SetDefault(SetDefaultArgs),
}

#[derive(Debug, Args, Clone)]
pub struct CreateArgs {
    #[clap(long, alias = "name")]
    pub account_name: Option<String>,
    #[clap(long, alias = "dry-run")]
    pub is_dry_run: bool,
    pub is_default: bool,
    pub fee: Option<u32>,
}

#[derive(Debug, Args, Clone)]
pub struct SetDefaultArgs {
    pub account_name: ComponentAddressOrName,
}

#[derive(Debug, Args, Clone)]
pub struct GetBalancesArgs {
    pub account_name: Option<ComponentAddressOrName>,
}

#[derive(Debug, Args, Clone)]
pub struct GetArgs {
    pub name: ComponentAddressOrName,
}

#[derive(Debug, Args, Clone)]
pub struct ClaimBurnArgs {
    #[clap(long, short = 'a', alias = "account")]
    account: Option<ComponentAddressOrName>,
    #[clap(long, short = 'i', alias = "input")]
    proof_file: Option<PathBuf>,
    /// Optional proof JSON from the L1 console wallet. If not provided, you will be prompted to enter it.
    #[clap(long, short = 'j', alias = "json")]
    proof_json: Option<serde_json::Value>,
    #[clap(long, short = 'f')]
    fee: Option<u32>,
}

#[derive(Debug, Args, Clone)]
pub struct RevealFundsArgs {
    /// Amount of funds to reveal
    reveal_amount: u64,
    /// The account name where the funds will be revealed
    account: Option<ComponentAddressOrName>,
    /// The fee to pay for the reveal transaction
    #[clap(long, short = 'f')]
    fee: Option<u32>,
    /// If set, the fee will be paid from the revealed funds instead of from the account resulting in less revealed
    /// funds than requested.
    #[clap(long, default_value_t = true)]
    pay_from_reveal: bool,
}

#[derive(Debug, Args, Clone)]
pub struct CreateFreeTestCoinsArgs {
    pub account: Option<ComponentAddressOrName>,
    #[clap(long, short, alias = "amount")]
    pub amount: Option<u64>,
    #[clap(long, short, alias = "fee")]
    pub fee: Option<u64>,
}

impl AccountsSubcommand {
    pub async fn handle(self, mut client: WalletDaemonClient) -> Result<(), anyhow::Error> {
        match self {
            AccountsSubcommand::Create(args) => {
                handle_create(args, &mut client).await?;
            },
            AccountsSubcommand::GetBalances(args) => {
                handle_get_balances(args, &mut client).await?;
            },
            AccountsSubcommand::List => {
                handle_list(&mut client).await?;
            },
            AccountsSubcommand::Invoke {
                account,
                method,
                args,
                fee,
            } => hande_invoke(account, method, args, fee, &mut client).await?,
            AccountsSubcommand::Get(args) => handle_get(args, &mut client).await?,
            AccountsSubcommand::ClaimBurn(args) => handle_claim_burn(args, &mut client).await?,
            AccountsSubcommand::RevealFunds(args) => handle_reveal_funds(args, &mut client).await?,
            AccountsSubcommand::CreateFreeTestCoins(args) => handle_create_free_test_coins(args, &mut client).await?,
            AccountsSubcommand::SetDefault(args) => handle_set_default(args, &mut client).await?,
        }
        Ok(())
    }
}

async fn handle_create(args: CreateArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    println!("Submitted new account creation transaction...");
    let resp = client
        .create_account(AccountsCreateRequest {
            account_name: args.account_name,
            custom_access_rules: None,
            is_default: args.is_default,
            fee: args.fee.map(|u| Amount::new(u.into())),
        })
        .await?;

    println!();
    println!("✅ Account created");
    println!("   address: {}", resp.address);
    println!("   public key (hex): {}", resp.public_key);
    println!("   public key (base64): {}", base64::encode(resp.public_key.as_bytes()));
    Ok(())
}

async fn handle_set_default(args: SetDefaultArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let _resp = client.accounts_set_default(args.account_name).await?;
    println!("✅ Default account set");
    Ok(())
}

async fn hande_invoke(
    account: Option<ComponentAddressOrName>,
    method: String,
    args: Vec<CliArg>,
    fee: Option<u32>,
    client: &mut WalletDaemonClient,
) -> Result<(), anyhow::Error> {
    println!("Submitted invoke transaction for account...",);
    let resp = client
        .invoke_account_method(AccountsInvokeRequest {
            account,
            method,
            args: args.into_iter().map(|a| a.into_arg()).collect(),
            fee: fee.map(|u| Amount::new(u.into())),
        })
        .await?;

    println!();
    println!("✅ Account invoked succeeded");
    println!();
    match resp.result {
        Some(result) => print_execution_results(&[result]),
        None => {
            println!("No result returned");
        },
    }
    Ok(())
}

async fn handle_get_balances(args: GetBalancesArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let resp = client
        .get_account_balances(AccountsGetBalancesRequest {
            account: args.account_name,
            refresh: true,
        })
        .await?;

    if resp.balances.is_empty() {
        println!("Account {} has no vaults", resp.address);
        return Ok(());
    }

    println!("Account {} balances:", resp.address);
    println!();
    let mut table = Table::new();
    table.enable_row_count();
    table.set_titles(vec!["VaultId", "Resource", "Balance"]);
    for balance in resp.balances {
        table.add_row(table_row!(
            balance.vault_address,
            format!("{} {:?}", balance.resource_address, balance.resource_type),
            balance.to_balance_string()
        ));
    }
    table.print_stdout();
    Ok(())
}

pub async fn handle_claim_burn(args: ClaimBurnArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let ClaimBurnArgs {
        account,
        proof_json,
        fee,
        proof_file,
    } = args;

    let claim_proof = if let Some(proof_json) = proof_json {
        proof_json
    } else if let Some(proof_file) = proof_file {
        let proof_json = fs::read_to_string(proof_file).map_err(|e| anyhow!("Failed to read proof file: {}", e))?;
        json::from_str::<json::Value>(proof_json.trim()).map_err(|e| anyhow!("Failed to parse proof JSON: {}", e))?
    } else {
        println!(
            "Please paste console wallet JSON output from claim_burn call in the terminal: Press <Ctrl/Cmd + d> once \
             done"
        );

        let mut proof_json = String::new();
        io::stdin().read_to_string(&mut proof_json)?;
        json::from_str::<json::Value>(proof_json.trim()).map_err(|e| anyhow!("Failed to parse proof JSON: {}", e))?
    };

    println!("✅ Claim burn submitted");

    let req = ClaimBurnRequest {
        account,
        claim_proof,
        fee: fee.map(|f| f.try_into()).transpose()?,
    };

    let resp = client
        .claim_burn(req)
        .await
        .map_err(|e| anyhow!("Failed to claim burn with error = {}", e.to_string()))?;

    println!("Total transaction fee: {}", resp.fee);
    println!();

    summarize_finalize_result(&resp.result);
    Ok(())
}

async fn handle_create_free_test_coins(
    args: CreateFreeTestCoinsArgs,
    client: &mut WalletDaemonClient,
) -> Result<(), anyhow::Error> {
    println!("Creating free test coins...");
    let resp = client
        .create_free_test_coins(AccountsCreateFreeTestCoinsRequest {
            account: args.account,
            amount: Amount::new(args.amount.unwrap_or(100000) as i64),
            fee: args.fee.map(|u| u.try_into()).transpose()?,
        })
        .await?;

    println!("✅ Free test coins created");
    println!("   amount: {}", resp.amount);
    println!("   transaction fee: {}", resp.fee);
    Ok(())
}

async fn handle_list(client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let resp = client.list_accounts(0, 100).await?;

    if resp.accounts.is_empty() {
        println!("No accounts found");
        return Ok(());
    }

    let mut table = Table::new();
    table.enable_row_count();
    table.set_titles(vec!["Name", "Address", "Public Key", "Default"]);
    println!("Accounts:");
    for AccountInfo { account, public_key } in resp.accounts {
        table.add_row(table_row!(
            account.name,
            account.address,
            public_key,
            if account.is_default { "✅" } else { "" }
        ));
    }
    table.print_stdout();
    Ok(())
}

async fn handle_get(args: GetArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    println!("Get account component address by its name...");
    let resp = client.accounts_get(args.name.clone()).await?;

    println!(
        "Account {} substate_address: {}",
        resp.account.name, resp.account.address
    );
    println!();

    Ok(())
}

pub async fn handle_reveal_funds(args: RevealFundsArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let RevealFundsArgs {
        account,
        reveal_amount,
        fee,
        pay_from_reveal,
    } = args;

    println!("Submitting reveal transaction...");
    let resp = client
        .accounts_reveal_funds(RevealFundsRequest {
            account,
            amount_to_reveal: Amount::try_from(reveal_amount).expect("Reveal amount too large"),
            fee: fee.map(|f| f.try_into()).transpose()?,
            pay_fee_from_reveal: pay_from_reveal,
        })
        .await?;

    println!("Transaction: {}", resp.hash);
    println!("Fee: {}", resp.fee);
    println!();
    summarize_finalize_result(&resp.result);

    Ok(())
}
