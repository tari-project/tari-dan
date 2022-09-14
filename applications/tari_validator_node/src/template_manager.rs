use futures::future::join_all;
use log::*;
use tari_common_types::types::FixedHash;
use tari_dan_core::{
    storage::{chain::DbTemplate, DbFactory},
    DigitalAssetError,
};

use crate::SqliteDbFactory;

const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

pub struct TemplateMetadata {
    address: FixedHash,
    // this must be in the form of "https://example.com/my_template.wasm"
    url: String,
}

#[allow(dead_code)]
pub struct Template {
    metadata: TemplateMetadata,
    compiled_code: Vec<u8>,
}

// we encapsulate the db row format to not expose it to the caller
impl From<DbTemplate> for Template {
    fn from(record: DbTemplate) -> Self {
        Template {
            metadata: TemplateMetadata {
                address: record.template_address,
                url: record.url,
            },
            compiled_code: record.compiled_code,
        }
    }
}

pub struct TemplateManager {
    db_factory: SqliteDbFactory,
}

impl TemplateManager {
    pub fn new(db_factory: SqliteDbFactory) -> Self {
        // TODO: preload some example templates
        Self { db_factory }
    }

    // to be used in the future by the engine to retrieve the wasm code for transaction execution
    #[allow(dead_code)]
    pub async fn get_template(&self, address: &FixedHash) -> Result<Option<Template>, DigitalAssetError> {
        let db = self.db_factory.get_or_create_template_db()?;
        let result = db.find_template_by_address(address)?;

        match result {
            Some(db_template) => Ok(Some(db_template.into())),
            None => Ok(None),
        }
    }

    pub async fn add_templates(&self, templates_metadata: Vec<TemplateMetadata>) -> Result<(), DigitalAssetError> {
        info!(target: LOG_TARGET, "Adding {} new templates", templates_metadata.len());

        // we can add each individual template in parallel
        let tasks: Vec<_> = templates_metadata.iter().map(|md| self.add_template(md)).collect();

        join_all(tasks).await;

        Ok(())
    }

    async fn add_template(&self, template_metadata: &TemplateMetadata) -> Result<(), DigitalAssetError> {
        // fetch the compiled wasm code from the web
        let template_wasm = self.fecth_template_wasm(&template_metadata.url).await?;

        // check that the code we fetched is valid (the template address is the hash)
        let hash = FixedHash::hash_bytes(&template_wasm);
        if template_metadata.address != hash {
            return Err(DigitalAssetError::TemplateCodeHashMismatch);
        }

        // finally, store the full template (metadata + wasm binary) in the database
        self.store_template_in_db(template_metadata, template_wasm)?;

        Ok(())
    }

    async fn fecth_template_wasm(&self, url: &str) -> Result<Vec<u8>, DigitalAssetError> {
        let res = reqwest::get(url)
            .await
            .map_err(|_| DigitalAssetError::TemplateCodeFetchError)?;
        let wasm_bytes = res
            .bytes()
            .await
            .map_err(|_| DigitalAssetError::TemplateCodeFetchError)?
            .to_vec();

        Ok(wasm_bytes)
    }

    fn store_template_in_db(
        &self,
        template_metadata: &TemplateMetadata,
        template_wasm: Vec<u8>,
    ) -> Result<(), DigitalAssetError> {
        let template = DbTemplate {
            template_address: template_metadata.address,
            url: template_metadata.url.clone(),
            height: 0, // TODO: pass the height of the block
            compiled_code: template_wasm,
        };

        let db = self.db_factory.get_or_create_template_db()?;
        db.insert_template(&template)?;

        Ok(())
    }
}
