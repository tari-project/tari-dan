use log::*;
use tari_common_types::types::FixedHash;
use tari_dan_core::{
    storage::{chain::DbTemplate, DbFactory},
    DigitalAssetError,
};

use crate::SqliteDbFactory;

const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

// TODO: integrate this definition into the engine
#[allow(dead_code)]
pub struct TemplateMetadata {
    address: FixedHash,
    url: String,
}

pub struct TemplateManager {
    #[allow(dead_code)]
    db_factory: SqliteDbFactory,
}

impl TemplateManager {
    pub fn new(db_factory: SqliteDbFactory) -> Self {
        // TODO: preload some example templates
        Self { db_factory }
    }

    pub async fn add_templates(&self, templates_metadata: Vec<TemplateMetadata>) {
        info!(target: LOG_TARGET, "Adding {} new templates", templates_metadata.len(),);
        // TODO: retrieve the wasm binary by calling the URl in the metadata
        // TODO: store the template, including the wasm binary, in the database
    }

    #[allow(dead_code)]
    fn store_template_in_db(
        &self,
        template_metadata: TemplateMetadata,
        template_wasm: Vec<u8>,
    ) -> Result<(), DigitalAssetError> {
        let template = DbTemplate {
            template_address: template_metadata.address,
            url: template_metadata.url,
            height: 0, // TODO: pass the height of the block
            compiled_code: template_wasm,
        };

        let template_db = self.db_factory.get_or_create_template_db()?;
        template_db.insert_template(&template)?;

        Ok(())
    }
}
