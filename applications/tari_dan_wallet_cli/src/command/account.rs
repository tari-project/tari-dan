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

use std::{io, io::Read};

use anyhow::anyhow;
use clap::{Args, Subcommand};
use serde_json as json;
use tari_wallet_daemon_client::{
    types::{
        AccountByNameResponse,
        AccountsCreateRequest,
        AccountsGetBalancesRequest,
        AccountsInvokeRequest,
        ClaimBurnRequest,
    },
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
    #[clap(alias = "get-balance")]
    GetBalances(GetBalancesArgs),
    List,
    Invoke {
        #[clap(long, alias = "name", short = 'n')]
        account: String,
        method: String,
        #[clap(long, short = 'a')]
        args: Vec<CliArg>,
    },
    #[clap(alias = "get")]
    GetByName(GetByNameArgs),
    #[clap(alias = "claim-burn")]
    ClaimBurn(ClaimBurnArgs),
}

#[derive(Debug, Args, Clone)]
pub struct CreateArgs {
    #[clap(long, alias = "name")]
    pub account_name: Option<String>,
    #[clap(long, alias = "dry-run")]
    pub is_dry_run: bool,
}

#[derive(Debug, Args, Clone)]
pub struct GetBalancesArgs {
    #[clap(long, alias = "name")]
    pub account_name: String,
}

#[derive(Debug, Args, Clone)]
pub struct GetByNameArgs {
    #[clap(long, alias = "name")]
    pub name: String,
}

#[derive(Debug, Args, Clone)]
pub struct ClaimBurnArgs {
    #[clap(long, short = 'n', alias = "name")]
    account_name: String,
    /// Optional proof JSON from the L1 console wallet. If not provided, you will be prompted to enter it.
    #[clap(long, short = 'j', alias = "json")]
    proof_json: Option<serde_json::Value>,
    #[clap(long, short = 'f')]
    fee: Option<u64>,
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
            AccountsSubcommand::Invoke { account, method, args } => {
                hande_invoke(account, method, args, &mut client).await?
            },
            AccountsSubcommand::GetByName(args) => handle_get_by_name(args, &mut client).await?,
            AccountsSubcommand::ClaimBurn(args) => handle_claim_burn(args, &mut client).await?,
        }
        Ok(())
    }
}

async fn handle_create(args: CreateArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    println!("Submitted new account creation transaction...");
    let resp = client
        .create_account(AccountsCreateRequest {
            account_name: args.account_name,
            signing_key_index: None,
            custom_access_rules: None,
            fee: Some(2),
        })
        .await?;

    println!();
    println!("✅ Account created");
    println!("   address: {}", resp.address);
    Ok(())
}

async fn hande_invoke(
    account: String,
    method: String,
    args: Vec<CliArg>,
    client: &mut WalletDaemonClient,
) -> Result<(), anyhow::Error> {
    println!("Submitted invoke transaction for account {}...", account);
    let resp = client
        .invoke_account_method(AccountsInvokeRequest {
            account_name: account,
            method,
            args: args.into_iter().map(|a| a.into_arg()).collect(),
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
    println!("Checking balances for account '{}'...", args.account_name);
    let resp = client
        .get_account_balances(AccountsGetBalancesRequest {
            account_name: args.account_name,
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
    table.set_titles(vec!["Resource", "Balance"]);
    for (resx, amt) in resp.balances {
        table.add_row(table_row!(resx, amt));
    }
    table.print_stdout();
    Ok(())
}

pub async fn handle_claim_burn(args: ClaimBurnArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    let ClaimBurnArgs {
        account_name,
        proof_json,
        fee,
    } = args;

    let AccountByNameResponse { account_address } = client.get_by_name(account_name).await?;

    let claim_proof = if let Some(proof_json) = proof_json {
        proof_json
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
        account: account_address.as_component_address().unwrap(),
        claim_proof,
        fee: fee.unwrap_or(1),
    };

    let resp = client
        .claim_burn(req)
        .await
        .map_err(|e| anyhow!("Failed to claim burn with error = {}", e.to_string()))?;

    summarize_finalize_result(&resp.result);
    Ok(())
}

async fn handle_list(client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    println!("Submitted account list transaction...");
    let resp = client.list_accounts(0, 100).await?;

    if resp.accounts.is_empty() {
        println!("No accounts found");
        return Ok(());
    }

    let mut table = Table::new();
    table.enable_row_count();
    table.set_titles(vec!["Name", "Address", "Key Index"]);
    println!("Accounts:");
    for account in resp.accounts {
        table.add_row(table_row!(account.name, account.address, account.key_index));
    }
    table.print_stdout();
    Ok(())
}

async fn handle_get_by_name(args: GetByNameArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    println!("Get account component address by its name...");
    let resp = client.get_by_name(args.name.clone()).await?;

    println!("Account {} substate_address: {}", args.name, resp.account_address);
    println!();

    Ok(())
}
