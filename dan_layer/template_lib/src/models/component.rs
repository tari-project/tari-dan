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

use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

use crate::{hash::HashParseError, models::TemplateAddress, prelude::AccessRules, Hash};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ComponentAddress(Hash);

impl ComponentAddress {
    pub fn new(address: Hash) -> Self {
        Self(address)
    }

    pub fn hash(&self) -> &Hash {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_hex(hex: &str) -> Result<Self, HashParseError> {
        let hash = Hash::from_hex(hex)?;
        Ok(Self::new(hash))
    }
}

impl FromStr for ComponentAddress {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = if let Some(stripped) = s.strip_prefix("component_") {
            stripped
        } else {
            s
        };
        let hash = Hash::from_hex(s)?;
        Ok(Self::new(hash))
    }
}

impl<T: Into<Hash>> From<T> for ComponentAddress {
    fn from(address: T) -> Self {
        Self::new(address.into())
    }
}

impl TryFrom<Vec<u8>> for ComponentAddress {
    type Error = HashParseError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let hash = Hash::try_from(value)?;
        Ok(Self::new(hash))
    }
}

impl Display for ComponentAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "component_{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHeader {
    pub template_address: TemplateAddress,
    pub module_name: String,
    // TODO: Access rules should be a separate substate?
    pub access_rules: AccessRules,
    // TODO: Split the state from the header
    pub state: ComponentBody,
}

impl ComponentHeader {
    pub fn into_component(self) -> ComponentBody {
        self.state
    }

    pub fn state(&self) -> &[u8] {
        &self.state.state
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentBody {
    pub state: Vec<u8>,
}

impl ComponentBody {
    pub fn set(&mut self, state: Vec<u8>) -> &mut Self {
        self.state = state;
        self
    }
}
