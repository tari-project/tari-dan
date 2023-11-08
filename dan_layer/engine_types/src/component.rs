//   Copyright 2023. The Tari Project
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
use tari_template_lib::{
    auth::{ComponentAccessRules, OwnerRule, Ownership},
    crypto::RistrettoPublicKeyBytes,
    models::TemplateAddress,
    prelude::ComponentAddress,
    Hash,
};

use crate::{
    hashing::{hasher32, EngineHashDomainLabel},
    serde_with,
};

pub fn new_component_address_from_parts(template_address: &TemplateAddress, component_id: &Hash) -> ComponentAddress {
    let address = hasher32(EngineHashDomainLabel::ComponentAddress)
        .chain(template_address)
        .chain(component_id)
        .result();
    ComponentAddress::new(address)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHeader {
    #[serde(with = "serde_with::hex")]
    pub template_address: TemplateAddress,
    pub module_name: String,
    pub owner_key: RistrettoPublicKeyBytes,
    pub owner_rule: OwnerRule,
    pub access_rules: ComponentAccessRules,
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

    pub fn as_ownership(&self) -> Ownership<'_> {
        Ownership {
            owner_key: &self.owner_key,
            owner_rule: &self.owner_rule,
        }
    }

    pub fn access_rules(&self) -> &ComponentAccessRules {
        &self.access_rules
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentBody {
    #[serde(with = "serde_with::hex")]
    pub state: Vec<u8>,
}

impl ComponentBody {
    pub fn set(&mut self, state: Vec<u8>) -> &mut Self {
        self.state = state;
        self
    }
}
