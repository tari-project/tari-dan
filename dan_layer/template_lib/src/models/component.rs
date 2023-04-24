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
    error::Error,
    fmt::{Display, Formatter},
    str::FromStr,
};

use ciborium::tag::Required;
use serde::{Deserialize, Serialize};

use super::BinaryTag;
use crate::{models::TemplateAddress, prelude::AccessRules, Hash};

const TAG: u64 = BinaryTag::ComponentAddress as u64;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ComponentAddress(Required<ComponentAddressInner, TAG>);

impl ComponentAddress {
    pub fn new(template_address: TemplateAddress, component_id: Hash) -> Self {
        let inner = ComponentAddressInner {
            template_address,
            component_id,
        };
        Self(Required::<ComponentAddressInner, TAG>(inner))
    }

    pub fn template_address(&self) -> &TemplateAddress {
        &self.0 .0.template_address
    }

    pub fn component_id(&self) -> Hash {
        self.0 .0.component_id
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ComponentAddressInner {
    template_address: TemplateAddress,
    component_id: Hash,
}

impl Display for ComponentAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "component_{}_{}", self.0 .0.template_address, self.0 .0.component_id)
    }
}

impl FromStr for ComponentAddress {
    type Err = ComponentAddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let segments: Vec<&str> = s.split('_').collect();

        // the segments should be "component_TEMPLATEHASH_COMPONENTID"
        if segments.len() != 3 {
            return Err(ComponentAddressParseError);
        }

        // parse the prefix
        if segments[0] != "component" {
            return Err(ComponentAddressParseError);
        }

        // parse the template address
        let template_address = TemplateAddress::from_hex(segments[1]).map_err(|_| ComponentAddressParseError)?;

        // parse the component id
        let component_id = Hash::from_hex(segments[2]).map_err(|_| ComponentAddressParseError)?;

        // build and return the comopnent address
        Ok(Self::new(template_address, component_id))
    }
}

#[derive(Debug)]
pub struct ComponentAddressParseError;

impl Error for ComponentAddressParseError {}

impl Display for ComponentAddressParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse component address")
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
