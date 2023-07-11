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

use blst::min_sig::{SecretKey as BlsSecretKey};
use log::{error, info, warn};
use tari_app_grpc::tari_rpc::RegisterValidatorNodeResponse;
use tari_base_node_client::BaseNodeClientError;
use tari_common::configuration::bootstrap::{grpc_default_port, ApplicationType};
use tari_comms::NodeIdentity;
use tari_dan_common_types::Epoch;
use tari_dan_storage_sqlite::error::SqliteStorageError;
use tari_epoch_manager::{
    base_layer::{EpochManagerError, EpochManagerEvent, EpochManagerHandle},
    EpochManager,
};
use tari_shutdown::ShutdownSignal;
use tokio::{task, task::JoinHandle, time};

use crate::{
    grpc::base_layer_wallet::{GrpcWalletClient, WalletGrpcError},
    validator_node_identity::ValidatorNodeIdentity,
    ApplicationConfig,
};

const LOG_TARGET: &str = "tari::dan::validator_node::auto_registration";
const MAX_REGISTRATION_ATTEMPTS: u8 = 8;
const REGISTRATION_COOLDOWN_IN_MS: u32 = 350;

#[derive(Debug, thiserror::Error)]
pub enum AutoRegistrationError {
    #[error("Registration failed: {details}")]
    RegistrationFailed { details: String },
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Sqlite storage error: {0}")]
    SqliteStorageError(#[from] SqliteStorageError),
    #[error("Base node error: {0}")]
    BaseNodeError(#[from] BaseNodeClientError),
    #[error("Wallet GRPC error: {0}")]
    WalletGrpcError(#[from] WalletGrpcError),
}

pub async fn register(
    mut wallet_client: GrpcWalletClient,
    validator_node_identity: &ValidatorNodeIdentity,
    epoch_manager: &EpochManagerHandle,
) -> Result<RegisterValidatorNodeResponse, AutoRegistrationError> {
    let balance = wallet_client.get_balance().await?;
    let constants = epoch_manager.get_base_layer_consensus_constants().await?;
    if balance.available_balance == constants.validator_node_registration_min_deposit_amount().as_u64() {
        return Err(AutoRegistrationError::RegistrationFailed {
            details: format!(
                "Wallet does not have sufficient balance to pay for registration. Available funds: {}",
                balance.available_balance
            ),
        });
    }

    let mut attempts = 1;

    loop {
        match wallet_client.register_validator_node(validator_node_identity).await {
            Ok(resp) => {
                let tx_id = resp.transaction_id;
                info!(
                    target: LOG_TARGET,
                    "✅ Validator node registration submitted (tx_id: {})", tx_id
                );

                let current_epoch = epoch_manager.current_epoch().await?;
                epoch_manager.update_last_registration_epoch(current_epoch).await?;

                return Ok(resp);
            },
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "❌ Validator node registration failed with error {}. Trying again, attempt {} of {}.",
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
) -> JoinHandle<Result<(), anyhow::Error>> {
    info!(target: LOG_TARGET, "♽️ Node configured for auto registration");

    task::spawn(async move {
        start(config, node_identity, epoch_manager, shutdown).await?;
        Ok(())
    })
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
                    EpochManagerEvent::EpochChanged(epoch) => {
                        if let Err(err) = handle_epoch_changed(&config, &node_identity, &epoch_manager).await {
                            error!(target: LOG_TARGET, "Auto-registration failed for epoch {} with error: {}", epoch, err);
                        }
                    }
                }
            },
            _ = shutdown.wait() => break
        }
    }

    Ok(())
}

async fn handle_epoch_changed(
    config: &ApplicationConfig,
    node_identity: &NodeIdentity,
    epoch_manager: &EpochManagerHandle,
) -> Result<(), AutoRegistrationError> {
    if epoch_manager.last_registration_epoch().await?.is_none() {
        info!(
            target: LOG_TARGET,
            "📋️ Validator has never registered. Auto-registration will only occur after initial registration."
        );
        return Ok(());
    }

    let remaining_epochs = epoch_manager.remaining_registration_epochs().await?.unwrap_or(Epoch(0));
    if remaining_epochs.is_zero() {
        let wallet_client = GrpcWalletClient::new(config.validator_node.wallet_grpc_address.unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::ConsoleWallet, config.network);
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
        }));

        let consensus_secret_key = BlsSecretKey::from_bytes(
            &std::fs::read(config.validator_node.consensus_secret_key_file.clone())
                .map_err(|e| AutoRegistrationError::RegistrationFailed { details: format!("{:?}", e) })?,
        ).map_err(|e| AutoRegistrationError::RegistrationFailed { details: format!("{:?}", e) })?;
        let validator_node_identity =
            ValidatorNodeIdentity::new(node_identity.clone(), consensus_secret_key.clone());
        register(wallet_client, &validator_node_identity, epoch_manager).await?;
    } else {
        info!(
            target: LOG_TARGET,
            "📋️ Validator is already registered or has already submitted registration. Auto-registration will occur \
             in {} epochs.",
            remaining_epochs
        );
    }

    Ok(())
}
