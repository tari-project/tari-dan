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
use tari_bor::{decode, decode_exact, encode, BorError};
use tari_common_types::types::FixedHash;
use tari_template_lib::{
    models::{
        ComponentAddress,
        NonFungibleAddress,
        NonFungibleIndexAddress,
        ObjectKey,
        ResourceAddress,
        UnclaimedConfidentialOutputAddress,
        VaultId,
    },
    prelude::PUBLIC_IDENTITY_RESOURCE_ADDRESS,
    Hash,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    component::ComponentHeader,
    confidential::UnclaimedConfidentialOutput,
    fee_claim::{FeeClaim, FeeClaimAddress},
    hashing::{hasher32, substate_value_hasher32, EngineHashDomainLabel},
    non_fungible::NonFungibleContainer,
    non_fungible_index::NonFungibleIndex,
    resource::Resource,
    serde_with,
    transaction_receipt::{TransactionReceipt, TransactionReceiptAddress},
    vault::Vault,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
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

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BorError> {
        decode(bytes)
    }

    pub fn to_value_hash(&self) -> FixedHash {
        hash_substate(self.substate_value(), self.version)
    }
}

pub fn hash_substate(substate: &SubstateValue, version: u32) -> FixedHash {
    substate_value_hasher32()
        .chain(substate)
        .chain(&version)
        .result()
        .into_array()
        .into()
}

/// Base object address, version tuples
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum SubstateId {
    Component(#[serde(with = "serde_with::string")] ComponentAddress),
    Resource(#[serde(with = "serde_with::string")] ResourceAddress),
    Vault(#[serde(with = "serde_with::string")] VaultId),
    UnclaimedConfidentialOutput(#[serde(with = "serde_with::string")] UnclaimedConfidentialOutputAddress),
    NonFungible(#[serde(with = "serde_with::string")] NonFungibleAddress),
    NonFungibleIndex(#[serde(with = "serde_with::string")] NonFungibleIndexAddress),
    TransactionReceipt(#[serde(with = "serde_with::string")] TransactionReceiptAddress),
    FeeClaim(#[serde(with = "serde_with::string")] FeeClaimAddress),
}

impl SubstateId {
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

    pub fn as_unclaimed_confidential_output_address(&self) -> Option<UnclaimedConfidentialOutputAddress> {
        match self {
            Self::UnclaimedConfidentialOutput(address) => Some(*address),
            _ => None,
        }
    }

    /// Returns true for any substate that has is "versionable" i.e. can have a version > 0, otherwise false.
    pub fn is_versioned(&self) -> bool {
        match self {
            SubstateId::Component(_) |
            SubstateId::Resource(_) |
            SubstateId::Vault(_) |
            SubstateId::NonFungibleIndex(_) |
            SubstateId::NonFungible(_) => true,
            SubstateId::UnclaimedConfidentialOutput(_) |
            SubstateId::TransactionReceipt(_) |
            SubstateId::FeeClaim(_) => false,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BorError> {
        decode_exact(bytes)
    }

    pub fn to_object_key(&self) -> ObjectKey {
        match self {
            SubstateId::Component(addr) => *addr.as_object_key(),
            SubstateId::Resource(addr) => *addr.as_object_key(),
            SubstateId::Vault(addr) => *addr.as_object_key(),
            SubstateId::NonFungible(addr) => {
                let key = hasher32(EngineHashDomainLabel::NonFungibleId)
                    .chain(addr.resource_address())
                    .chain(addr.id())
                    .result()
                    .trailing_bytes()
                    .into();

                ObjectKey::new(addr.resource_address().as_entity_id(), key)
            },
            SubstateId::NonFungibleIndex(addr) => {
                let key = hasher32(EngineHashDomainLabel::NonFungibleIndex)
                    .chain(addr.resource_address())
                    .chain(&addr.index())
                    .result()
                    .trailing_bytes()
                    .into();
                ObjectKey::new(addr.resource_address().as_entity_id(), key)
            },
            SubstateId::UnclaimedConfidentialOutput(addr) => *addr.as_object_key(),
            SubstateId::TransactionReceipt(addr) => *addr.as_object_key(),
            SubstateId::FeeClaim(addr) => *addr.as_object_key(),
        }
    }

    // TODO: look at using BECH32 standard
    pub fn to_address_string(&self) -> String {
        self.to_string()
    }

    pub fn as_non_fungible_address(&self) -> Option<&NonFungibleAddress> {
        match self {
            SubstateId::NonFungible(addr) => Some(addr),
            _ => None,
        }
    }

    pub fn as_non_fungible_index_address(&self) -> Option<&NonFungibleIndexAddress> {
        match self {
            SubstateId::NonFungibleIndex(addr) => Some(addr),
            _ => None,
        }
    }

    pub fn is_resource(&self) -> bool {
        matches!(self, Self::Resource(_))
    }

    pub fn is_component(&self) -> bool {
        matches!(self, Self::Component(_))
    }

    pub fn is_root(&self) -> bool {
        // A component is a "root" substate i.e. it may not have a parent node. NOTE: this concept isn't well-defined
        // right now, this is simply used to prevent components being detected as dangling.
        matches!(self, Self::Component(_) | Self::NonFungibleIndex(_))
    }

    pub fn is_public_key_identity(&self) -> bool {
        matches!(self, Self::NonFungible(addr) if *addr.resource_address() == PUBLIC_IDENTITY_RESOURCE_ADDRESS)
    }

    pub fn is_virtual(&self) -> bool {
        self.is_public_key_identity()
    }

    pub fn is_vault(&self) -> bool {
        matches!(self, Self::Vault(_))
    }

    pub fn is_non_fungible(&self) -> bool {
        matches!(self, Self::NonFungible(_))
    }

    pub fn is_non_fungible_index(&self) -> bool {
        matches!(self, Self::NonFungibleIndex(_))
    }

    pub fn is_layer1_commitment(&self) -> bool {
        matches!(self, Self::UnclaimedConfidentialOutput(_))
    }

    pub fn is_transaction_receipt(&self) -> bool {
        matches!(self, Self::TransactionReceipt(_))
    }

    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::TransactionReceipt(_) | Self::Resource(_))
    }
}

impl From<ComponentAddress> for SubstateId {
    fn from(address: ComponentAddress) -> Self {
        Self::Component(address)
    }
}

impl From<ResourceAddress> for SubstateId {
    fn from(address: ResourceAddress) -> Self {
        Self::Resource(address)
    }
}

impl From<VaultId> for SubstateId {
    fn from(address: VaultId) -> Self {
        Self::Vault(address)
    }
}

impl From<NonFungibleAddress> for SubstateId {
    fn from(address: NonFungibleAddress) -> Self {
        Self::NonFungible(address)
    }
}

impl From<NonFungibleIndexAddress> for SubstateId {
    fn from(address: NonFungibleIndexAddress) -> Self {
        Self::NonFungibleIndex(address)
    }
}

impl From<UnclaimedConfidentialOutputAddress> for SubstateId {
    fn from(address: UnclaimedConfidentialOutputAddress) -> Self {
        Self::UnclaimedConfidentialOutput(address)
    }
}

impl From<FeeClaimAddress> for SubstateId {
    fn from(address: FeeClaimAddress) -> Self {
        Self::FeeClaim(address)
    }
}

impl From<TransactionReceiptAddress> for SubstateId {
    fn from(address: TransactionReceiptAddress) -> Self {
        Self::TransactionReceipt(address)
    }
}

impl TryFrom<SubstateId> for ComponentAddress {
    type Error = SubstateId;

    fn try_from(value: SubstateId) -> Result<Self, Self::Error> {
        match value {
            SubstateId::Component(addr) => Ok(addr),
            _ => Err(value),
        }
    }
}

impl Display for SubstateId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SubstateId::Component(addr) => write!(f, "{}", addr),
            SubstateId::Resource(addr) => write!(f, "{}", addr),
            SubstateId::Vault(addr) => write!(f, "{}", addr),
            SubstateId::NonFungible(addr) => write!(f, "{}", addr),
            SubstateId::NonFungibleIndex(addr) => write!(f, "{}", addr),
            SubstateId::UnclaimedConfidentialOutput(commitment_address) => write!(f, "{}", commitment_address),
            SubstateId::TransactionReceipt(addr) => write!(f, "{}", addr),
            SubstateId::FeeClaim(addr) => write!(f, "{}", addr),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid substate id '{0}'")]
pub struct InvalidSubstateIdFormat(String);

impl FromStr for SubstateId {
    type Err = InvalidSubstateIdFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once('_') {
            Some(("component", addr)) => {
                let addr = ComponentAddress::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                Ok(SubstateId::Component(addr))
            },
            Some(("resource", addr)) => {
                match addr.split_once(' ') {
                    Some((resource_str, addr)) => match addr.split_once('_') {
                        // resource_xxxx nft_xxxxx
                        Some(("nft", _)) => {
                            let nft_address =
                                NonFungibleAddress::from_str(s).map_err(|e| InvalidSubstateIdFormat(e.to_string()))?;
                            Ok(SubstateId::NonFungible(nft_address))
                        },
                        // resource_xxxx index_
                        Some(("index", index_str)) => {
                            let resource_addr = ResourceAddress::from_hex(resource_str)
                                .map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                            let index = u64::from_str(index_str).map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                            Ok(SubstateId::NonFungibleIndex(NonFungibleIndexAddress::new(
                                resource_addr,
                                index,
                            )))
                        },
                        _ => Err(InvalidSubstateIdFormat(s.to_string())),
                    },
                    // resource_xxxx
                    None => {
                        let addr =
                            ResourceAddress::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                        Ok(SubstateId::Resource(addr))
                    },
                }
            },
            Some(("vault", addr)) => {
                let id = VaultId::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                Ok(SubstateId::Vault(id))
            },
            Some(("commitment", addr)) => {
                let commitment_address = UnclaimedConfidentialOutputAddress::from_hex(addr)
                    .map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                Ok(SubstateId::UnclaimedConfidentialOutput(commitment_address))
            },
            Some(("txreceipt", addr)) => {
                let tx_receipt_addr =
                    TransactionReceiptAddress::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(addr.to_string()))?;
                Ok(SubstateId::TransactionReceipt(tx_receipt_addr))
            },
            Some(("feeclaim", addr)) => {
                let addr = Hash::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(addr.to_string()))?;
                Ok(SubstateId::FeeClaim(addr.into()))
            },
            Some(_) | None => Err(InvalidSubstateIdFormat(s.to_string())),
        }
    }
}

macro_rules! impl_partial_eq {
    ($typ:ty, $variant:ident) => {
        impl PartialEq<$typ> for SubstateId {
            fn eq(&self, other: &$typ) -> bool {
                match self {
                    SubstateId::$variant(addr) => addr == other,
                    _ => false,
                }
            }
        }
        impl PartialEq<SubstateId> for $typ {
            fn eq(&self, other: &SubstateId) -> bool {
                other == self
            }
        }
    };
}
impl_partial_eq!(ComponentAddress, Component);
impl_partial_eq!(ResourceAddress, Resource);
impl_partial_eq!(VaultId, Vault);
impl_partial_eq!(UnclaimedConfidentialOutputAddress, UnclaimedConfidentialOutput);
impl_partial_eq!(NonFungibleAddress, NonFungible);
impl_partial_eq!(TransactionReceiptAddress, TransactionReceipt);
impl_partial_eq!(FeeClaimAddress, FeeClaim);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum SubstateValue {
    Component(ComponentHeader),
    Resource(Resource),
    Vault(Vault),
    NonFungible(NonFungibleContainer),
    NonFungibleIndex(NonFungibleIndex),
    UnclaimedConfidentialOutput(UnclaimedConfidentialOutput),
    TransactionReceipt(TransactionReceipt),
    FeeClaim(FeeClaim),
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

    pub fn vault(&self) -> Option<&Vault> {
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

    pub fn non_fungible_index(&self) -> Option<&NonFungibleIndex> {
        match self {
            SubstateValue::NonFungibleIndex(index) => Some(index),
            _ => None,
        }
    }

    pub fn into_non_fungible_index(self) -> Option<NonFungibleIndex> {
        match self {
            SubstateValue::NonFungibleIndex(index) => Some(index),
            _ => None,
        }
    }

    pub fn into_unclaimed_confidential_output(self) -> Option<UnclaimedConfidentialOutput> {
        match self {
            SubstateValue::UnclaimedConfidentialOutput(output) => Some(output),
            _ => None,
        }
    }

    pub fn into_transaction_receipt(self) -> Option<TransactionReceipt> {
        match self {
            SubstateValue::TransactionReceipt(tx_receipt) => Some(tx_receipt),
            _ => None,
        }
    }

    pub fn as_component(&self) -> Option<&ComponentHeader> {
        match self {
            SubstateValue::Component(component) => Some(component),
            _ => None,
        }
    }

    pub fn as_transaction_receipt(&self) -> Option<&TransactionReceipt> {
        match self {
            SubstateValue::TransactionReceipt(tx_receipt) => Some(tx_receipt),
            _ => None,
        }
    }

    pub fn as_transaction_receipt_mut(&mut self) -> Option<&mut TransactionReceipt> {
        match self {
            SubstateValue::TransactionReceipt(tx_receipt) => Some(tx_receipt),
            _ => None,
        }
    }

    pub fn as_resource(&self) -> Option<&Resource> {
        match self {
            SubstateValue::Resource(resource) => Some(resource),
            _ => None,
        }
    }

    pub fn as_resource_mut(&mut self) -> Option<&mut Resource> {
        match self {
            SubstateValue::Resource(resource) => Some(resource),
            _ => None,
        }
    }

    pub fn as_vault(&self) -> Option<&Vault> {
        match self {
            SubstateValue::Vault(vault) => Some(vault),
            _ => None,
        }
    }

    pub fn as_vault_mut(&mut self) -> Option<&mut Vault> {
        match self {
            SubstateValue::Vault(vault) => Some(vault),
            _ => None,
        }
    }

    pub fn as_non_fungible(&self) -> Option<&NonFungibleContainer> {
        match self {
            SubstateValue::NonFungible(nft) => Some(nft),
            _ => None,
        }
    }

    pub fn as_non_fungible_mut(&mut self) -> Option<&mut NonFungibleContainer> {
        match self {
            SubstateValue::NonFungible(nft) => Some(nft),
            _ => None,
        }
    }

    pub fn as_unclaimed_confidential_output(&self) -> Option<&UnclaimedConfidentialOutput> {
        match self {
            SubstateValue::UnclaimedConfidentialOutput(output) => Some(output),
            _ => None,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BorError> {
        decode_exact(bytes)
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

impl From<NonFungibleIndex> for SubstateValue {
    fn from(index: NonFungibleIndex) -> Self {
        Self::NonFungibleIndex(index)
    }
}

impl From<FeeClaim> for SubstateValue {
    fn from(fee_claim: FeeClaim) -> Self {
        Self::FeeClaim(fee_claim)
    }
}

impl From<TransactionReceipt> for SubstateValue {
    fn from(tx_receipt: TransactionReceipt) -> Self {
        Self::TransactionReceipt(tx_receipt)
    }
}

impl Display for SubstateValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // TODO: improve output
        match self {
            SubstateValue::Component(component) => write!(f, "{:?}", component.state()),
            SubstateValue::Resource(resource) => write!(f, "{:?}", resource,),
            SubstateValue::Vault(vault) => write!(f, "{:?}", vault),
            SubstateValue::NonFungible(nft) => write!(f, "{:?}", nft),
            SubstateValue::NonFungibleIndex(index) => write!(f, "{:?}", index),
            SubstateValue::UnclaimedConfidentialOutput(commitment) => write!(f, "{:?}", commitment),
            SubstateValue::TransactionReceipt(tx_receipt) => write!(f, "{:?}", tx_receipt),
            SubstateValue::FeeClaim(fee_claim) => write!(f, "{:?}", fee_claim),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct SubstateDiff {
    up_substates: Vec<(SubstateId, Substate)>,
    down_substates: Vec<(SubstateId, u32)>,
}

impl SubstateDiff {
    pub fn new() -> Self {
        Self {
            up_substates: Vec::new(),
            down_substates: Vec::new(),
        }
    }

    pub fn up(&mut self, address: SubstateId, value: Substate) {
        self.up_substates.push((address, value));
    }

    pub fn extend_up(&mut self, iter: impl Iterator<Item = (SubstateId, Substate)>) -> &mut Self {
        self.up_substates.extend(iter);
        self
    }

    pub fn down(&mut self, address: SubstateId, version: u32) {
        self.down_substates.push((address, version));
    }

    pub fn extend_down(&mut self, iter: impl Iterator<Item = (SubstateId, u32)>) -> &mut Self {
        self.down_substates.extend(iter);
        self
    }

    pub fn up_iter(&self) -> impl Iterator<Item = &(SubstateId, Substate)> + '_ {
        self.up_substates.iter()
    }

    pub fn into_up_iter(self) -> impl Iterator<Item = (SubstateId, Substate)> {
        self.up_substates.into_iter()
    }

    pub fn down_iter(&self) -> impl Iterator<Item = &(SubstateId, u32)> + '_ {
        self.down_substates.iter()
    }

    pub fn up_len(&self) -> usize {
        self.up_substates.len()
    }

    pub fn down_len(&self) -> usize {
        self.down_substates.len()
    }

    pub fn len(&self) -> usize {
        self.up_len() + self.down_len()
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
            SubstateId::from_str("component_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff")
                .unwrap()
                .as_component_address()
                .unwrap();
            SubstateId::from_str("vault_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff")
                .unwrap()
                .as_vault_id()
                .unwrap();
            SubstateId::from_str("resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff")
                .unwrap()
                .as_resource_address()
                .unwrap();
            SubstateId::from_str(
                "resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff nft_str:SpecialNft",
            )
            .unwrap()
            .as_non_fungible_address()
            .unwrap();
            SubstateId::from_str(
                "resource_a7cf4fd18ada7f367b1c102a9c158abc3754491665033231c5eb907fffffffff \
                 nft_uuid:7f19c3fe5fa13ff66a0d379fe5f9e3508acbd338db6bedd7350d8d565b2c5d32",
            )
            .unwrap()
            .as_non_fungible_address()
            .unwrap();
            SubstateId::from_str("resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff index_0")
                .unwrap()
                .as_non_fungible_index_address()
                .unwrap();
        }
    }
}
