//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use clap::{Args, Subcommand};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::Epoch;
use tari_template_lib::{crypto::RistrettoPublicKeyBytes, models::Amount};
use tari_utilities::ByteArray;
use tari_wallet_daemon_client::{
    types::{ClaimValidatorFeesRequest, GetValidatorFeesRequest},
    ComponentAddressOrName,
    WalletDaemonClient,
};

use crate::{command::transaction::summarize_finalize_result, from_hex::FromHex};

#[derive(Debug, Subcommand, Clone)]
pub enum ValidatorSubcommand {
    ClaimFees(ClaimFeesArgs),
    GetFees(GetFeesArgs),
}

#[derive(Debug, Args, Clone)]
pub struct ClaimFeesArgs {
    #[clap(long, short = 'a', alias = "account")]
    pub dest_account_name: Option<String>,
    #[clap(long, short = 'v')]
    pub validator_public_key: FromHex<RistrettoPublicKeyBytes>,
    pub claim_fees_public_key: FromHex<RistrettoPublicKeyBytes>,
    #[clap(long, short = 'e')]
    pub epoch: u64,
    #[clap(long)]
    pub max_fee: Option<u32>,
    #[clap(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args, Clone)]
pub struct GetFeesArgs {
    #[clap(long, short = 'v')]
    pub validator_public_key: FromHex<RistrettoPublicKeyBytes>,
}

impl ValidatorSubcommand {
    pub async fn handle(self, mut client: WalletDaemonClient) -> Result<(), anyhow::Error> {
        match self {
            ValidatorSubcommand::ClaimFees(args) => {
                handle_claim_validator_fees(args, &mut client).await?;
            },
            ValidatorSubcommand::GetFees(args) => {
                handle_get_fees(args, &mut client).await?;
            },
        }
        Ok(())
    }
}

pub async fn handle_get_fees(args: GetFeesArgs, client: &mut WalletDaemonClient) -> Result<(), anyhow::Error> {
    // TODO: complete this handler once this request is implemented
    let resp = client
        .get_validator_fee_summary(GetValidatorFeesRequest {
            validator_public_key: PublicKey::from_canonical_bytes(args.validator_public_key.into_inner().as_bytes())
                .map_err(anyhow::Error::msg)?,
            epoch: Epoch(0),
        })
        .await?;

    println!("{:?}", resp);
    Ok(())
}

pub async fn handle_claim_validator_fees(
    args: ClaimFeesArgs,
    client: &mut WalletDaemonClient,
) -> Result<(), anyhow::Error> {
    let ClaimFeesArgs {
        dest_account_name,
        validator_public_key,
        claim_fees_public_key,
        epoch,
        max_fee,
        dry_run,
    } = args;

    println!("Submitting claim validator fees transaction...");

    let resp = client
        .claim_validator_fees(ClaimValidatorFeesRequest {
            account: dest_account_name
                .map(|name| ComponentAddressOrName::from_str(&name))
                .transpose()?,
            max_fee: max_fee.map(Amount::from),
            validator_public_key: PublicKey::from_canonical_bytes(validator_public_key.into_inner().as_bytes())
                .map_err(anyhow::Error::msg)?,
            claim_fees_public_key: PublicKey::from_canonical_bytes(claim_fees_public_key.into_inner().as_bytes())
                .map_err(anyhow::Error::msg)?,
            epoch: Epoch(epoch),
            dry_run,
        })
        .await?;

    println!("Transaction: {}", resp.transaction_id);
    println!("Fee: {}", resp.fee);
    println!();
    summarize_finalize_result(&resp.result);

    Ok(())
}
