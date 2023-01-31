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

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use tari_engine_types::TemplateAddress;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateConfig {
    max_cache_size_bytes: u64,
    debug_replacements: Vec<String>,
}

impl Default for TemplateConfig {
    fn default() -> Self {
        Self {
            max_cache_size_bytes: 200 * 1024 * 1024,
            debug_replacements: Vec::new(),
        }
    }
}

impl TemplateConfig {
    pub fn debug_replacements(&self) -> HashMap<TemplateAddress, PathBuf> {
        let mut result = HashMap::new();
        for row in &self.debug_replacements {
            let (template_address, path) = row.split_once('=').expect("USAGE: [templateaddress]=[path]");
            let template_address = TemplateAddress::from_hex(template_address).expect("Not a valid template address");
            result.insert(template_address, path.into());
        }
        result
    }

    pub fn max_cache_size_bytes(&self) -> u64 {
        self.max_cache_size_bytes
    }
}
