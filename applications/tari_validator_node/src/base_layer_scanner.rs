//  Copyright 2022, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{convert::TryInto, time::Duration};

use log::*;
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_core::transactions::transaction_components::CodeTemplateRegistration;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_core::{
    models::BaseLayerMetadata,
    services::{base_node_error::BaseNodeError, epoch_manager::EpochManagerError, BaseNodeClient},
    DigitalAssetError,
};
use tari_dan_storage::global::{GlobalDb, MetadataKey};
use tari_dan_storage_sqlite::{error::SqliteStorageError, global::SqliteGlobalDbAdapter};
use tari_shutdown::ShutdownSignal;
use tokio::{task, time};

use crate::{
    p2p::services::{
        epoch_manager::handle::EpochManagerHandle,
        template_manager::{handle::TemplateManagerHandle, TemplateManagerError},
    },
    GrpcBaseNodeClient,
    ValidatorNodeConfig,
};

const LOG_TARGET: &str = "tari::validator_node::base_layer_scanner";

pub fn spawn(
    config: ValidatorNodeConfig,
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    base_node_client: GrpcBaseNodeClient,
    epoch_manager: EpochManagerHandle,
    template_manager: TemplateManagerHandle,
    shutdown: ShutdownSignal,
) {
    task::spawn(async move {
        let base_layer_scanner = BaseLayerScanner::new(
            config,
            global_db,
            base_node_client,
            epoch_manager,
            template_manager,
            shutdown,
        );

        if let Err(err) = base_layer_scanner.start().await {
            error!(target: LOG_TARGET, "Base layer scanner failed with error: {}", err);
        }
    });
}

pub struct BaseLayerScanner {
    config: ValidatorNodeConfig,
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    last_scanned_height: u64,
    last_scanned_hash: Option<FixedHash>,
    base_node_client: GrpcBaseNodeClient,
    epoch_manager: EpochManagerHandle,
    template_manager: TemplateManagerHandle,
    shutdown: ShutdownSignal,
}

impl BaseLayerScanner {
    pub fn new(
        config: ValidatorNodeConfig,
        global_db: GlobalDb<SqliteGlobalDbAdapter>,
        base_node_client: GrpcBaseNodeClient,
        epoch_manager: EpochManagerHandle,
        template_manager: TemplateManagerHandle,
        shutdown: ShutdownSignal,
    ) -> Self {
        Self {
            config,
            global_db,
            last_scanned_height: 0,
            last_scanned_hash: None,
            base_node_client,
            epoch_manager,
            template_manager,
            shutdown,
        }
    }

    pub async fn start(mut self) -> Result<(), BaseLayerScannerError> {
        if !self.config.scan_base_layer {
            info!(
                target: LOG_TARGET,
                "⚠️ scan_base_layer turned OFF. Base layer scanner is exiting."
            );
            return Ok(());
        }

        self.load_initial_state()?;

        loop {
            // fetch the new base layer info since the previous scan
            let tip = self.base_node_client.get_tip_info().await?;
            if tip.height_of_longest_chain > self.last_scanned_height {
                // let new_blocks = self
                //     .base_node_client
                //     .get_blocks(self.last_scanned_hash, tip.height_of_longest_chain)
                //     .await?;
                for height in self.last_scanned_height + 1..=tip.height_of_longest_chain {
                    self.process_block(height).await?;
                }
                self.set_last_scanned_block(&tip)?;
            } else {
                tokio::select! {
                   _ = time::sleep(Duration::from_secs(self.config.base_layer_scanning_interval_in_seconds)) => {},
                   _ = &mut self.shutdown => break
                }
            }
        }

        Ok(())
    }

    // TODO: Use hashes instead of height to avoid reorg problems
    async fn process_block(&mut self, height: u64) -> Result<(), BaseLayerScannerError> {
        let template_registrations = self.scan_for_new_templates(height).await?;

        // both epoch and template tasks are I/O bound,
        // so they can be ran concurrently as they do not block CPU between them
        let epoch_task = self.epoch_manager.update_epoch(height);
        let template_task = self.template_manager.add_templates(template_registrations);

        // wait for all tasks to finish
        let (epoch_result, template_result) = tokio::join!(epoch_task, template_task);

        if let Err(err) = epoch_result {
            error!(
                target: LOG_TARGET,
                "🚨 Epoch manager failed to update epoch at height {}: {}", height, err
            );
        }

        if let Err(err) = template_result {
            error!(
                target: LOG_TARGET,
                "🚨 Template manager failed to add templates at height {}: {}", height, err
            );
        }

        Ok(())
    }

    fn load_initial_state(&mut self) -> Result<(), BaseLayerScannerError> {
        self.last_scanned_hash = None;
        self.last_scanned_height = 0;
        let tx = self.global_db.create_transaction()?;
        let metadata = self.global_db.metadata(&tx);

        self.last_scanned_hash = metadata
            .get_metadata(MetadataKey::BaseLayerScannerLastScannedBlockHash)?
            .map(TryInto::try_into)
            .transpose()?;
        self.last_scanned_height = metadata
            .get_metadata(MetadataKey::BaseLayerScannerLastScannedBlockHeight)?
            .map(|data| {
                if data.len() == 8 {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(&data);
                    Ok(u64::from_le_bytes(buf))
                } else {
                    Err(BaseLayerScannerError::DataCorruption {
                        details: "LastScannedBaseLayerBlockHeight did not contain little-endian u64 data".to_string(),
                    })
                }
            })
            .transpose()?
            .unwrap_or(0);
        Ok(())
    }

    async fn scan_for_new_templates(
        &mut self,
        height: u64,
    ) -> Result<Vec<CodeTemplateRegistration>, BaseLayerScannerError> {
        info!(
            target: LOG_TARGET,
            "🔍 Scanning base layer (from height: {}) for new templates", self.last_scanned_height
        );

        let template_registrations = self.base_node_client.get_template_registrations(height).await?;
        if !template_registrations.is_empty() {
            info!(
                target: LOG_TARGET,
                "📁 {} new template(s) found",
                template_registrations.len()
            );
        }

        Ok(template_registrations)
    }

    fn set_last_scanned_block(&mut self, tip: &BaseLayerMetadata) -> Result<(), BaseLayerScannerError> {
        let tx = self.global_db.create_transaction()?;
        let metadata = self.global_db.metadata(&tx);
        metadata.set_metadata(
            MetadataKey::BaseLayerScannerLastScannedBlockHash,
            tip.tip_hash.as_bytes(),
        )?;
        metadata.set_metadata(
            MetadataKey::BaseLayerScannerLastScannedBlockHeight,
            &tip.height_of_longest_chain.to_le_bytes(),
        )?;
        self.global_db.commit(tx)?;
        self.last_scanned_hash = Some(tip.tip_hash);
        self.last_scanned_height = tip.height_of_longest_chain;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BaseLayerScannerError {
    #[error(transparent)]
    FixedHashSizeError(#[from] FixedHashSizeError),
    #[error("Storage error: {0}")]
    SqliteStorageError(#[from] SqliteStorageError),
    #[error("DigitalAsset error: {0}")]
    DigitalAssetError(#[from] DigitalAssetError),
    #[error("Data corruption: {details}")]
    DataCorruption { details: String },
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Template manager error: {0}")]
    TemplateManagerError(#[from] TemplateManagerError),
    #[error("Base node client error: {0}")]
    BaseNodeError(#[from] BaseNodeError),
}
