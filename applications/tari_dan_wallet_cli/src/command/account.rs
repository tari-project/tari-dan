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

use clap::{Args, Subcommand};
use tari_wallet_daemon_client::{
    types::{AccountsCreateRequest, AccountsGetBalancesRequest},
    WalletDaemonClient,
};

use crate::{table::Table, table_row};

#[derive(Debug, Subcommand, Clone)]
pub enum AccountsSubcommand {
    #[clap(alias = "new")]
    Create(CreateArgs),
    #[clap(alias = "get-balance")]
    GetBalances(GetBalancesArgs),
    List,
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
    for (resx, amt) in resp.balances {
        println!("- Resource: {} Balance: {}", resx, amt);
    }
    Ok(())
}

async fn handle_list(client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    println!("Submitted account list transaction...");
    let resp = client.list_accounts(100).await?;

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
