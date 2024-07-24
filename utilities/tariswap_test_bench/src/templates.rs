//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_validator_node_client::types::{GetTemplatesRequest, GetTemplatesResponse, TemplateMetadata};

use crate::cli::CommonArgs;

pub async fn get_templates(cli: &CommonArgs) -> anyhow::Result<(TemplateMetadata, TemplateMetadata)> {
    let mut client = tari_validator_node_client::ValidatorNodeClient::connect(cli.validator_node_url.clone())?;
    let GetTemplatesResponse { templates } = client.get_active_templates(GetTemplatesRequest { limit: 100 }).await?;

    let tariswap = if let Some(template_address) = cli.faucet_template {
        templates
            .iter()
            .find(|t| t.address == template_address)
            .ok_or(anyhow::anyhow!("Tariswap template not found"))?
            .clone()
    } else {
        templates
            .iter()
            .find(|t| t.name == "TariSwapPool")
            .ok_or(anyhow::anyhow!("Tariswap template not found"))?
            .clone()
    };

    let faucet = templates
        .iter()
        .find(|t| t.name == "TestFaucet")
        .ok_or(anyhow::anyhow!("Faucet template not found"))?
        .clone();

    log::info!("Faucet template: {}", faucet.address);
    log::info!("Tariswap template: {}", tariswap.address);

    Ok((faucet, tariswap))
}
