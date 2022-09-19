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
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_core::{
    models::BaseLayerMetadata,
    services::{base_node_error::BaseNodeError, epoch_manager::EpochManagerError, BaseNodeClient},
    storage::{
        global::{GlobalDb, GlobalDbMetadataKey},
        StorageError,
    },
    DigitalAssetError,
};
use tari_dan_storage_sqlite::global::SqliteGlobalDbBackendAdapter;
use tari_shutdown::ShutdownSignal;
use tokio::time;

use crate::{
    p2p::services::{
        epoch_manager::handle::EpochManagerHandle,
        template_manager::{handle::TemplateManagerHandle, template_manager::TemplateMetadata, TemplateManagerError},
    },
    GrpcBaseNodeClient,
    ValidatorNodeConfig,
};

const LOG_TARGET: &str = "tari::validator_node::base_layer_scanner";

pub struct BaseLayerScanner {
    config: ValidatorNodeConfig,
    global_db: GlobalDb<SqliteGlobalDbBackendAdapter>,
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
        global_db: GlobalDb<SqliteGlobalDbBackendAdapter>,
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
        self.load_initial_state()?;

        if !self.config.scan_base_layer {
            info!(
                target: LOG_TARGET,
                "⚠️ scan_base_layer turned OFF. Base layer scanner is shutting down."
            );
            self.shutdown.await;
            return Ok(());
        }

        loop {
            // fetch the new base layer info since the previous scan
            let tip = self.base_node_client.get_tip_info().await?;
            // let block = self.base_node_client.get_block(tip.height).await?;
            let new_templates_metadata = self.scan_for_new_templates(&tip).await?;

            // both epoch and template tasks are I/O bound,
            // so they can be ran concurrently as they do not block CPU between them
            let epoch_task = self.epoch_manager.update_epoch(tip.clone());
            let template_task = self.template_manager.add_templates(new_templates_metadata);

            // wait for all tasks to finish
            let results = tokio::join!(epoch_task, template_task);

            // propagate any error that may happen
            // TODO: there could be a cleaner way of propagating the errors of the individual tasks
            // TODO: maybe we want to be resilient to invalid data in base layer and just log the error?
            results.0?;
            results.1?;

            // setup the next scan cycle
            self.set_last_scanned_block(&tip)?;
            tokio::select! {
                _ = time::sleep(Duration::from_secs(self.config.base_layer_scanning_interval_in_seconds)) => {},
                _ = &mut self.shutdown => break
            }
        }

        Ok(())
    }

    fn load_initial_state(&mut self) -> Result<(), BaseLayerScannerError> {
        self.last_scanned_hash = None;
        self.last_scanned_height = 0;

        self.last_scanned_hash = self
            .global_db
            .get_data(GlobalDbMetadataKey::LastScannedBaseLayerBlockHash)?
            .map(TryInto::try_into)
            .transpose()?;
        self.last_scanned_height = self
            .global_db
            .get_data(GlobalDbMetadataKey::LastScannedBaseLayerBlockHeight)?
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
        tip: &BaseLayerMetadata,
    ) -> Result<Vec<TemplateMetadata>, BaseLayerScannerError> {
        info!(
            target: LOG_TARGET,
            "🔍 Scanning base layer (tip: {}) for new templates", tip.height_of_longest_chain
        );

        Ok(vec![])

        // TODO: when template publishing is implemented in the base layer, uncomment this code for real base layer
        // scanning let outputs = self
        // .base_node_client
        // .get_templates(self.last_scanned_hash, self.identity.public_key())
        // .await?;
        //
        // let mut new_templates = vec![];
        //
        // for utxo in outputs {
        // let output = some_or_continue!(utxo.output.into_unpruned_output());
        // let mined_height = utxo.mined_height;
        // let sidechain_features = some_or_continue!(output.features.sidechain_features);
        // let template = sidechain_features.template;
        // new_contracts.push(contract);
        // }
        //
        // info!(target: LOG_TARGET, "{} new template(s) found", new_templates.len());
        // Ok(new_templates)
    }

    fn set_last_scanned_block(&mut self, tip: &BaseLayerMetadata) -> Result<(), BaseLayerScannerError> {
        self.global_db.set_data(
            GlobalDbMetadataKey::LastScannedBaseLayerBlockHash,
            tip.tip_hash.as_bytes(),
        )?;
        self.global_db.set_data(
            GlobalDbMetadataKey::LastScannedBaseLayerBlockHeight,
            &tip.height_of_longest_chain.to_le_bytes(),
        )?;
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
    StorageError(#[from] StorageError),
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

// macro_rules! some_or_continue {
// ($expr:expr) => {
// match $expr {
// Some(x) => x,
// None => continue,
// }
// };
// }
