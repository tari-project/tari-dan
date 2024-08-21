// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use crate::{
    config::Config,
    helpers::{contains_key, read_registration_file, to_block_height, to_vn_public_keys},
    manager::ManagerHandle,
};
use log::*;
use tokio::time::{self, Duration};

pub async fn registration_loop(config: Config, mut manager_handle: ManagerHandle) -> anyhow::Result<ManagerHandle> {
    let mut interval = time::interval(Duration::from_secs(30));
    let constants = manager_handle.get_consensus_constants(0).await;
    let validity_period = constants.as_ref().unwrap().validator_node_validity_period;
    let epoch_length = constants.unwrap().epoch_length;
    let total_blocks_duration = validity_period * epoch_length;
    debug!(
        "Registrations are currently valid for {} blocks ({} epochs)",
        total_blocks_duration, validity_period
    );
    let mut registered_at_block = 0;
    let local_node = read_registration_file(config.vn_registration_file).await?;
    let local_key = local_node.public_key;
    debug!("Local public key: {}", local_key.clone());
    let mut sent_registration = false;
    let mut counter = 0;

    loop {
        interval.tick().await;

        let tip_info = manager_handle.get_tip_info().await;
        if let Err(e) = tip_info {
            error!("Failed to get tip info: {}", e);
            continue;
        }
        let curr_height = to_block_height(tip_info.unwrap());
        debug!("Current block height: {}", curr_height);

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

        let reg_expiration_block = registered_at_block + total_blocks_duration;
        // if the node is already registered and still valid, skip registration
        if contains_key(active_keys.clone(), local_key.clone()) && curr_height < reg_expiration_block {
            info!("Node has an active registration, skip");
            continue;
        }

        if sent_registration {
            info!("Node is not registered but recently sent a registration request, waiting..");
            counter += 1;

            // waiting 20 minutes
            if counter > 40 {
                error!("Node registration request timed out, retrying..");
                counter = 0;
                sent_registration = false;
            }

            continue;
        }

        info!("Local node not active or about to expire, attempting to register..");
        let tx = manager_handle.register_validator_node().await.unwrap();
        if !tx.is_success {
            error!("Failed to register node: {}", tx.failure_message);
            continue;
        }
        info!(
            "Registered node at height {} with transaction id: {}",
            curr_height, tx.transaction_id
        );
        registered_at_block = curr_height;
        sent_registration = true;
    }
}
