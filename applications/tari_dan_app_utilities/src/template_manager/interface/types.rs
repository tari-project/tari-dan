//  Copyright 2023. The Tari Project
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

use tari_dan_storage::global::{DbTemplate, DbTemplateType};
use tari_template_lib::models::TemplateAddress;
use tari_validator_node_client::types::TemplateAbi;
use tokio::sync::oneshot;

use super::{TemplateManagerError, TemplateRegistration};

#[derive(Debug, Clone)]
pub struct TemplateMetadata {
    pub name: String,
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
            name: reg.template_name,
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
            name: record.template_name,
            address: (*record.template_address).into(),
            url: record.url,
            binary_sha: vec![],
            height: record.height,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TemplateExecutable {
    CompiledWasm(Vec<u8>),
    Manifest(String),
    Flow(String),
}

#[derive(Debug, Clone)]
pub struct Template {
    pub metadata: TemplateMetadata,
    pub executable: TemplateExecutable,
}

// we encapsulate the db row format to not expose it to the caller
impl From<DbTemplate> for Template {
    fn from(record: DbTemplate) -> Self {
        Template {
            metadata: TemplateMetadata {
                name: record.template_name,
                // TODO: this will change when common engine types are moved around
                address: (*record.template_address).into(),
                url: record.url,
                // TODO: add field to db
                binary_sha: vec![],
                height: record.height,
            },
            executable: match record.template_type {
                DbTemplateType::Wasm => TemplateExecutable::CompiledWasm(record.compiled_code.unwrap()),
                DbTemplateType::Flow => TemplateExecutable::Flow(record.flow_json.unwrap()),
                DbTemplateType::Manifest => TemplateExecutable::Manifest(record.manifest.unwrap()),
            },
        }
    }
}

#[derive(Debug)]
pub enum TemplateManagerRequest {
    AddTemplate {
        template: Box<TemplateRegistration>,
        reply: oneshot::Sender<Result<(), TemplateManagerError>>,
    },
    GetTemplate {
        address: TemplateAddress,
        reply: oneshot::Sender<Result<Template, TemplateManagerError>>,
    },
    GetTemplates {
        limit: usize,
        reply: oneshot::Sender<Result<Vec<TemplateMetadata>, TemplateManagerError>>,
    },
    LoadTemplateAbi {
        address: TemplateAddress,
        reply: oneshot::Sender<Result<TemplateAbi, TemplateManagerError>>,
    },
}
