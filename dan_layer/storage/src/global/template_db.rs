//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::str::FromStr;

use chrono::NaiveDateTime;
use tari_common_types::types::FixedHash;

use crate::global::GlobalDbAdapter;

pub struct TemplateDb<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
}

impl<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> TemplateDb<'a, 'tx, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>) -> Self {
        Self { backend, tx }
    }

    pub fn get_template(&mut self, key: &[u8]) -> Result<Option<DbTemplate>, TGlobalDbAdapter::Error> {
        self.backend.get_template(self.tx, key)
    }

    pub fn get_templates(&mut self, limit: usize) -> Result<Vec<DbTemplate>, TGlobalDbAdapter::Error> {
        self.backend.get_templates(self.tx, limit)
    }

    pub fn insert_template(&mut self, template: DbTemplate) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.insert_template(self.tx, template)
    }

    pub fn update_template(&mut self, key: &[u8], update: DbTemplateUpdate) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.update_template(self.tx, key, update)
    }

    pub fn template_exists(&mut self, key: &[u8]) -> Result<bool, TGlobalDbAdapter::Error> {
        self.backend.template_exists(self.tx, key)
    }
}

#[derive(Debug, Clone)]
pub struct DbTemplate {
    pub template_name: String,
    // TODO: change to TemplateAddress type
    pub template_address: FixedHash,
    pub url: String,
    pub height: u64,
    pub template_type: DbTemplateType,
    pub compiled_code: Option<Vec<u8>>,
    pub flow_json: Option<String>,
    pub manifest: Option<String>,
    pub status: TemplateStatus,
    pub added_at: NaiveDateTime,
}

#[derive(Debug, Clone, Default)]
pub struct DbTemplateUpdate {
    pub compiled_code: Option<Vec<u8>>,
    pub flow_json: Option<String>,
    pub manifest: Option<String>,
    pub status: Option<TemplateStatus>,
}

#[derive(Debug, Clone)]
pub enum DbTemplateType {
    Wasm,
    Flow,
    Manifest,
}

impl FromStr for DbTemplateType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_lowercase();
        match normalized.as_str() {
            "wasm" => Ok(DbTemplateType::Wasm),
            "flow" => Ok(DbTemplateType::Flow),
            "manifest" => Ok(DbTemplateType::Manifest),
            _ => Err(()),
        }
    }
}

impl DbTemplateType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DbTemplateType::Wasm => "Wasm",
            DbTemplateType::Flow => "Flow",
            DbTemplateType::Manifest => "Manifest",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TemplateStatus {
    /// Template has been registered but has not completed
    #[default]
    New,
    /// Template download has begun but not completed
    Pending,
    /// Template download has completed
    Active,
    /// Template download completed but was invalid
    Invalid,
    /// Template download failed
    DownloadFailed,
    /// Template has been deprecated
    Deprecated,
}

impl FromStr for TemplateStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_lowercase();
        match normalized.as_str() {
            "new" => Ok(TemplateStatus::New),
            "pending" => Ok(TemplateStatus::Pending),
            "active" => Ok(TemplateStatus::Active),
            "invalid" => Ok(TemplateStatus::Invalid),
            "downloadfailed" => Ok(TemplateStatus::DownloadFailed),
            "deprecated" => Ok(TemplateStatus::Deprecated),
            _ => Err(()),
        }
    }
}

impl TemplateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TemplateStatus::New => "New",
            TemplateStatus::Pending => "Pending",
            TemplateStatus::Active => "Active",
            TemplateStatus::Invalid => "Invalid",
            TemplateStatus::DownloadFailed => "DownloadFailed",
            TemplateStatus::Deprecated => "Deprecated",
        }
    }
}
