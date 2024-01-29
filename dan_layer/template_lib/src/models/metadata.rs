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

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;
use tari_template_abi::rust::{collections::BTreeMap, fmt::Display};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::BinaryTag;
const TAG: u64 = BinaryTag::Metadata as u64;

/// A collection of user-defined data used to describe other types, for example, non-fungible tokens or events
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Metadata(#[cfg_attr(feature = "ts", ts(type = "string"))] BorTag<BTreeMap<String, String>, TAG>);

impl Metadata {
    pub const fn new() -> Self {
        Self(BorTag::new(BTreeMap::new()))
    }

    pub fn insert<K: Into<String>, V: Into<String>>(&mut self, key: K, value: V) -> &mut Self {
        let key = key.into();
        let value = value.into();
        self.0.insert(key, value);
        self
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.0.get(key)
    }

    pub fn merge(&mut self, other: Metadata) -> &mut Self {
        self.0.extend(other.0.into_inner());
        self
    }
}

impl FromStr for Metadata {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.contains('=') {
            return Err("Invalid metadata string, missing '='".to_string());
        }
        let pairs = s.split(',').map(|pair| {
            let mut split = pair.split('=');
            let key = split.next().ok_or_else(|| "Missing key".to_string())?;
            let value = split.next().ok_or_else(|| "Missing value".to_string())?;
            Ok::<(String, String), String>((key.to_string(), value.to_string()))
        });
        let mut map = BTreeMap::new();
        for pair in pairs {
            let (key, value) = pair?;
            map.insert(key, value);
        }
        Ok(Self(BorTag::new(map)))
    }
}

impl From<BTreeMap<String, String>> for Metadata {
    fn from(value: BTreeMap<String, String>) -> Self {
        Self(BorTag::new(value))
    }
}

impl<K: Into<String>, V: Into<String>, const N: usize> From<[(K, V); N]> for Metadata {
    fn from(value: [(K, V); N]) -> Self {
        Self(BorTag::new(BTreeMap::from(value.map(|(k, v)| (k.into(), v.into())))))
    }
}

impl IntoIterator for Metadata {
    type IntoIter = std::collections::btree_map::IntoIter<String, String>;
    type Item = (String, String);

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_inner().into_iter()
    }
}

impl Default for Metadata {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for Metadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Metadata: ")?;
        for (key, value) in &*self.0 {
            write!(f, "key = {}, value = {} ", key, value)?;
        }
        Ok(())
    }
}
