//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_validator_node_client::types::{GetTemplatesRequest, GetTemplatesResponse, TemplateMetadata};
use url::Url;

pub async fn get_templates(vn_url: &Url) -> anyhow::Result<(TemplateMetadata, TemplateMetadata)> {
    let mut client = tari_validator_node_client::ValidatorNodeClient::connect(vn_url.clone())?;
    let GetTemplatesResponse { templates } = client.get_active_templates(GetTemplatesRequest { limit: 100 }).await?;

    let faucet = templates
        .iter()
        .find(|t| t.name.to_ascii_lowercase() == "faucet")
        .ok_or(anyhow::anyhow!("Faucet template not found"))?
        .clone();

    let tariswap = templates
        .iter()
        .find(|t| t.name.to_ascii_lowercase() == "tariswap")
        .ok_or(anyhow::anyhow!("Tariswap template not found"))?
        .clone();

    Ok((faucet, tariswap))
}
