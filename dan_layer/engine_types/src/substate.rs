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
use tari_bor::{borsh, decode, decode_exact, encode, Decode, Encode};
use tari_common_types::types::Commitment;
use tari_template_lib::{
    models::{
        Address,
        AddressListId,
        AddressListItemAddress,
        ComponentAddress,
        ComponentHeader,
        LayerOneCommitmentAddress,
        NonFungibleAddress,
        NonFungibleId,
        ResourceAddress,
        VaultId,
    },
    Hash,
};
use tari_utilities::ByteArray;

use crate::{
    address_list::{AddressList, AddressListItem},
    hashing::{hasher, EngineHashDomainLabel},
    non_fungible::NonFungibleContainer,
    resource::Resource,
    vault::Vault,
};

#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize)]
pub struct Substate {
    substate: SubstateValue,
    version: u32,
}

impl Substate {
    pub fn new<T: Into<SubstateValue>>(version: u32, substate: T) -> Self {
        Self {
            substate: substate.into(),
            version,
        }
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Encode, Decode, Serialize, Deserialize)]
pub enum SubstateAddress {
    Component(ComponentAddress),
    Resource(ResourceAddress),
    Vault(VaultId),
    LayerOneCommitment(LayerOneCommitmentAddress),
    NonFungible(NonFungibleAddress),
    AddressList(AddressListId),
    AddressListItem(AddressListItemAddress),
}

impl SubstateAddress {
    pub fn as_component_address(&self) -> Option<ComponentAddress> {
        match self {
            Self::Component(addr) => Some(*addr),
            _ => None,
        }
    }

    pub fn as_vault_id(&self) -> Option<VaultId> {
        match self {
            Self::Vault(id) => Some(*id),
            _ => None,
        }
    }

    pub fn as_resource_address(&self) -> Option<ResourceAddress> {
        match self {
            Self::Resource(address) => Some(*address),
            _ => None,
        }
    }

    pub fn to_canonical_hash(&self) -> Hash {
        match self {
            SubstateAddress::Component(address) => *address.hash(),
            SubstateAddress::Resource(address) => *address.hash(),
            SubstateAddress::Vault(id) => *id.hash(),
            SubstateAddress::LayerOneCommitment(address) => *address.hash(),
            SubstateAddress::NonFungible(address) => hasher(EngineHashDomainLabel::NonFungibleId)
                .chain(address.resource_address().hash())
                .chain(address.id())
                .result(),
            SubstateAddress::AddressList(id) => *id.hash(),
            SubstateAddress::AddressListItem(address) => hasher(EngineHashDomainLabel::AddressListItemAddress)
                .chain(address.list_id().hash())
                .chain(&address.index())
                .result(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> std::io::Result<Self> {
        decode_exact(bytes)
    }

    // TODO: look at using BECH32 standard
    pub fn to_address_string(&self) -> String {
        self.to_string()
    }

    pub fn as_non_fungible_address(&self) -> Option<&NonFungibleAddress> {
        match self {
            SubstateAddress::NonFungible(addr) => Some(addr),
            _ => None,
        }
    }

    pub fn as_address_list_id(&self) -> Option<&AddressListId> {
        match self {
            SubstateAddress::AddressList(id) => Some(id),
            _ => None,
        }
    }

    pub fn as_address_list_item_address(&self) -> Option<&AddressListItemAddress> {
        match self {
            SubstateAddress::AddressListItem(addr) => Some(addr),
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
        matches!(self, Self::NonFungible(_))
    }

    pub fn is_address_list(&self) -> bool {
        matches!(self, Self::AddressList(_))
    }

    pub fn is_address_list_item(&self) -> bool {
        matches!(self, Self::AddressListItem(_))
    }

    pub fn is_layer1_commitment(&self) -> bool {
        matches!(self, Self::LayerOneCommitment(_))
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

impl From<NonFungibleAddress> for SubstateAddress {
    fn from(address: NonFungibleAddress) -> Self {
        Self::NonFungible(address)
    }
}

impl From<AddressListId> for SubstateAddress {
    fn from(address: AddressListId) -> Self {
        Self::AddressList(address)
    }
}

impl From<AddressListItemAddress> for SubstateAddress {
    fn from(address: AddressListItemAddress) -> Self {
        Self::AddressListItem(address)
    }
}

impl From<Address> for SubstateAddress {
    fn from(address: Address) -> Self {
        match address {
            Address::Component(addr) => Self::from(addr),
            Address::Resource(addr) => Self::from(addr),
            Address::Vault(id) => Self::from(id),
            Address::NonFungible(addr) => Self::from(addr),
            Address::AddressList(id) => Self::from(id),
        }
    }
}

impl Display for SubstateAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SubstateAddress::Component(addr) => write!(f, "{}", addr),
            SubstateAddress::Resource(addr) => write!(f, "{}", addr),
            SubstateAddress::Vault(addr) => write!(f, "{}", addr),
            SubstateAddress::NonFungible(addr) => write!(f, "{}", addr),
            SubstateAddress::AddressList(addr) => write!(f, "{}", addr),
            SubstateAddress::AddressListItem(addr) => write!(f, "{}", addr),
            SubstateAddress::LayerOneCommitment(commitment_address) => write!(f, "{}", commitment_address),
        }
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
                match addr.split_once(' ') {
                    // resource_xxxx nft_xxxxx
                    Some((resource_str, addr)) => match addr.split_once('_') {
                        Some(("nft", addr)) => {
                            let resource_addr = ResourceAddress::from_hex(resource_str)
                                .map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                            let id = NonFungibleId::try_from_canonical_string(addr)
                                .map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                            Ok(SubstateAddress::NonFungible(NonFungibleAddress::new(resource_addr, id)))
                        },
                        _ => Err(InvalidSubstateAddressFormat(s.to_string())),
                    },
                    // resource_xxxx
                    None => {
                        let addr =
                            ResourceAddress::from_hex(addr).map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                        Ok(SubstateAddress::Resource(addr))
                    },
                }
            },
            Some(("vault", addr)) => {
                let id = VaultId::from_hex(addr).map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                Ok(SubstateAddress::Vault(id))
            },
            Some(("addresslist", addr)) => {
                match addr.split_once(' ') {
                    // addresslist_xxxx item:xxxxx
                    Some((list_str, item)) => match item.split_once(':') {
                        Some(("item", index_str)) => {
                            let list_id = AddressListId::from_hex(list_str)
                                .map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                            let index =
                                u64::from_str(index_str).map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                            Ok(SubstateAddress::AddressListItem(AddressListItemAddress::new(
                                list_id, index,
                            )))
                        },
                        _ => Err(InvalidSubstateAddressFormat(s.to_string())),
                    },
                    // addresslist_xxxx
                    None => {
                        let list_id =
                            AddressListId::from_hex(addr).map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                        Ok(SubstateAddress::AddressList(list_id))
                    },
                }
            },
            Some(("commitment", addr)) => {
                let commitment_address = LayerOneCommitmentAddress::from_hex(addr)
                    .map_err(|_| InvalidSubstateAddressFormat(s.to_string()))?;
                Ok(SubstateAddress::LayerOneCommitment(commitment_address))
            },
            Some(_) | None => Err(InvalidSubstateAddressFormat(s.to_string())),
        }
    }
}

macro_rules! impl_partial_eq {
    ($typ:ty, $variant:ident) => {
        impl PartialEq<$typ> for SubstateAddress {
            fn eq(&self, other: &$typ) -> bool {
                match self {
                    SubstateAddress::$variant(addr) => addr == other,
                    _ => false,
                }
            }
        }
        impl PartialEq<SubstateAddress> for $typ {
            fn eq(&self, other: &SubstateAddress) -> bool {
                other == self
            }
        }
    };
}
impl_partial_eq!(ComponentAddress, Component);
impl_partial_eq!(ResourceAddress, Resource);
impl_partial_eq!(VaultId, Vault);
impl_partial_eq!(LayerOneCommitmentAddress, LayerOneCommitment);
impl_partial_eq!(NonFungibleAddress, NonFungible);

#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize)]
pub enum SubstateValue {
    Component(ComponentHeader),
    Resource(Resource),
    Vault(Vault),
    NonFungible(NonFungibleContainer),
    AddressList(AddressList),
    AddressListItem(AddressListItem),
    LayerOneCommitment(Vec<u8>),
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

    pub fn non_fungible(&self) -> Option<&NonFungibleContainer> {
        match self {
            SubstateValue::NonFungible(nft) => Some(nft),
            _ => None,
        }
    }

    pub fn into_non_fungible(self) -> Option<NonFungibleContainer> {
        match self {
            SubstateValue::NonFungible(nft) => Some(nft),
            _ => None,
        }
    }

    pub fn address_list(&self) -> Option<&AddressList> {
        match self {
            SubstateValue::AddressList(list) => Some(list),
            _ => None,
        }
    }

    pub fn into_address_list(self) -> Option<AddressList> {
        match self {
            SubstateValue::AddressList(list) => Some(list),
            _ => None,
        }
    }

    pub fn address_list_item(&self) -> Option<&AddressListItem> {
        match self {
            SubstateValue::AddressListItem(item) => Some(item),
            _ => None,
        }
    }

    pub fn into_address_list_item(self) -> Option<AddressListItem> {
        match self {
            SubstateValue::AddressListItem(item) => Some(item),
            _ => None,
        }
    }

    pub fn into_layer_one_commitment(self) -> Option<Commitment> {
        match self {
            SubstateValue::LayerOneCommitment(commitment) => Some(
                Commitment::from_bytes(&commitment)
                    .expect("Should never fail to parse commitment, because it's saved from the db"),
            ),
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

impl From<NonFungibleContainer> for SubstateValue {
    fn from(token: NonFungibleContainer) -> Self {
        Self::NonFungible(token)
    }
}

impl From<AddressList> for SubstateValue {
    fn from(list: AddressList) -> Self {
        Self::AddressList(list)
    }
}

impl From<AddressListItem> for SubstateValue {
    fn from(item: AddressListItem) -> Self {
        Self::AddressListItem(item)
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

#[cfg(test)]
mod tests {
    use super::*;

    mod substate_address_parse {
        use super::*;

        #[test]
        fn it_parses_valid_substate_addresses() {
            SubstateAddress::from_str("component_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab55ff1ff64")
                .unwrap()
                .as_component_address()
                .unwrap();
            SubstateAddress::from_str("vault_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab55ff1ff64")
                .unwrap()
                .as_vault_id()
                .unwrap();
            SubstateAddress::from_str("resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab55ff1ff64")
                .unwrap()
                .as_resource_address()
                .unwrap();
            SubstateAddress::from_str(
                "resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab55ff1ff64 nft_str:SpecialNft",
            )
            .unwrap()
            .as_non_fungible_address()
            .unwrap();
            SubstateAddress::from_str("addresslist_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab55ff1ff64")
                .unwrap()
                .as_address_list_id()
                .unwrap();
            SubstateAddress::from_str(
                "addresslist_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab55ff1ff64 item:0",
            )
            .unwrap()
            .as_address_list_item_address()
            .unwrap();

            SubstateAddress::from_str(
                "resource_a7cf4fd18ada7f367b1c102a9c158abc3754491665033231c5eb907fa14dfe2b \
                 nft_uuid:7f19c3fe5fa13ff66a0d379fe5f9e3508acbd338db6bedd7350d8d565b2c5d32",
            )
            .unwrap()
            .as_non_fungible_address()
            .unwrap();
        }
    }
}
