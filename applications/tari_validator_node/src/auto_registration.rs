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

use std::{sync::Arc, time::Duration};

use log::{error, info};
use tari_comms::NodeIdentity;
use tari_dan_core::{
    services::epoch_manager::{EpochManager, EpochManagerError},
    DigitalAssetError,
};
use tari_shutdown::ShutdownSignal;
use tari_wallet_grpc_client::WalletClientError;
use tokio::{task, time};

use crate::{p2p::services::epoch_manager::handle::EpochManagerHandle, ApplicationConfig, GrpcWalletClient};

const LOG_TARGET: &str = "tari::validator_node::app";

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
}

pub fn spawn(
    config: ApplicationConfig,
    node_identity: Arc<NodeIdentity>,
    epoch_manager: EpochManagerHandle,
    shutdown: ShutdownSignal,
) {
    info!(target: LOG_TARGET, "♽️ Node configured for auto registration");

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
    let mut wallet_client = GrpcWalletClient::new(config.validator_node.wallet_grpc_address);
    let mut current_epoch = epoch_manager.current_epoch().await?;

    loop {
        tokio::select! {
            _ = time::sleep(Duration::from_secs(120)) => {
                let epoch_changed = epoch_manager.current_epoch().await?;

                if current_epoch != epoch_changed {
                    current_epoch = epoch_changed;

                    match wallet_client.register_validator_node(&node_identity).await {
                        Ok(resp) => {
                            let tx_id = resp.transaction_id;
                            info!(target: LOG_TARGET, "✅ Validator node auto registration submitted (tx_id: {})", tx_id);
                        },
                        Err(e) => return Err(AutoRegistrationError::RegistrationFailed {
                          details: e.to_string(),
                        })
                    }
                }
            },
            _ = shutdown.wait() => break
        }
    }

    Ok(())
}
