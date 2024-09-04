// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    io,
    path::{Path, PathBuf},
};

use minotari_app_grpc::tari_rpc::{ConsensusConstants, GetActiveValidatorNodesResponse};
use tari_common_types::types::PublicKey;
use tari_core::transactions::transaction_components::ValidatorNodeSignature;
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tokio::fs;

use crate::{config::Config, constants::DEFAULT_THRESHOLD_WARN_EXPIRATION};

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

pub async fn read_registration_file<P: AsRef<Path>>(
    vn_registration_file: P,
) -> anyhow::Result<Option<ValidatorNodeRegistration>> {
    log::debug!(
        "Using VN registration file at: {}",
        vn_registration_file.as_ref().display()
    );
    match fs::read_to_string(vn_registration_file).await {
        Ok(info) => {
            let reg = json5::from_str(&info)?;
            Ok(Some(reg))
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => {
            log::error!("Failed to read VN registration file: {}", e);
            Err(e.into())
        },
    }
}

pub fn to_vn_public_keys(vns: Vec<GetActiveValidatorNodesResponse>) -> Vec<PublicKey> {
    vns.into_iter()
        .map(|vn| PublicKey::from_vec(&vn.public_key).expect("Invalid public key, should not happen"))
        .collect()
}

pub fn contains_key(vns: Vec<RistrettoPublicKey>, needle: PublicKey) -> bool {
    vns.iter().any(|vn| vn.eq(&needle))
}

pub fn is_close_to_expiry(
    constants: ConsensusConstants,
    current_block: u64,
    last_registered_block: Option<u64>,
) -> bool {
    // if we haven't registered yet in this session, return false
    if last_registered_block.is_none() {
        return false;
    }
    let epoch_length = constants.epoch_length;
    let validity_period = constants.validator_node_validity_period;
    let registration_duration = validity_period * epoch_length;
    // check if the current block is an epoch or less away from expiring
    current_block + epoch_length >= last_registered_block.unwrap() + registration_duration
}

pub fn is_warning_close_to_expiry(
    constants: ConsensusConstants,
    current_block: u64,
    last_registered_block: u64,
) -> bool {
    let registration_duration = constants.epoch_length * constants.validator_node_validity_period;
    // if we have approached the expiration threshold
    current_block + DEFAULT_THRESHOLD_WARN_EXPIRATION >= last_registered_block + registration_duration
}
