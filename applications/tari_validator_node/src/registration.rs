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

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use log::{error, info, warn};
use tari_app_grpc::tari_rpc::RegisterValidatorNodeResponse;
use tari_common::configuration::bootstrap::{grpc_default_port, ApplicationType};
use tari_comms::NodeIdentity;
use tari_dan_common_types::Epoch;
use tari_dan_core::{
    services::{
        base_node_error::BaseNodeError,
        epoch_manager::{EpochManager, EpochManagerError},
    },
    DigitalAssetError,
};
use tari_dan_storage_sqlite::error::SqliteStorageError;
use tari_shutdown::ShutdownSignal;
use tari_wallet_grpc_client::WalletClientError;
use tokio::{task, time};

use crate::{
    grpc::services::base_node_client::GrpcBaseNodeClient,
    p2p::services::epoch_manager::{epoch_manager_service::EpochManagerEvent, handle::EpochManagerHandle},
    ApplicationConfig,
    GrpcWalletClient,
};

const LOG_TARGET: &str = "tari::validator_node::app";
const MAX_REGISTRATION_ATTEMPTS: u8 = 8;
const REGISTRATION_COOLDOWN_IN_MS: u32 = 350;

#[derive(Debug, thiserror::Error)]
pub enum AutoRegistrationError {
    #[error("Registration failed: {details}")]
    RegistrationFailed { details: String },
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Wallet client error: {0}")]
    WalletClientError(#[from] WalletClientError),
    #[error("DigitalAsset error: {0}")]
    DigitalAssetError(#[from] DigitalAssetError),
    #[error("Sqlite storage error: {0}")]
    SqliteStorageError(#[from] SqliteStorageError),
    #[error("Base node error: {0}")]
    BaseNodeError(#[from] BaseNodeError),
    #[error("Math Overflow error")]
    MathOverflow,
}

pub async fn register(
    mut wallet_client: GrpcWalletClient,
    node_identity: &NodeIdentity,
    epoch_manager: &EpochManagerHandle,
) -> Result<RegisterValidatorNodeResponse, AutoRegistrationError> {
    let mut attempts = 1;

    loop {
        match wallet_client.register_validator_node(node_identity).await {
            Ok(resp) => {
                let tx_id = resp.transaction_id;
                info!(
                    target: LOG_TARGET,
                    "??? Validator node registration submitted (tx_id: {})", tx_id
                );

                let current_epoch = epoch_manager.current_epoch().await?;
                epoch_manager.update_last_registration_epoch(current_epoch).await?;

                return Ok(resp);
            },
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "??? Validator node registration failed with error {}. Trying again, attempt {} of {}.",
                    e.to_string(),
                    attempts,
                    MAX_REGISTRATION_ATTEMPTS,
                );

                if attempts >= MAX_REGISTRATION_ATTEMPTS {
                    return Err(AutoRegistrationError::RegistrationFailed { details: e.to_string() });
                }

                time::sleep(Duration::from_millis(u64::from(
                    REGISTRATION_COOLDOWN_IN_MS * u32::from(attempts),
                )))
                .await;

                attempts += 1;
            },
        }
    }
}

pub fn spawn(
    config: ApplicationConfig,
    node_identity: Arc<NodeIdentity>,
    epoch_manager: EpochManagerHandle,
    shutdown: ShutdownSignal,
) {
    if !config.validator_node.auto_register {
        info!(target: LOG_TARGET, "?????? Node auto registration is disabled");
        return;
    }

    info!(target: LOG_TARGET, "?????? Node configured for auto registration");

    task::spawn(async move {
        if let Err(err) = start(config, node_identity, epoch_manager, shutdown).await {
            error!(target: LOG_TARGET, "Auto registration failed to initialize: {}", err);
        }
    });
}

async fn start(
    config: ApplicationConfig,
    node_identity: Arc<NodeIdentity>,
    epoch_manager: EpochManagerHandle,
    mut shutdown: ShutdownSignal,
) -> Result<(), AutoRegistrationError> {
    let mut rx = epoch_manager.subscribe().await?;

    loop {
        tokio::select! {
            Ok(event) = rx.recv() => {
                match event {
                    EpochManagerEvent::EpochChanged(current_epoch) =>
                        handle_epoch_changed(current_epoch, &config, &node_identity, &epoch_manager, ).await?,
                }
            },
            _ = shutdown.wait() => break
        }
    }

    Ok(())
}

async fn handle_epoch_changed(
    current_epoch: Epoch,
    config: &ApplicationConfig,
    node_identity: &NodeIdentity,
    epoch_manager: &EpochManagerHandle,
) -> Result<(), AutoRegistrationError> {
    let last_registration_epoch = epoch_manager.last_registration_epoch().await?.unwrap_or(Epoch(0));

    // TODO: This need to consider the validator node confirmation period and submit a reregistration which the tip
    //       has progressed to the last valid epoch for this node

    let mut base_node_client =
        GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address.unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::BaseNode, config.network);
            ([127, 0, 0, 1], port).into()
        }));

    // TODO: This logic probably need to move into the epoch manager because it is aware of the base layer consensus
    //       constants
    // e.g. if epoch_manager.epochs_until_next_registration() <= 1 { ... }
    let current_block_height = current_epoch.as_u64() * 10 + 1;
    let validator_node_registration_expiry = base_node_client
        .get_consensus_constants(current_block_height)
        .await
        .map_err(AutoRegistrationError::BaseNodeError)?
        .validator_node_registration_expiry()
        .as_u64() *
        10;

    let last_registration_height = last_registration_epoch.as_u64() * 10 + 1;
    let num_blocks_since_last_reg = current_block_height
        .checked_sub(last_registration_height)
        .ok_or(AutoRegistrationError::MathOverflow)?;
    if num_blocks_since_last_reg >= validator_node_registration_expiry {
        let wallet_client = GrpcWalletClient::new(config.validator_node.wallet_grpc_address.unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::ConsoleWallet, config.network);
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
        }));

        register(wallet_client, node_identity, epoch_manager).await?;
    }
    Ok(())
}
