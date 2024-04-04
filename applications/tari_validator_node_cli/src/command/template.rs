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

use clap::{Subcommand};
use tari_engine_types::{TemplateAddress};
use tari_validator_node_client::{
    types::{GetTemplateRequest, GetTemplateResponse, GetTemplatesRequest},
    ValidatorNodeClient,
};

use crate::{from_hex::FromHex, table::Table, table_row};

#[derive(Debug, Subcommand, Clone)]
pub enum TemplateSubcommand {
    Get { template_address: FromHex<TemplateAddress> },
    List,
}


impl TemplateSubcommand {
    pub async fn handle(self, client: ValidatorNodeClient) -> Result<(), anyhow::Error> {
        #[allow(clippy::enum_glob_use)]
        use TemplateSubcommand::*;
        match self {
            Get { template_address } => handle_get(template_address.into_inner(), client).await?,
            List => handle_list(client).await?,
        }
        Ok(())
    }
}

async fn handle_get(template_address: TemplateAddress, mut client: ValidatorNodeClient) -> Result<(), anyhow::Error> {
    let GetTemplateResponse {
        registration_metadata,
        abi,
    } = client.get_template(GetTemplateRequest { template_address }).await?;
    println!(
        "Template {} | Mined at {}",
        registration_metadata.address, registration_metadata.height
    );
    println!();

    let mut table = Table::new();
    table.set_titles(vec!["Function", "Args", "Returns"]);
    for f in abi.functions {
        table.add_row(table_row![
            format!("{}::{}", abi.template_name, f.name),
            f.arguments
                .iter()
                .map(|a| format!("{}:{}", a.name, a.arg_type))
                .collect::<Vec<_>>()
                .join(","),
            f.output
        ]);
    }
    table.print_stdout();

    Ok(())
}

async fn handle_list(mut client: ValidatorNodeClient) -> Result<(), anyhow::Error> {
    let templates = client.get_active_templates(GetTemplatesRequest { limit: 10 }).await?;

    let mut table = Table::new();
    table
        .set_titles(vec!["Name", "Address", "Download Url", "Mined Height", "Status"])
        .enable_row_count();
    for template in templates.templates {
        table.add_row(table_row![
            template.name,
            template.address,
            template.url,
            template.height,
            "Active"
        ]);
    }
    table.print_stdout();
    Ok(())
}
