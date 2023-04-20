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

use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    fs,
};

use chrono::Utc;
use log::*;
use tari_core::transactions::transaction_components::TemplateType;
use tari_dan_app_utilities::template_manager::{
    Template,
    TemplateExecutable,
    TemplateManagerError,
    TemplateMetadata,
    TemplateRegistration,
};
use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_dan_engine::{
    flow::FlowFactory,
    function_definitions::FlowFunctionDefinition,
    packager::{LoadedTemplate, TemplateModuleLoader},
    wasm::WasmModule,
};
use tari_dan_storage::global::{DbTemplate, DbTemplateType, DbTemplateUpdate, GlobalDb, TemplateStatus};
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_engine_types::calculate_template_binary_hash;
use tari_template_builtin::get_template_builtin;
use tari_template_lib::models::TemplateAddress;

use crate::p2p::services::template_manager::TemplateConfig;

const LOG_TARGET: &str = "tari::validator_node::template_manager";

pub const ACCOUNT_TEMPLATE_ADDRESS: TemplateAddress = TemplateAddress::from_array([0; 32]);

#[derive(Debug, Clone)]
pub struct TemplateManager {
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    config: TemplateConfig,
    builtin_templates: HashMap<TemplateAddress, Template>,
    cache: mini_moka::sync::Cache<TemplateAddress, LoadedTemplate>,
}

impl TemplateManager {
    pub fn new(global_db: GlobalDb<SqliteGlobalDbAdapter>, config: TemplateConfig) -> Self {
        // load the builtin account templates
        let builtin_templates = Self::load_builtin_templates();

        Self {
            global_db,
            builtin_templates,
            cache: mini_moka::sync::Cache::builder()
                .weigher(|_, t: &LoadedTemplate| u32::try_from(t.code_size()).unwrap_or(u32::MAX))
                .max_capacity(config.max_cache_size_bytes())
                .build(),
            config,
        }
    }

    fn load_builtin_templates() -> HashMap<TemplateAddress, Template> {
        // for now, we only load the "account" template
        let mut builtin_templates = HashMap::new();

        // get the builtin WASM code of the account template
        let compiled_code = get_template_builtin(&ACCOUNT_TEMPLATE_ADDRESS);
        let template = Self::load_builtin_template("account", ACCOUNT_TEMPLATE_ADDRESS, compiled_code.to_vec());
        builtin_templates.insert(ACCOUNT_TEMPLATE_ADDRESS, template);

        builtin_templates
    }

    fn load_builtin_template(name: &str, address: TemplateAddress, compiled_code: Vec<u8>) -> Template {
        let compiled_code_len = compiled_code.len();
        info!(
            target: LOG_TARGET,
            "Loading builtin {} template: {} bytes", name, compiled_code_len
        );

        // build the template object of the account template
        let binary_sha = calculate_template_binary_hash(&compiled_code);
        Template {
            metadata: TemplateMetadata {
                name: name.to_string(),
                address,
                url: "".to_string(),
                binary_sha: binary_sha.to_vec(),
                height: 0,
            },
            executable: TemplateExecutable::CompiledWasm(compiled_code),
        }
    }

    pub fn template_exists(&self, address: &TemplateAddress) -> Result<bool, TemplateManagerError> {
        if self.builtin_templates.contains_key(address) {
            return Ok(true);
        }
        let mut tx = self.global_db.create_transaction()?;
        self.global_db
            .templates(&mut tx)
            .template_exists(address)
            .map_err(|_| TemplateManagerError::TemplateNotFound { address: *address })
    }

    pub fn fetch_template(&self, address: &TemplateAddress) -> Result<Template, TemplateManagerError> {
        // first of all, check if the address is for a bulitin template
        if let Some(template) = self.builtin_templates.get(address) {
            return Ok(template.to_owned());
        }

        let mut tx = self.global_db.create_transaction()?;
        let template = self
            .global_db
            .templates(&mut tx)
            .get_template(address)?
            .ok_or(TemplateManagerError::TemplateNotFound { address: *address })?;

        if !matches!(template.status, TemplateStatus::Active | TemplateStatus::Deprecated) {
            return Err(TemplateManagerError::TemplateUnavailable);
        }

        // first check debug
        if let Some(dbg_replacement) = self.config.debug_replacements().get(address) {
            let mut result: Template = template.into();
            match &mut result.executable {
                TemplateExecutable::CompiledWasm(wasm) => {
                    let binary = fs::read(dbg_replacement).expect("Could not read debug file");
                    *wasm = binary;
                },
                TemplateExecutable::Flow(_) => {
                    todo!("debug replacements for flow templates not implemented");
                },
                _ => return Err(TemplateManagerError::TemplateUnavailable),
            }

            Ok(result)
        } else {
            Ok(template.into())
        }
    }

    pub fn fetch_template_metadata(&self, limit: usize) -> Result<Vec<TemplateMetadata>, TemplateManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        // TODO: we should be able to fetch just the metadata and not the compiled code
        let templates = self.global_db.templates(&mut tx).get_templates(limit)?;
        let mut templates: Vec<TemplateMetadata> = templates.into_iter().map(Into::into).collect();
        let mut builtin_metadata: Vec<TemplateMetadata> =
            self.builtin_templates.values().map(|t| t.metadata.to_owned()).collect();
        templates.append(&mut builtin_metadata);

        Ok(templates)
    }

    pub(super) fn add_template(&self, template: TemplateRegistration) -> Result<(), TemplateManagerError> {
        let template = DbTemplate {
            template_name: template.template_name,
            template_address: template.template_address.into_array().into(),
            expected_hash: template.registration.binary_sha.into_vec().try_into()?,
            url: template.registration.binary_url.into_string(),
            height: template.mined_height,
            status: TemplateStatus::New,
            compiled_code: None,
            added_at: Utc::now().naive_utc(),
            template_type: match template.registration.template_type {
                TemplateType::Wasm { .. } => DbTemplateType::Wasm,
                TemplateType::Flow { .. } => DbTemplateType::Flow,
                TemplateType::Manifest { .. } => DbTemplateType::Manifest,
            },
            flow_json: None,
            manifest: None,
        };

        let mut tx = self.global_db.create_transaction()?;
        let mut templates_db = self.global_db.templates(&mut tx);
        if templates_db.get_template(&*template.template_address)?.is_some() {
            return Ok(());
        }
        templates_db.insert_template(template)?;
        tx.commit()?;

        Ok(())
    }

    pub(super) fn update_template(
        &self,
        address: TemplateAddress,
        update: DbTemplateUpdate,
    ) -> Result<(), TemplateManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut template_db = self.global_db.templates(&mut tx);
        template_db.update_template(&address, update)?;
        tx.commit()?;

        Ok(())
    }

    pub(super) fn fetch_pending_templates(&self) -> Result<Vec<DbTemplate>, TemplateManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let templates = self.global_db.templates(&mut tx).get_pending_templates(1000)?;
        Ok(templates)
    }
}

impl TemplateProvider for TemplateManager {
    type Error = TemplateManagerError;
    type Template = LoadedTemplate;

    fn get_template_module(&self, address: &TemplateAddress) -> Result<Self::Template, Self::Error> {
        if let Some(template) = self.cache.get(address) {
            debug!(target: LOG_TARGET, "CACHE HIT: Template {}", address);
            return Ok(template);
        }

        let template = self.fetch_template(address)?;
        debug!(target: LOG_TARGET, "CACHE MISS: Template {}", address);
        let loaded = match template.executable {
            TemplateExecutable::CompiledWasm(wasm) => {
                let module = WasmModule::from_code(wasm);
                module.load_template()?
            },
            TemplateExecutable::Manifest(_) => return Err(TemplateManagerError::UnsupportedTemplateType),
            TemplateExecutable::Flow(flow_json) => {
                let definition: FlowFunctionDefinition = serde_json::from_str(&flow_json)?;
                let factory = FlowFactory::try_create::<Self>(definition)?;
                LoadedTemplate::Flow(factory)
            },
        };

        self.cache.insert(*address, loaded.clone());

        Ok(loaded)
    }
}
