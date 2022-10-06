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

use futures::future::join_all;
use tari_common_types::types::FixedHash;
use tari_core::transactions::transaction_components::CodeTemplateRegistration;
use tari_dan_core::{services::TemplateProvider, storage::DbFactory};
use tari_dan_engine::wasm::WasmModule;
use tari_dan_storage::global::DbTemplate;
use tari_template_lib::models::TemplateAddress;

use crate::{p2p::services::template_manager::TemplateManagerError, SqliteDbFactory};

const _LOG_TARGET: &str = "tari::validator_node::template_manager";

#[derive(Debug, Clone)]
pub struct TemplateMetadata {
    address: FixedHash,
    // this must be in the form of "https://example.com/my_template.wasm"
    url: String,
    // block height in which the template was published
    height: u64,
}

impl From<CodeTemplateRegistration> for TemplateMetadata {
    fn from(reg: CodeTemplateRegistration) -> Self {
        TemplateMetadata {
            address: reg.hash(),
            url: reg.binary_url.to_string(),
            height: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Template {
    pub metadata: TemplateMetadata,
    pub compiled_code: Vec<u8>,
}

// we encapsulate the db row format to not expose it to the caller
impl From<DbTemplate> for Template {
    fn from(record: DbTemplate) -> Self {
        Template {
            metadata: TemplateMetadata {
                address: record.template_address,
                url: record.url,
                height: record.height,
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

    pub fn fetch_template(&self, address: &TemplateAddress) -> Result<Template, TemplateManagerError> {
        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db.create_transaction()?;
        let template = db
            .templates(&tx)
            .get_template(address)?
            .ok_or(TemplateManagerError::TemplateNotFound { address: *address })?;

        Ok(template.into())
    }

    pub async fn add_templates(
        &self,
        template_registations: Vec<CodeTemplateRegistration>,
    ) -> Result<(), TemplateManagerError> {
        // extract the metadata that we need to store
        let templates_metadata: Vec<TemplateMetadata> = template_registations.into_iter().map(Into::into).collect();

        // we can add each individual template in parallel
        let tasks = templates_metadata.iter().map(|md| self.add_template(md));

        // wait for all templates to be stores
        let results = join_all(tasks).await;

        // propagate any error that may happen
        for result in results {
            result?
        }

        Ok(())
    }

    async fn add_template(&self, template_metadata: &TemplateMetadata) -> Result<(), TemplateManagerError> {
        // fetch the compiled wasm code from the web
        let template_wasm = self.fetch_template_wasm(&template_metadata.url).await?;

        // check that the code we fetched is valid (the template address is the hash)
        // TODO: we will need a consistent way of hashing the template fields
        // let hash = hasher("template").chain(&template_wasm).result();
        // if template_metadata.address.as_slice() != hash.as_slice() {
        //   return Err(TemplateManagerError::TemplateCodeHashMismatch);
        // }

        // finally, store the full template (metadata + wasm binary) in the database
        self.store_template_in_db(template_metadata, template_wasm)?;

        Ok(())
    }

    async fn fetch_template_wasm(&self, url: &str) -> Result<Vec<u8>, TemplateManagerError> {
        let res = reqwest::get(url)
            .await
            .map_err(|_| TemplateManagerError::TemplateCodeFetchError)?;
        let wasm_bytes = res
            .bytes()
            .await
            .map_err(|_| TemplateManagerError::TemplateCodeFetchError)?
            .to_vec();

        Ok(wasm_bytes)
    }

    fn store_template_in_db(
        &self,
        template_metadata: &TemplateMetadata,
        template_wasm: Vec<u8>,
    ) -> Result<(), TemplateManagerError> {
        let template = DbTemplate {
            template_address: template_metadata.address,
            url: template_metadata.url.clone(),
            height: template_metadata.height,
            compiled_code: template_wasm,
        };

        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db.create_transaction()?;
        let template_db = db.templates(&tx);
        template_db.insert_template(template)?;
        db.commit(tx)?;

        Ok(())
    }
}

impl TemplateProvider for TemplateManager {
    type Error = TemplateManagerError;
    type Template = WasmModule;

    fn get_template(&self, address: &TemplateAddress) -> Result<Self::Template, Self::Error> {
        let template = self.fetch_template(address)?;
        Ok(WasmModule::from_code(template.compiled_code))
    }
}
