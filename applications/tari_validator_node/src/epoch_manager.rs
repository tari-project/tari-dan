use log::*;
use tari_dan_core::{models::BaseLayerMetadata, DigitalAssetError};

const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

pub struct EpochManager {}

impl EpochManager {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn update_epoch(&self, tip: &BaseLayerMetadata) -> Result<(), DigitalAssetError> {
        info!(
            target: LOG_TARGET,
            "Updating epoch for base layer tip {} ({})", tip.height_of_longest_chain, tip.tip_hash,
        );

        // TODO: calculate and store the epoch in the db

        Ok(())
    }
}
