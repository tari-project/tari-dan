//  Copyright 2022. The Tari Project
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
use std::collections::HashMap;

use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_template_abi::TemplateDef;
use tari_template_lib::models::TemplateAddress;
use thiserror::Error;

use crate::packager::template::LoadedTemplate;

#[derive(Debug, Clone)]
pub struct Package {
    templates: HashMap<TemplateAddress, LoadedTemplate>,
}

impl Package {
    pub fn builder() -> PackageBuilder {
        PackageBuilder::new()
    }

    pub fn get_template_by_address(&self, addr: &TemplateAddress) -> Option<&LoadedTemplate> {
        self.templates.get(addr)
    }

    pub fn get_template_defs(&self) -> HashMap<TemplateAddress, TemplateDef> {
        self.templates
            .iter()
            .map(|(addr, template)| (*addr, template.template_def().clone()))
            .collect()
    }

    pub fn total_code_byte_size(&self) -> usize {
        self.templates.values().map(|t| t.code_size()).sum()
    }
}

#[derive(Debug, Clone, Default)]
pub struct PackageBuilder {
    templates: HashMap<TemplateAddress, LoadedTemplate>,
}

impl PackageBuilder {
    pub fn new() -> Self {
        Self {
            templates: HashMap::new(),
        }
    }

    /// Adds a template to the package. The address given is typically the hash of the template registration UTXO.
    pub fn add_template(&mut self, address: TemplateAddress, template: LoadedTemplate) -> &mut Self {
        self.templates.insert(address, template);
        self
    }

    pub fn build(&mut self) -> Package {
        Package {
            templates: self.templates.drain().collect(),
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum PackageError {
    #[error("Template not found")]
    TemplateNotFound,
}

impl TemplateProvider for Package {
    type Error = PackageError;
    type Template = LoadedTemplate;

    fn get_template_module(&self, id: &tari_engine_types::TemplateAddress) -> Result<Self::Template, Self::Error> {
        self.templates.get(id).cloned().ok_or(PackageError::TemplateNotFound)
    }
}
