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
use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::Epoch;
use tari_template_lib::crypto::RistrettoPublicKeyBytes;
use tari_validator_node_client::{types::GetValidatorFeesRequest, ValidatorNodeClient};

use crate::{cli_range::CliRange, from_hex::FromHex, table::Table, table_row};

#[derive(Debug, Subcommand, Clone)]
pub enum VnSubcommand {
    #[clap(alias = "get-fees")]
    GetFeeInfo(GetFeesArgs),
}

impl VnSubcommand {
    pub async fn handle(self, mut client: ValidatorNodeClient) -> Result<(), anyhow::Error> {
        match self {
            VnSubcommand::GetFeeInfo(args) => {
                handle_get_fee_info(args, &mut client).await?;
            },
        }
        Ok(())
    }
}

#[derive(Debug, Args, Clone)]
pub struct GetFeesArgs {
    #[clap(long, alias = "vn")]
    validator_public_key: Option<FromHex<RistrettoPublicKeyBytes>>,
    #[clap(long, short = 'e')]
    epoch_range: Option<CliRange<Epoch>>,
}

async fn handle_get_fee_info(args: GetFeesArgs, client: &mut ValidatorNodeClient) -> anyhow::Result<()> {
    let stats = client.get_epoch_manager_stats().await?;
    let epoch_range = args
        .epoch_range
        .map(|r| r.into_inner())
        .unwrap_or(Epoch(0)..=stats.current_epoch);

    println!(
        "Fetching fee data from epochs {} - {}",
        epoch_range.start().as_u64(),
        epoch_range.end().as_u64()
    );
    println!();

    let resp = client
        .get_fees(GetValidatorFeesRequest {
            epoch_range,
            validator_public_key: args
                .validator_public_key
                .map(|pk| PublicKey::from_canonical_bytes(pk.into_inner().as_bytes()))
                .transpose()
                .map_err(anyhow::Error::msg)?,
        })
        .await?;

    let mut table = Table::new();
    table
        .enable_row_count()
        .set_titles(vec!["Validator", "Epoch", "Block", "Total Due", "Total Tx fees"]);

    for fee in resp.fees {
        table.add_row(table_row!(
            fee.validator_public_key,
            fee.epoch,
            fee.block_id,
            fee.total_fee_due,
            fee.total_transaction_fee
        ));
    }

    table.print_stdout();
    Ok(())
}
