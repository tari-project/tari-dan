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
use tari_common_types::types::PublicKey;
use tari_template_lib::{
    auth::{ComponentAccessRules, OwnerRule, Ownership},
    crypto::RistrettoPublicKeyBytes,
    models::{ComponentKey, EntityId, ObjectKey, TemplateAddress},
    prelude::ComponentAddress,
    Hash,
};
use tari_utilities::ByteArray;
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    hashing::{hasher32, EngineHashDomainLabel},
    indexed_value::{IndexedValueError, IndexedWellKnownTypes},
    serde_with,
    substate::SubstateId,
};

/// Derives a component address.
///
/// This can be used to derive the component address from a public key if the component sets OwnerRule::OwnedBySigner or
/// OwnerRule::ByPublicKey
pub fn new_account_address_from_parts(template_address: &TemplateAddress, public_key: &PublicKey) -> ComponentAddress {
    let address = hasher32(EngineHashDomainLabel::ComponentAddress)
        .chain(template_address)
        .chain(public_key)
        .result();
    let key = ObjectKey::new(
        EntityId::from_array(Hash::try_from(public_key.as_bytes()).unwrap().leading_bytes()),
        ComponentKey::new(address.trailing_bytes()),
    );
    ComponentAddress::new(key)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ComponentHeader {
    #[serde(with = "serde_with::hex")]
    #[cfg_attr(feature = "ts", ts(type = "Uint8Array"))]
    pub template_address: TemplateAddress,
    pub module_name: String,
    #[serde(with = "serde_with::hex::option")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub owner_key: Option<RistrettoPublicKeyBytes>,
    pub owner_rule: OwnerRule,
    pub access_rules: ComponentAccessRules,
    pub entity_id: EntityId,
    // TODO: Split the state from the header
    pub body: ComponentBody,
}

impl ComponentHeader {
    pub fn into_component(self) -> ComponentBody {
        self.body
    }

    pub fn state(&self) -> &tari_bor::Value {
        &self.body.state
    }

    pub fn into_state(self) -> tari_bor::Value {
        self.body.state
    }

    pub fn as_ownership(&self) -> Ownership<'_> {
        Ownership {
            owner_key: self.owner_key.as_ref(),
            owner_rule: &self.owner_rule,
        }
    }

    pub fn access_rules(&self) -> &ComponentAccessRules {
        &self.access_rules
    }

    pub fn set_access_rules(&mut self, access_rules: ComponentAccessRules) -> &mut Self {
        self.access_rules = access_rules;
        self
    }

    pub fn contains_substate(&self, address: &SubstateId) -> Result<bool, IndexedValueError> {
        let found = IndexedWellKnownTypes::value_contains_substate(self.state(), address)?;
        Ok(found)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ComponentBody {
    #[serde(with = "serde_with::cbor_value")]
    #[cfg_attr(feature = "ts", ts(type = "any"))]
    pub state: tari_bor::Value,
}

impl ComponentBody {
    pub fn set(&mut self, state: tari_bor::Value) -> &mut Self {
        self.state = state;
        self
    }
}
