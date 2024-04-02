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
    time::Duration,
};

use log::{error, info, warn};
use minotari_app_grpc::tari_rpc::RegisterValidatorNodeResponse;
use tari_base_node_client::BaseNodeClientError;
use tari_common::configuration::bootstrap::{grpc_default_port, ApplicationType};
use tari_dan_app_utilities::keypair::RistrettoKeypair;
use tari_dan_common_types::{Epoch, PeerAddress};
use tari_dan_storage_sqlite::error::SqliteStorageError;
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerError, EpochManagerEvent, EpochManagerReader};
use tari_shutdown::ShutdownSignal;
use tokio::{task, task::JoinHandle, time};

use crate::{
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
    #[error("Fee claim public key not set")]
    FeeClaimPublicKeyNotSet,
}

pub fn spawn(
    config: ApplicationConfig,
    keypair: RistrettoKeypair,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    shutdown: ShutdownSignal,
) -> JoinHandle<Result<(), anyhow::Error>> {
    info!(target: LOG_TARGET, "♽️ Node configured for auto registration");

    task::spawn(async move {
        start(config, keypair, epoch_manager, shutdown).await?;
        Ok(())
    })
}

async fn start(
    config: ApplicationConfig,
    keypair: RistrettoKeypair,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    mut shutdown: ShutdownSignal,
) -> Result<(), AutoRegistrationError> {
    let mut rx = epoch_manager.subscribe().await?;

    loop {
        tokio::select! {
            Ok(event) = rx.recv() => {
                match event {
                    EpochManagerEvent::EpochChanged(epoch) => {
                        if let Err(err) = handle_epoch_changed(&config, &keypair, &epoch_manager).await {
                            error!(target: LOG_TARGET, "Auto-registration failed for epoch {} with error: {}", epoch, err);
                        }
                    },
                    EpochManagerEvent::ThisValidatorIsRegistered {..} => {}
                }
            },
            _ = shutdown.wait() => break
        }
    }

    Ok(())
}

async fn handle_epoch_changed(
    config: &ApplicationConfig,
    keypair: &RistrettoKeypair,
    epoch_manager: &EpochManagerHandle<PeerAddress>,
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


        warn!(
            target: LOG_TARGET,
            "📋️ Validator has not registered for the current epoch. Auto-registration TODO"
        );
        todo!();
        //register(wallet_client, keypair, epoch_manager).await?;
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
