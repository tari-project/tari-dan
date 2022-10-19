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

use tari_dan_core::{services::TemplateProvider, storage::DbFactory};
use tari_dan_engine::wasm::WasmModule;
use tari_dan_storage::global::{DbTemplate, DbTemplateUpdate, TemplateStatus};
use tari_template_lib::models::TemplateAddress;

use crate::{
    p2p::services::template_manager::{handle::TemplateRegistration, TemplateManagerError},
    SqliteDbFactory,
};

const _LOG_TARGET: &str = "tari::validator_node::template_manager";

#[derive(Debug, Clone)]
pub struct TemplateMetadata {
    pub address: TemplateAddress,
    // this must be in the form of "https://example.com/my_template.wasm"
    pub url: String,
    /// SHA hash of binary
    pub binary_sha: Vec<u8>,
    /// Block height in which the template was published
    pub height: u64,
}

impl From<TemplateRegistration> for TemplateMetadata {
    fn from(reg: TemplateRegistration) -> Self {
        TemplateMetadata {
            address: reg.template_address,
            url: reg.registration.binary_url.into_string(),
            binary_sha: reg.registration.binary_sha.into_vec(),
            height: reg.mined_height,
        }
    }
}

// TODO: Allow fetching of just the template metadata without the compiled code
impl From<DbTemplate> for TemplateMetadata {
    fn from(record: DbTemplate) -> Self {
        TemplateMetadata {
            address: (*record.template_address).into(),
            url: record.url,
            binary_sha: vec![],
            height: record.height,
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
                // TODO: this will change when common engine types are moved around
                address: (*record.template_address).into(),
                url: record.url,
                // TODO: add field to db
                binary_sha: vec![],
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

    pub fn fetch_template_metadata(&self, limit: usize) -> Result<Vec<TemplateMetadata>, TemplateManagerError> {
        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db.create_transaction()?;
        // TODO: we should be able to fetch just the metadata and not the compiled code
        let templates = db.templates(&tx).get_templates(limit)?;
        Ok(templates.into_iter().map(Into::into).collect())
    }

    pub(super) fn add_template(&self, template: TemplateRegistration) -> Result<(), TemplateManagerError> {
        let template = DbTemplate {
            template_address: template.template_address.into_array().into(),
            url: template.registration.binary_url.into_string(),
            height: template.mined_height,
            status: TemplateStatus::New,
            compiled_code: vec![],
            added_at: time::OffsetDateTime::now_utc(),
        };

        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db.create_transaction()?;
        if db.templates(&tx).get_template(&*template.template_address)?.is_some() {
            return Ok(());
        }
        let template_db = db.templates(&tx);
        template_db.insert_template(template)?;
        db.commit(tx)?;

        Ok(())
    }

    pub(super) fn update_template(
        &self,
        address: TemplateAddress,
        update: DbTemplateUpdate,
    ) -> Result<(), TemplateManagerError> {
        let db = self.db_factory.get_or_create_global_db()?;
        let tx = db.create_transaction()?;
        let template_db = db.templates(&tx);
        template_db.update_template(&address, update)?;
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
