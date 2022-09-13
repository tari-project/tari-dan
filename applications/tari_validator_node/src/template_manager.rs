use log::*;
use tari_common_types::types::FixedHash;

const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

// TODO: integrate this definition into the engine
#[allow(dead_code)]
pub struct TemplateMetadata {
    address: FixedHash,
    url: String,
}

pub struct TemplateManager {}

impl TemplateManager {
    pub fn new() -> Self {
        // TODO: preload some example templates
        Self {}
    }

    pub async fn add_templates(&self, templates_metadata: Vec<TemplateMetadata>) {
        info!(target: LOG_TARGET, "Adding {} new templates", templates_metadata.len(),);
        // TODO: retrieve the wasm binary by calling the URl in the metadata
        // TODO: store the template, including the wasm binary, in the database
    }
}
