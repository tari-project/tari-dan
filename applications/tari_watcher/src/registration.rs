// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::FixedHash;
use tokio::time::{self, Duration};

use crate::{
    config::Config,
    helpers::{contains_key, read_registration_file, to_vn_public_keys},
    manager::ManagerHandle,
};

// TODO: make configurable
// Amount of time to wait before the watcher runs a check again
const REGISTRATION_LOOP_INTERVAL: Duration = Duration::from_secs(30);

// `registration_loop` periodically checks that the local node is still registered on the network.
// If it is no longer registered, it will attempt to re-register. It will do nothing if it is registered already.
// Currently, it will not keep track of when the registration was sent or register just in time before it expires.
// It is possible to add a threshold such as sending a registration request every (e.g.) 500 blocks to make sure it it
// always registered.
pub async fn registration_loop(config: Config, mut manager_handle: ManagerHandle) -> anyhow::Result<ManagerHandle> {
    let mut interval = time::interval(REGISTRATION_LOOP_INTERVAL);
    let constants = manager_handle.get_consensus_constants(0).await?;
    let total_blocks_duration = constants.validator_node_validity_period * constants.epoch_length;
    info!(
        "Registrations are currently valid for {} blocks ({} epochs)",
        total_blocks_duration, constants.validator_node_validity_period
    );
    let local_node = read_registration_file(config.vn_registration_file).await?;
    let local_key = local_node.public_key;
    debug!("Local public key: {}", local_key.clone());
    let mut last_block_hash: Option<FixedHash> = None;

    loop {
        interval.tick().await;

        let tip_info = manager_handle.get_tip_info().await;
        if let Err(e) = tip_info {
            error!("Failed to get tip info: {}", e);
            continue;
        }
        let curr_height = tip_info.as_ref().unwrap().height();
        if last_block_hash.is_none() || last_block_hash.unwrap() != tip_info.as_ref().unwrap().hash() {
            last_block_hash = Some(tip_info.unwrap().hash());
            debug!("New block hash at height {}: {}", curr_height, last_block_hash.unwrap());
        } else {
            debug!("Same block as previous tick");
        }

        let vn_status = manager_handle.get_active_validator_nodes().await;
        if let Err(e) = vn_status {
            error!("Failed to get active validators: {}", e);
            continue;
        }
        let active_keys = to_vn_public_keys(vn_status.unwrap());
        info!("Amount of active validator node keys: {}", active_keys.len());
        for key in &active_keys {
            info!("{}", key);
        }

        // if the node is already registered and still valid, skip registration
        if contains_key(active_keys.clone(), local_key.clone()) {
            info!("Node has an active registration, skip");
            continue;
        }

        info!("Local node not active or about to expire, attempting to register..");
        let tx = manager_handle.register_validator_node(curr_height).await;
        if let Err(e) = tx {
            error!("Failed to register node: {}", e);
            continue;
        }
        let tx = tx.unwrap();
        if !tx.is_success {
            error!("Failed to register node: {}", tx.failure_message);
            continue;
        }
        info!(
            "Registered node at block {} with transaction id: {}",
            curr_height, tx.transaction_id
        );
    }
}
