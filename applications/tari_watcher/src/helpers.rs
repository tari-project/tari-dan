// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use minotari_app_grpc::tari_rpc::GetActiveValidatorNodesResponse;
use tari_common_types::types::PublicKey;
use tari_core::transactions::transaction_components::ValidatorNodeSignature;
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tokio::fs;

use crate::config::Config;

pub async fn read_config_file(path: PathBuf) -> anyhow::Result<Config> {
    let content = fs::read_to_string(&path).await.map_err(|_| {
        format!(
            "Failed to read config file at {}",
            path.into_os_string().into_string().unwrap()
        )
    });

    let config = toml::from_str(&content.unwrap())?;

    Ok(config)
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ValidatorNodeRegistration {
    pub signature: ValidatorNodeSignature,
    pub public_key: PublicKey,
    pub claim_fees_public_key: PublicKey,
}

pub async fn read_registration_file(vn_registration_file: PathBuf) -> anyhow::Result<ValidatorNodeRegistration> {
    log::debug!("Using VN registration file at: {}", vn_registration_file.display());

    let info = fs::read_to_string(vn_registration_file).await?;
    let reg = json5::from_str(&info)?;
    Ok(reg)
}

pub fn to_vn_public_keys(vns: Vec<GetActiveValidatorNodesResponse>) -> Vec<PublicKey> {
    vns.into_iter()
        .map(|vn| PublicKey::from_vec(&vn.public_key).expect("Invalid public key, should not happen"))
        .collect()
}

pub fn contains_key(vns: Vec<RistrettoPublicKey>, needle: PublicKey) -> bool {
    vns.iter().any(|vn| vn.eq(&needle))
}
