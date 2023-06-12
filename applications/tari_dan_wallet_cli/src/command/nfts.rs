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
    fs,
    io::{self, Read},
    path::PathBuf,
    str::FromStr,
};

use anyhow::anyhow;
use clap::{Args, Subcommand};
use tari_template_lib::{
    crypto::RistrettoPublicKeyBytes,
    prelude::{Amount, NonFungibleAddress},
};
use tari_utilities::hex::Hex;
use tari_wallet_daemon_client::{types::MintAccountNFTRequest, ComponentAddressOrName, WalletDaemonClient};

use crate::command::transaction::summarize_finalize_result;

#[derive(Debug, Subcommand, Clone)]
pub enum AccountNFTSubcommand {
    #[clap(alias = "mint")]
    Mint(MintAccountNFTArgs),
}

#[derive(Debug, Args, Clone)]
pub struct MintAccountNFTArgs {
    #[clap(long, short = 'a', alias = "account")]
    pub account: Option<ComponentAddressOrName>,
    #[clap(long, short = 'o', alias = "owner-token")]
    pub owner_token: String,
    #[clap(long, short = 't', alias = "token-symbol")]
    pub token_symbol: Option<String>,
    #[clap(long, short = 'i', alias = "metadata-file")]
    pub metadata_file: Option<PathBuf>,
    #[clap(long, short = 'm', alias = "metadata")]
    pub metadata: Option<serde_json::Value>,
    #[clap(long, short = 'f', alias = "mint-fee")]
    pub mint_fee: Option<u32>,
    #[clap(long, short = 'c', alias = "create-account-nft-fee")]
    pub create_account_nft_fee: Option<u32>,
}

impl AccountNFTSubcommand {
    pub async fn handle(self, mut client: WalletDaemonClient) -> Result<(), anyhow::Error> {
        match self {
            Self::Mint(args) => {
                handle_mint_account_nft(args, &mut client).await?;
            },
        }
        Ok(())
    }
}

pub async fn handle_mint_account_nft(
    args: MintAccountNFTArgs,
    client: &mut WalletDaemonClient,
) -> Result<(), anyhow::Error> {
    let MintAccountNFTArgs {
        account,
        owner_token,
        token_symbol,
        metadata_file,
        metadata,
        mint_fee,
        create_account_nft_fee,
    } = args;

    let account = if let Some(account) = account {
        account
    } else {
        println!(
            "Please paste console wallet account name or respective component address from mint_account_nft call in \
             the terminal: Press <Ctrl/Cmd + d> once done"
        );

        let mut account = String::new();
        io::stdin().read_to_string(&mut account)?;
        ComponentAddressOrName::from_str(&account)
            .map_err(|e| anyhow!("Failed to parse account name or component address, with error = {}", e))?
    };

    let token_symbol = if let Some(token_symbol) = token_symbol {
        token_symbol
    } else {
        println!(
            "Please paste console wallet token symbol from mint_account_nft call in the terminal: Press <Ctrl/Cmd + \
             d> once done"
        );

        let mut token_symbol = String::new();
        io::stdin().read_to_string(&mut token_symbol)?;
        token_symbol
    };

    let metadata = if let Some(metadata) = metadata {
        metadata
    } else if let Some(metadata_file) = metadata_file {
        let metadata = fs::read_to_string(metadata_file).map_err(|e| anyhow!("Failed to read metadata file: {}", e))?;
        serde_json::from_str::<serde_json::Value>(metadata.trim())
            .map_err(|e| anyhow!("Failed to parse metadata JSON: {}", e))?
    } else {
        println!(
            "Please paste console wallet JSON metadata from mint_account_nft call in the terminal: Press <Ctrl/Cmd + \
             d> once done"
        );

        let mut metadata = String::new();
        io::stdin().read_to_string(&mut metadata)?;
        serde_json::from_str::<serde_json::Value>(metadata.trim())
            .map_err(|e| anyhow!("Failed to parse metadata: {}", e))?
    };

    let owner_token = NonFungibleAddress::from_public_key(
        RistrettoPublicKeyBytes::from_bytes(
            &Vec::<u8>::from_hex(&owner_token)
                .map_err(|e| anyhow!("Failed to parse owner token, with error = {}", e))?,
        )
        .map_err(|e| anyhow!("Failed to parse owner token, with error = {}", e.to_error_string()))?,
    );

    println!("✅ Mint account NFT submitted");

    let req = MintAccountNFTRequest {
        account,
        owner_token,
        token_symbol,
        metadata,
        mint_fee: mint_fee.map(|f| Amount::new(f as i64)),
        create_account_nft_fee: create_account_nft_fee.map(|f| Amount::new(f as i64)),
    };

    let resp = client
        .mint_account_nft(req)
        .await
        .map_err(|e| anyhow!("Failed to mint account NFT with error = {}", e.to_string()))?;

    println!("Total transaction fee: {}", resp.fee);
    println!();

    summarize_finalize_result(&resp.result);
    Ok(())
}
