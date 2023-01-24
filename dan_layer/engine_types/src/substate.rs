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

use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_bor::{borsh, decode, encode, Decode, Encode};
use tari_template_lib::{
    models::{ComponentAddress, ComponentHeader, NonFungible, NonFungibleId, ResourceAddress, VaultId},
    Hash,
};

use crate::{resource::Resource, vault::Vault};

#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize)]
pub struct Substate {
    address: SubstateAddress,
    substate: SubstateValue,
    version: u32,
}

impl Substate {
    pub fn new<T: Into<SubstateValue>>(address: SubstateAddress, version: u32, substate: T) -> Self {
        Self {
            address,
            substate: substate.into(),
            version,
        }
    }

    pub fn substate_address(&self) -> &SubstateAddress {
        &self.address
    }

    pub fn substate_value(&self) -> &SubstateValue {
        &self.substate
    }

    pub fn into_substate_value(self) -> SubstateValue {
        self.substate
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> std::io::Result<Self> {
        decode(bytes)
    }
}

/// Base object address, version tuples
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode, Serialize, Deserialize)]
pub enum SubstateAddress {
    Component(ComponentAddress),
    Resource(ResourceAddress),
    Vault(VaultId),
    NonFungible(ResourceAddress, NonFungibleId),
}

impl SubstateAddress {
    pub fn as_component_address(&self) -> Option<ComponentAddress> {
        match self {
            Self::Component(addr) => Some(*addr),
            _ => None,
        }
    }

    pub fn as_resource_address(&self) -> Option<ResourceAddress> {
        match self {
            Self::Resource(address) => Some(*address),
            _ => None,
        }
    }

    pub fn hash(&self) -> &Hash {
        match self {
            SubstateAddress::Component(address) => address.hash(),
            SubstateAddress::Resource(address) => address.hash(),
            SubstateAddress::Vault(id) => id.hash(),
            SubstateAddress::NonFungible(_, id) => id.hash(),
        }
    }

    // TODO: look at using BECH32 standard
    pub fn to_address_string(&self) -> String {
        match self {
            SubstateAddress::Component(addr) => addr.to_string(),
            SubstateAddress::Resource(addr) => addr.to_string(),
            SubstateAddress::Vault(addr) => addr.to_string(),
            SubstateAddress::NonFungible(resource_addr, addr) => format!("{} {}", resource_addr, addr),
        }
    }

    pub fn as_non_fungible_address(&self) -> Option<(ResourceAddress, NonFungibleId)> {
        match self {
            SubstateAddress::NonFungible(resource_address, nft_id) => Some((*resource_address, *nft_id)),
            _ => None,
        }
    }

    pub fn is_resource(&self) -> bool {
        matches!(self, Self::Resource(_))
    }

    pub fn is_component(&self) -> bool {
        matches!(self, Self::Component(_))
    }

    pub fn is_vault(&self) -> bool {
        matches!(self, Self::Vault(_))
    }

    pub fn is_non_fungible(&self) -> bool {
        matches!(self, Self::NonFungible(_, _))
    }
}

impl From<ComponentAddress> for SubstateAddress {
    fn from(address: ComponentAddress) -> Self {
        Self::Component(address)
    }
}

impl From<ResourceAddress> for SubstateAddress {
    fn from(address: ResourceAddress) -> Self {
        Self::Resource(address)
    }
}

impl From<VaultId> for SubstateAddress {
    fn from(address: VaultId) -> Self {
        Self::Vault(address)
    }
}

impl Display for SubstateAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_address_string())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid substate address '{0}'")]
pub struct InvalidSubstateAddressFormat(String);

impl FromStr for SubstateAddress {
    type Err = InvalidSubstateAddressFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once('_') {
            Some(("component", addr)) => {
                let addr = ComponentAddress::from_hex(addr).map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                Ok(SubstateAddress::Component(addr))
            },
            Some(("resource", addr)) => {
                let addr = ResourceAddress::from_hex(addr).map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                Ok(SubstateAddress::Resource(addr))
            },
            Some(("vault", addr)) => {
                let id = VaultId::from_hex(addr).map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                Ok(SubstateAddress::Vault(id))
            },
            Some(("nft", addr)) => {
                let id = NonFungibleId::from_hex(addr).map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                // TODO: We need to add more structure to objects with child/parent relationships.
                //       Setting the resource to 000.. has no effect because the system knows about this substate and
                //       includes the correct resource in the final address, however this may change as the NFT ID
                //       should be qualified by the parent resource.
                Ok(SubstateAddress::NonFungible(ResourceAddress::new(Hash::default()), id))
            },
            Some(_) | None => Err(InvalidSubstateAddressFormat(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize)]
pub enum SubstateValue {
    Component(ComponentHeader),
    Resource(Resource),
    Vault(Vault),
    NonFungible(NonFungible),
}

impl SubstateValue {
    pub fn into_component(self) -> Option<ComponentHeader> {
        match self {
            SubstateValue::Component(component) => Some(component),
            _ => None,
        }
    }

    pub fn component(&self) -> Option<&ComponentHeader> {
        match self {
            SubstateValue::Component(component) => Some(component),
            _ => None,
        }
    }

    pub fn component_mut(&mut self) -> Option<&mut ComponentHeader> {
        match self {
            SubstateValue::Component(component) => Some(component),
            _ => None,
        }
    }

    pub fn into_vault(self) -> Option<Vault> {
        match self {
            SubstateValue::Vault(vault) => Some(vault),
            _ => None,
        }
    }

    pub fn into_resource(self) -> Option<Resource> {
        match self {
            SubstateValue::Resource(resource) => Some(resource),
            _ => None,
        }
    }

    pub fn resource_address(&self) -> Option<ResourceAddress> {
        match self {
            SubstateValue::Resource(resource) => Some(*resource.resource_address()),
            SubstateValue::Vault(vault) => Some(*vault.resource_address()),
            _ => None,
        }
    }

    pub fn into_non_fungible(self) -> Option<NonFungible> {
        match self {
            SubstateValue::NonFungible(nft) => Some(nft),
            _ => None,
        }
    }
}

impl From<ComponentHeader> for SubstateValue {
    fn from(component: ComponentHeader) -> Self {
        Self::Component(component)
    }
}

impl From<Resource> for SubstateValue {
    fn from(resource: Resource) -> Self {
        Self::Resource(resource)
    }
}

impl From<Vault> for SubstateValue {
    fn from(vault: Vault) -> Self {
        Self::Vault(vault)
    }
}

impl From<NonFungible> for SubstateValue {
    fn from(token: NonFungible) -> Self {
        Self::NonFungible(token)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubstateDiff {
    up_substates: Vec<(SubstateAddress, Substate)>,
    down_substates: Vec<(SubstateAddress, u32)>,
}

impl SubstateDiff {
    pub fn new() -> Self {
        Self {
            up_substates: Vec::new(),
            down_substates: Vec::new(),
        }
    }

    pub fn up(&mut self, address: SubstateAddress, value: Substate) {
        self.up_substates.push((address, value));
    }

    pub fn down(&mut self, address: SubstateAddress, version: u32) {
        self.down_substates.push((address, version));
    }

    pub fn up_iter(&self) -> impl Iterator<Item = &(SubstateAddress, Substate)> + '_ {
        self.up_substates.iter()
    }

    pub fn into_up_iter(self) -> impl Iterator<Item = (SubstateAddress, Substate)> {
        self.up_substates.into_iter()
    }

    pub fn down_iter(&self) -> impl Iterator<Item = &(SubstateAddress, u32)> + '_ {
        self.down_substates.iter()
    }

    pub fn len(&self) -> usize {
        self.up_substates.len() + self.down_substates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
