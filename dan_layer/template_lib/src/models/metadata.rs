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

use serde::{Deserialize, Serialize};
use std::fmt::Display;
use tari_bor::BorTag;
use tari_template_abi::rust::collections::BTreeMap;

use super::BinaryTag;
const TAG: u64 = BinaryTag::Metadata as u64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Metadata(BorTag<BTreeMap<String, String>, TAG>);

impl Metadata {
    pub fn new() -> Self {
        Self(BorTag::new(BTreeMap::new()))
    }

    pub fn insert<K: Into<String>, V: Into<String>>(&mut self, key: K, value: V) -> &mut Self {
        let key = key.into();
        let value = value.into();
        self.0.insert(key, value);
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|v| v.as_str())
    }
}

impl From<BTreeMap<String, String>> for Metadata {
    fn from(value: BTreeMap<String, String>) -> Self {
        Self(BorTag::new(value))
    }
}

impl Default for Metadata {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for Metadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = vec![];
        for (key, value) in self.0.iter() {
            s.push(format!("key = {}, value = {}", key, value))
        }
        write!(f, "Metadata: {:?}", s)
    }
}
