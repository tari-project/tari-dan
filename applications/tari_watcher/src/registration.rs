// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::FixedHash;
use tokio::time::{self, Duration, MissedTickBehavior};

use crate::{
    config::Config,
    helpers::{contains_key, is_close_to_expiry, read_registration_file, to_vn_public_keys},
    manager::ManagerHandle,
};

// TODO: make configurable
// Amount of time to wait before the watcher runs a check again
const REGISTRATION_LOOP_INTERVAL: Duration = Duration::from_secs(30);

// Periodically checks that the local node is still registered on the network.
// If it is no longer registered or close to expiry (1 epoch of blocks or less), it will attempt to re-register.
// It will do nothing if it is registered already and not close to expiry.
pub async fn registration_loop(config: Config, mut handle: ManagerHandle) -> anyhow::Result<ManagerHandle> {
    let mut interval = time::interval(REGISTRATION_LOOP_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_block_hash: Option<FixedHash> = None;
    let mut last_registered: Option<u64> = None;
    let mut recently_registered = false;

    loop {
        interval.tick().await;

        let Some(vn_reg_data) = read_registration_file(&config.vn_registration_file).await? else {
            info!("No registration data found, will try again in 30s");
            continue;
        };
        let public_key = vn_reg_data.public_key;
        debug!("Local public key: {}", public_key.clone());

        let tip_info = handle.get_tip_info().await;
        if let Err(e) = tip_info {
            error!("Failed to get tip info: {}", e);
            continue;
        }

        let current_block = tip_info.as_ref().unwrap().height();
        if last_block_hash.is_none() || last_block_hash.unwrap() != tip_info.as_ref().unwrap().hash() {
            last_block_hash = Some(tip_info.unwrap().hash());
            debug!(
                "New block hash at height {}: {}",
                current_block,
                last_block_hash.unwrap()
            );
        } else {
            debug!("Same block as previous tick");
        }

        let vn_status = handle.get_active_validator_nodes().await;
        if let Err(e) = vn_status {
            error!("Failed to get active validators: {}", e);
            continue;
        }
        let active_keys = to_vn_public_keys(vn_status.unwrap());
        info!("Amount of active validator node keys: {}", active_keys.len());
        for key in &active_keys {
            info!("{}", key);
        }

        let constants = handle.get_consensus_constants(current_block).await;
        if let Err(e) = constants {
            error!("Failed to get consensus constants: {}", e);
            continue;
        }

        // if the node is already registered and not close to expiring in the next epoch, skip registration
        if contains_key(active_keys.clone(), public_key.clone()) &&
            !is_close_to_expiry(constants.unwrap(), current_block, last_registered) ||
            recently_registered
        {
            info!("VN has an active registration and will not expire in the next epoch, skip");
            recently_registered = false;
            continue;
        }

        // if we are not currently registered or close to expiring, attempt to register

        info!("VN not active or about to expire, attempting to register..");
        let tx = handle.register_validator_node(current_block).await;
        if let Err(e) = tx {
            error!("Failed to register VN: {}", e);
            continue;
        }
        let tx = tx.unwrap();
        if !tx.is_success {
            error!("Failed to register VN: {}", tx.failure_message);
            continue;
        }
        info!(
            "Registered VN at block {} with transaction id: {}",
            current_block, tx.transaction_id
        );

        last_registered = Some(current_block);
        // give the network another tick to process the registration
        recently_registered = true;
    }
}
