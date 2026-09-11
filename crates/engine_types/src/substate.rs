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
    any,
    fmt,
    fmt::{Debug, Display, Formatter},
    str::FromStr,
};

use borsh::{BorshDeserialize, BorshSerialize};
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_bor::{BorError, decode, decode_exact, encode};
use tari_template_lib::types::{
    ClaimedOutputTombstoneAddress,
    ComponentAddress,
    ConfidentialOutputAddress,
    Hash32,
    NonFungibleAddress,
    ObjectKey,
    ResourceAddress,
    TemplateAddress,
    TransactionReceiptAddress,
    UtxoAddress,
    ValidatorFeePoolAddress,
    VaultId,
    address_prefixes,
    constants::{PUBLIC_IDENTITY_RESOURCE_ADDRESS, STEALTH_TARI_RESOURCE_ADDRESS},
};

use crate::{
    Epoch,
    ProtocolVersion,
    ValidatorFeePool,
    ValidatorFeeWithdrawal,
    component::Component,
    confidential::ClaimedOutputTombstone,
    confidential_output::ConfidentialOutput,
    hashing::{EngineHashDomainLabel, hasher32, substate_value_hasher32},
    non_fungible::NonFungibleContainer,
    published_template::{PublishedTemplate, PublishedTemplateAddress},
    resource::Resource,
    substate_hasher::SubstateHashMessage,
    transaction_receipt::TransactionReceipt,
    utxo::Utxo,
    vault::Vault,
};

#[derive(Debug, Clone, minicbor::Encode, minicbor::Decode, minicbor::CborLen, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Substate {
    #[n(0)]
    substate: SubstateValue,
    #[n(1)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    version: u64,
}

impl Substate {
    pub fn new<T: Into<SubstateValue>>(version: u64, substate: T) -> Self {
        Self {
            substate: substate.into(),
            version,
        }
    }

    pub fn substate_value(&self) -> &SubstateValue {
        &self.substate
    }

    pub fn substate_value_mut(&mut self) -> &mut SubstateValue {
        &mut self.substate
    }

    pub fn into_substate_value(self) -> SubstateValue {
        self.substate
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BorError> {
        decode(bytes)
    }

    pub fn to_value_hash(&self, network: Network, epoch: Epoch) -> Hash32 {
        hash_substate(network, self.substate_value(), self.version, epoch)
    }

    pub fn previous_version(&self) -> Option<u64> {
        self.version.checked_sub(1)
    }
}

/// Hashes a substate into its canonical value hash. The `epoch` argument binds the schema version
/// (derived from `network` and epoch via `ProtocolVersion::at`) into the hash preimage, so substates
/// produced under different schema versions can never collide in the JMT.
pub fn hash_substate(network: Network, substate: &SubstateValue, version: u64, epoch: Epoch) -> Hash32 {
    let proto_version = ProtocolVersion::at(network, epoch);
    substate_value_hasher32()
        .chain(&substate.as_hash_message(proto_version))
        .chain(&version)
        .chain(&epoch)
        .result()
        .into_array()
        .into()
}

// BorshDeserialize is implemented for this struct because we de/encode keys in the database using this format
/// Base object address, version tuples
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "string"))]
pub enum SubstateId {
    #[n(0)]
    Component(#[n(0)] ComponentAddress),
    #[n(1)]
    Resource(#[n(0)] ResourceAddress),
    #[n(2)]
    Vault(#[n(0)] VaultId),
    #[n(3)]
    ClaimedOutputTombstone(#[n(0)] ClaimedOutputTombstoneAddress),
    #[n(4)]
    NonFungible(#[n(0)] NonFungibleAddress),
    #[n(5)]
    TransactionReceipt(#[n(0)] TransactionReceiptAddress),
    #[n(6)]
    Template(#[n(0)] PublishedTemplateAddress),
    #[n(7)]
    ValidatorFeePool(#[n(0)] ValidatorFeePoolAddress),
    #[n(8)]
    Utxo(#[n(0)] UtxoAddress),
    #[n(9)]
    ConfidentialOutput(#[n(0)] ConfidentialOutputAddress),
}

impl SubstateId {
    pub const fn as_component_address(&self) -> Option<ComponentAddress> {
        match self {
            Self::Component(addr) => Some(*addr),
            _ => None,
        }
    }

    pub const fn as_vault_id(&self) -> Option<VaultId> {
        match self {
            Self::Vault(id) => Some(*id),
            _ => None,
        }
    }

    pub const fn as_resource_address(&self) -> Option<ResourceAddress> {
        match self {
            Self::Resource(address) => Some(*address),
            _ => None,
        }
    }

    pub const fn as_unclaimed_confidential_output_address(&self) -> Option<ClaimedOutputTombstoneAddress> {
        match self {
            Self::ClaimedOutputTombstone(address) => Some(*address),
            _ => None,
        }
    }

    pub const fn as_template(&self) -> Option<PublishedTemplateAddress> {
        match self {
            Self::Template(address) => Some(*address),
            _ => None,
        }
    }

    pub const fn as_transaction_receipt_address(&self) -> Option<TransactionReceiptAddress> {
        match self {
            Self::TransactionReceipt(address) => Some(*address),
            _ => None,
        }
    }

    pub const fn as_validator_fee_pool_address(&self) -> Option<ValidatorFeePoolAddress> {
        match self {
            Self::ValidatorFeePool(address) => Some(*address),
            _ => None,
        }
    }

    pub fn as_utxo_address(&self) -> Option<UtxoAddress> {
        match self {
            Self::Utxo(address) => Some(address.clone()),
            _ => None,
        }
    }

    pub fn as_confidential_output_address(&self) -> Option<&ConfidentialOutputAddress> {
        match self {
            Self::ConfidentialOutput(address) => Some(address),
            _ => None,
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

                // This makes each NFT live on the same shard as the resource (for possible improved efficiency)
                // TODO: Review whether this is a good idea, as it leads to shard hotspots.
                ObjectKey::new(addr.resource_address().as_entity_id(), key)
            },
            SubstateId::ClaimedOutputTombstone(addr) => *addr.as_object_key(),
            SubstateId::TransactionReceipt(addr) => *addr.as_object_key(),
            SubstateId::Template(addr) => *addr.as_object_key(),
            SubstateId::ValidatorFeePool(addr) => *addr.as_object_key(),
            SubstateId::Utxo(addr) => {
                let key = hasher32(EngineHashDomainLabel::UtxoAddress)
                    .chain(addr.resource_address())
                    .chain(addr.id())
                    .result()
                    .into_array();

                ObjectKey::from_array(key)
            },
            SubstateId::ConfidentialOutput(addr) => {
                let key = hasher32(EngineHashDomainLabel::ConfidentialOutputAddress)
                    .chain(addr.resource_address())
                    .chain(addr.commitment())
                    .result()
                    .into_array();

                ObjectKey::from_array(key)
            },
        }
    }

    pub fn to_address_string(&self) -> String {
        self.to_string()
    }

    pub const fn as_non_fungible_address(&self) -> Option<&NonFungibleAddress> {
        match self {
            SubstateId::NonFungible(addr) => Some(addr),
            _ => None,
        }
    }

    pub const fn is_resource(&self) -> bool {
        matches!(self, Self::Resource(_))
    }

    pub const fn is_component(&self) -> bool {
        matches!(self, Self::Component(_))
    }

    pub const fn is_root(&self) -> bool {
        // A component and utxo are "root" substates i.e. they may not have a parent node. NOTE: this concept isn't
        // well-defined right now, this is simply used to prevent components being detected as dangling.
        matches!(
            self,
            Self::Component(_) | Self::Utxo(_) | Self::ConfidentialOutput(_) | Self::ClaimedOutputTombstone(_)
        )
    }

    pub fn is_public_key_identity(&self) -> bool {
        matches!(self, Self::NonFungible(addr) if *addr.resource_address() == PUBLIC_IDENTITY_RESOURCE_ADDRESS)
    }

    /// Returns `true` for substate ids the engine reserves and never stores: public-key identities, the caller
    /// badges stamped into an authorization scope at frame push, and the two resources those badges are namespaced
    /// by. Nothing can ever be resolved or locked at one of these, so they are neither derived as transaction
    /// inputs nor brought into a call scope, and a transaction naming one as an input is rejected at validation.
    pub fn is_virtual(&self) -> bool {
        match self {
            Self::NonFungible(addr) => {
                let resource = addr.resource_address();
                resource.is_caller_badge() || *resource == PUBLIC_IDENTITY_RESOURCE_ADDRESS
            },
            Self::Resource(addr) => addr.is_caller_badge(),
            _ => false,
        }
    }

    pub const fn is_vault(&self) -> bool {
        matches!(self, Self::Vault(_))
    }

    pub const fn is_non_fungible(&self) -> bool {
        matches!(self, Self::NonFungible(_))
    }

    pub const fn is_claimed_output_tombstone(&self) -> bool {
        matches!(self, Self::ClaimedOutputTombstone(_))
    }

    pub const fn is_transaction_receipt(&self) -> bool {
        matches!(self, Self::TransactionReceipt(_))
    }

    pub const fn is_template(&self) -> bool {
        matches!(self, Self::Template(_))
    }

    pub const fn is_validator_fee_pool(&self) -> bool {
        matches!(self, Self::ValidatorFeePool(_))
    }

    pub const fn is_utxo(&self) -> bool {
        matches!(self, Self::Utxo(_))
    }

    pub const fn is_confidential_output(&self) -> bool {
        matches!(self, Self::ConfidentialOutput(_))
    }

    pub const fn is_global(&self) -> bool {
        self.is_template()
    }

    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::TransactionReceipt(_) | Self::Template(_) | Self::ClaimedOutputTombstone(_)
        ) || {
            let addr = self.as_resource_address();
            addr == Some(STEALTH_TARI_RESOURCE_ADDRESS) || addr == Some(PUBLIC_IDENTITY_RESOURCE_ADDRESS)
        }
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

impl From<ClaimedOutputTombstoneAddress> for SubstateId {
    fn from(address: ClaimedOutputTombstoneAddress) -> Self {
        Self::ClaimedOutputTombstone(address)
    }
}

impl From<TransactionReceiptAddress> for SubstateId {
    fn from(address: TransactionReceiptAddress) -> Self {
        Self::TransactionReceipt(address)
    }
}

impl From<PublishedTemplateAddress> for SubstateId {
    fn from(address: PublishedTemplateAddress) -> Self {
        Self::Template(address)
    }
}

impl From<ValidatorFeePoolAddress> for SubstateId {
    fn from(address: ValidatorFeePoolAddress) -> Self {
        Self::ValidatorFeePool(address)
    }
}

impl From<UtxoAddress> for SubstateId {
    fn from(address: UtxoAddress) -> Self {
        Self::Utxo(address)
    }
}

impl From<ConfidentialOutputAddress> for SubstateId {
    fn from(address: ConfidentialOutputAddress) -> Self {
        Self::ConfidentialOutput(address)
    }
}

impl AsRef<SubstateId> for SubstateId {
    fn as_ref(&self) -> &SubstateId {
        self
    }
}
#[derive(Debug, thiserror::Error)]
#[error("Could not convert substate ID variant '{substate_id}' to {expected}")]
pub struct InvalidSubstateIdVariant {
    pub substate_id: SubstateId,
    pub expected: &'static str,
}

impl TryFrom<SubstateId> for ComponentAddress {
    type Error = InvalidSubstateIdVariant;

    fn try_from(value: SubstateId) -> Result<Self, Self::Error> {
        match value {
            SubstateId::Component(addr) => Ok(addr),
            _ => Err(InvalidSubstateIdVariant {
                substate_id: value,
                expected: any::type_name::<Self>(),
            }),
        }
    }
}

impl TryFrom<SubstateId> for ResourceAddress {
    type Error = InvalidSubstateIdVariant;

    fn try_from(value: SubstateId) -> Result<Self, Self::Error> {
        match value {
            SubstateId::Resource(addr) => Ok(addr),
            _ => Err(InvalidSubstateIdVariant {
                substate_id: value,
                expected: any::type_name::<Self>(),
            }),
        }
    }
}

impl TryFrom<SubstateId> for VaultId {
    type Error = InvalidSubstateIdVariant;

    fn try_from(value: SubstateId) -> Result<Self, Self::Error> {
        match value {
            SubstateId::Vault(addr) => Ok(addr),
            _ => Err(InvalidSubstateIdVariant {
                substate_id: value,
                expected: any::type_name::<Self>(),
            }),
        }
    }
}

impl TryFrom<SubstateId> for NonFungibleAddress {
    type Error = InvalidSubstateIdVariant;

    fn try_from(value: SubstateId) -> Result<Self, Self::Error> {
        match value {
            SubstateId::NonFungible(addr) => Ok(addr),
            _ => Err(InvalidSubstateIdVariant {
                substate_id: value,
                expected: any::type_name::<Self>(),
            }),
        }
    }
}

impl TryFrom<SubstateId> for ClaimedOutputTombstoneAddress {
    type Error = InvalidSubstateIdVariant;

    fn try_from(value: SubstateId) -> Result<Self, Self::Error> {
        match value {
            SubstateId::ClaimedOutputTombstone(addr) => Ok(addr),
            _ => Err(InvalidSubstateIdVariant {
                substate_id: value,
                expected: any::type_name::<Self>(),
            }),
        }
    }
}

impl TryFrom<SubstateId> for TransactionReceiptAddress {
    type Error = InvalidSubstateIdVariant;

    fn try_from(value: SubstateId) -> Result<Self, Self::Error> {
        match value {
            SubstateId::TransactionReceipt(addr) => Ok(addr),
            _ => Err(InvalidSubstateIdVariant {
                substate_id: value,
                expected: any::type_name::<Self>(),
            }),
        }
    }
}

impl TryFrom<SubstateId> for PublishedTemplateAddress {
    type Error = InvalidSubstateIdVariant;

    fn try_from(value: SubstateId) -> Result<Self, Self::Error> {
        match value {
            SubstateId::Template(addr) => Ok(addr),
            _ => Err(InvalidSubstateIdVariant {
                substate_id: value,
                expected: any::type_name::<Self>(),
            }),
        }
    }
}

impl Display for SubstateId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            SubstateId::Component(addr) => Display::fmt(addr, f),
            SubstateId::Resource(addr) => Display::fmt(addr, f),
            SubstateId::Vault(addr) => Display::fmt(addr, f),
            SubstateId::NonFungible(addr) => Display::fmt(addr, f),
            SubstateId::ClaimedOutputTombstone(addr) => Display::fmt(addr, f),
            SubstateId::TransactionReceipt(addr) => Display::fmt(addr, f),
            SubstateId::Template(addr) => Display::fmt(addr, f),
            SubstateId::ValidatorFeePool(addr) => Display::fmt(addr, f),
            SubstateId::Utxo(addr) => Display::fmt(addr, f),
            SubstateId::ConfidentialOutput(addr) => Display::fmt(addr, f),
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
            Some((address_prefixes::COMPONENT, addr)) => {
                let addr = ComponentAddress::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                Ok(SubstateId::Component(addr))
            },
            Some((address_prefixes::RESOURCE, addr)) => {
                // resource_xxxxx
                let addr = ResourceAddress::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                Ok(SubstateId::Resource(addr))
            },
            Some((address_prefixes::NON_FUNGIBLE, rest)) => {
                // nft_{resource_hex}_{id_type}_{id}
                let addr = NonFungibleAddress::from_str(rest).map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                Ok(SubstateId::NonFungible(addr))
            },
            Some((address_prefixes::VAULT, addr)) => {
                let id = VaultId::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                Ok(SubstateId::Vault(id))
            },
            Some((address_prefixes::CLAIMED_OUTPUT_TOMBSTONE, addr)) => {
                let address = ClaimedOutputTombstoneAddress::from_hex(addr)
                    .map_err(|_| InvalidSubstateIdFormat(s.to_string()))?;
                Ok(SubstateId::ClaimedOutputTombstone(address))
            },
            Some((address_prefixes::TRANSACTION_RECEIPT, addr)) => {
                let tx_receipt_addr =
                    TransactionReceiptAddress::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(addr.to_string()))?;
                Ok(SubstateId::TransactionReceipt(tx_receipt_addr))
            },
            Some((address_prefixes::TEMPLATE, addr)) => {
                let addr = Hash32::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(addr.to_string()))?;
                Ok(SubstateId::Template(addr.into()))
            },
            Some((address_prefixes::VALIDATOR_FEE_POOL, addr)) => {
                let addr =
                    ValidatorFeePoolAddress::from_hex(addr).map_err(|_| InvalidSubstateIdFormat(addr.to_string()))?;
                Ok(SubstateId::ValidatorFeePool(addr))
            },
            Some((address_prefixes::UTXO, addr)) => {
                let addr = UtxoAddress::from_str(addr).map_err(|_| InvalidSubstateIdFormat(addr.to_string()))?;
                Ok(SubstateId::Utxo(addr))
            },
            Some((address_prefixes::CONFIDENTIAL_OUTPUT, addr)) => {
                let addr =
                    ConfidentialOutputAddress::from_str(addr).map_err(|_| InvalidSubstateIdFormat(addr.to_string()))?;
                Ok(SubstateId::ConfidentialOutput(addr))
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
impl_partial_eq!(ClaimedOutputTombstoneAddress, ClaimedOutputTombstone);
impl_partial_eq!(NonFungibleAddress, NonFungible);
impl_partial_eq!(ConfidentialOutputAddress, ConfidentialOutput);
impl_partial_eq!(TransactionReceiptAddress, TransactionReceipt);
impl_partial_eq!(PublishedTemplateAddress, Template);
impl_partial_eq!(ValidatorFeePoolAddress, ValidatorFeePool);
impl_partial_eq!(UtxoAddress, Utxo);

#[derive(
    Debug, Clone, Serialize, Deserialize, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum SubstateValue {
    #[n(0)]
    Component(#[n(0)] Component),
    #[n(1)]
    Resource(#[n(0)] Box<Resource>),
    #[n(2)]
    Vault(#[n(0)] Vault),
    #[n(3)]
    NonFungible(#[n(0)] NonFungibleContainer),
    #[n(4)]
    ClaimedOutputTombstone(#[n(0)] ClaimedOutputTombstone),
    #[n(5)]
    TransactionReceipt(#[n(0)] TransactionReceipt),
    #[n(6)]
    Template(#[n(0)] PublishedTemplate),
    #[n(7)]
    ValidatorFeePool(#[n(0)] ValidatorFeePool),
    #[n(8)]
    Utxo(#[n(0)] Utxo),
    #[n(9)]
    ConfidentialOutput(#[n(0)] ConfidentialOutput),
}

impl SubstateValue {
    pub fn into_component(self) -> Option<Component> {
        match self {
            SubstateValue::Component(component) => Some(component),
            _ => None,
        }
    }

    pub fn component(&self) -> Option<&Component> {
        match self {
            SubstateValue::Component(component) => Some(component),
            _ => None,
        }
    }

    pub fn component_mut(&mut self) -> Option<&mut Component> {
        match self {
            SubstateValue::Component(component) => Some(component),
            _ => None,
        }
    }

    pub fn published_template(&self) -> Option<&PublishedTemplate> {
        match self {
            SubstateValue::Template(template) => Some(template),
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
            SubstateValue::Resource(resource) => Some(*resource),
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

    pub fn into_unclaimed_confidential_output(self) -> Option<ClaimedOutputTombstone> {
        match self {
            SubstateValue::ClaimedOutputTombstone(output) => Some(output),
            _ => None,
        }
    }

    pub fn into_transaction_receipt(self) -> Option<TransactionReceipt> {
        match self {
            SubstateValue::TransactionReceipt(tx_receipt) => Some(tx_receipt),
            _ => None,
        }
    }

    pub fn into_template(self) -> Option<PublishedTemplate> {
        match self {
            SubstateValue::Template(template) => Some(template),
            _ => None,
        }
    }

    pub fn into_utxo(self) -> Option<Utxo> {
        match self {
            SubstateValue::Utxo(utxo) => Some(utxo),
            _ => None,
        }
    }

    pub fn as_component(&self) -> Option<&Component> {
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

    pub fn as_claimed_output_tombstone(&self) -> Option<&ClaimedOutputTombstone> {
        match self {
            SubstateValue::ClaimedOutputTombstone(output) => Some(output),
            _ => None,
        }
    }

    pub fn as_validator_fee_pool(&self) -> Option<&ValidatorFeePool> {
        match self {
            SubstateValue::ValidatorFeePool(value) => Some(value),
            _ => None,
        }
    }

    pub fn into_validator_fee_pool(self) -> Option<ValidatorFeePool> {
        match self {
            SubstateValue::ValidatorFeePool(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_validator_fee_pool_mut(&mut self) -> Option<&mut ValidatorFeePool> {
        match self {
            SubstateValue::ValidatorFeePool(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_template(&self) -> Option<&PublishedTemplate> {
        match self {
            SubstateValue::Template(template) => Some(template),
            _ => None,
        }
    }

    pub fn as_utxo(&self) -> Option<&Utxo> {
        match self {
            SubstateValue::Utxo(utxo) => Some(utxo),
            _ => None,
        }
    }

    pub fn as_utxo_mut(&mut self) -> Option<&mut Utxo> {
        match self {
            SubstateValue::Utxo(utxo) => Some(utxo),
            _ => None,
        }
    }

    pub fn as_confidential_output(&self) -> Option<&ConfidentialOutput> {
        match self {
            SubstateValue::ConfidentialOutput(output) => Some(output),
            _ => None,
        }
    }

    pub fn as_confidential_output_mut(&mut self) -> Option<&mut ConfidentialOutput> {
        match self {
            SubstateValue::ConfidentialOutput(output) => Some(output),
            _ => None,
        }
    }

    pub fn into_confidential_output(self) -> Option<ConfidentialOutput> {
        match self {
            SubstateValue::ConfidentialOutput(output) => Some(output),
            _ => None,
        }
    }

    pub fn component_template_address(&self) -> Option<&TemplateAddress> {
        self.as_component().map(|c| c.template_address())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BorError> {
        decode_exact(bytes)
    }

    pub fn as_hash_message(&self, proto_version: ProtocolVersion) -> SubstateHashMessage<'_> {
        SubstateHashMessage::new(proto_version, self)
    }
}

impl From<Component> for SubstateValue {
    fn from(component: Component) -> Self {
        Self::Component(component)
    }
}

impl From<Resource> for SubstateValue {
    fn from(resource: Resource) -> Self {
        Self::Resource(Box::new(resource))
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

impl From<TransactionReceipt> for SubstateValue {
    fn from(tx_receipt: TransactionReceipt) -> Self {
        Self::TransactionReceipt(tx_receipt)
    }
}

impl From<ClaimedOutputTombstone> for SubstateValue {
    fn from(output: ClaimedOutputTombstone) -> Self {
        Self::ClaimedOutputTombstone(output)
    }
}

impl From<PublishedTemplate> for SubstateValue {
    fn from(template: PublishedTemplate) -> Self {
        Self::Template(template)
    }
}

impl From<ValidatorFeePool> for SubstateValue {
    fn from(value: ValidatorFeePool) -> Self {
        Self::ValidatorFeePool(value)
    }
}

impl From<Utxo> for SubstateValue {
    fn from(utxo: Utxo) -> Self {
        Self::Utxo(utxo)
    }
}

impl From<ConfidentialOutput> for SubstateValue {
    fn from(output: ConfidentialOutput) -> Self {
        Self::ConfidentialOutput(output)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SubstateDiff {
    #[n(0)]
    up_substates: Vec<(SubstateId, Substate)>,
    #[n(1)]
    down_substates: Vec<(SubstateId, u64)>,
    #[n(2)]
    fee_withdrawals: Vec<ValidatorFeeWithdrawal>,
}

impl SubstateDiff {
    pub fn new() -> Self {
        Self {
            up_substates: Vec::new(),
            down_substates: Vec::new(),
            fee_withdrawals: Vec::new(),
        }
    }

    pub fn up(&mut self, id: SubstateId, value: Substate) {
        self.up_substates.push((id, value));
    }

    /// Set the fee withdrawals for this diff.
    ///
    /// # Panics
    /// Panics if the fee withdrawals have already been set.
    pub fn set_once_fee_withdrawals(&mut self, withdrawals: Vec<ValidatorFeeWithdrawal>) -> &mut Self {
        assert!(self.fee_withdrawals.is_empty(), "Fee withdrawals set more than once");
        self.fee_withdrawals = withdrawals;
        self
    }

    pub fn extend_up(&mut self, iter: impl Iterator<Item = (SubstateId, Substate)>) -> &mut Self {
        self.up_substates.extend(iter);
        self
    }

    pub fn down(&mut self, id: SubstateId, version: u64) {
        self.down_substates.push((id, version));
    }

    pub fn extend_down(&mut self, iter: impl Iterator<Item = (SubstateId, u64)>) -> &mut Self {
        self.down_substates.extend(iter);
        self
    }

    pub fn up_iter(&self) -> impl Iterator<Item = &(SubstateId, Substate)> + '_ {
        self.up_substates.iter()
    }

    pub fn into_up_iter(self) -> impl Iterator<Item = (SubstateId, Substate)> {
        self.up_substates.into_iter()
    }

    pub fn down_iter(&self) -> impl Iterator<Item = &(SubstateId, u64)> + '_ {
        self.down_substates.iter()
    }

    pub fn validator_fee_withdrawals(&self) -> &[ValidatorFeeWithdrawal] {
        &self.fee_withdrawals
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

    mod substate_id_parse {
        use super::*;

        #[test]
        fn it_parses_valid_substate_ids() {
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
            SubstateId::from_str("nft_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff_str_SpecialNft")
                .unwrap()
                .as_non_fungible_address()
                .unwrap();
            SubstateId::from_str(
                "nft_a7cf4fd18ada7f367b1c102a9c158abc3754491665033231c5eb907fffffffff_uuid_7f19c3fe5fa13ff66a0d379fe5f9e3508acbd338db6bedd7350d8d565b2c5d32",
            )
                .unwrap()
                .as_non_fungible_address()
                .unwrap();
            SubstateId::from_str("template_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff")
                .unwrap()
                .as_template()
                .unwrap();
        }

        #[test]
        fn it_parses_a_display_string() {
            fn check(s: &str) {
                let id = SubstateId::from_str(s).unwrap();
                assert_eq!(id.to_string(), s);
            }
            check("component_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff");
            check("vault_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff");
            check("resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff");
            check("nft_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff_str_SpecialNft");
            check(
                "nft_a7cf4fd18ada7f367b1c102a9c158abc3754491665033231c5eb907fffffffff_uuid_7f19c3fe5fa13ff66a0d379fe5f9e3508acbd338db6bedd7350d8d565b2c5d32",
            );
            check("vnfp_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff");
            check("txreceipt_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff");
            check("tombstone_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff");
            check("template_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff");
            check(
                "utxo_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff",
            );
        }
    }

    mod hash_substate_epoch_binding {
        use super::*;
        use crate::confidential::ClaimedOutputTombstone;

        const NETWORK: Network = Network::LocalNet;

        fn sample_value() -> SubstateValue {
            SubstateValue::ClaimedOutputTombstone(ClaimedOutputTombstone { value: 1 })
        }

        #[test]
        fn different_epochs_yield_different_hashes() {
            let v = sample_value();
            let h0 = hash_substate(NETWORK, &v, 0, Epoch::zero());
            let h1 = hash_substate(NETWORK, &v, 0, Epoch(1));
            assert_ne!(h0, h1, "epoch must bind into the hash preimage");
        }

        #[test]
        fn same_epoch_same_inputs_stable() {
            let v = sample_value();
            assert_eq!(
                hash_substate(NETWORK, &v, 0, Epoch(42)),
                hash_substate(NETWORK, &v, 0, Epoch(42))
            );
        }

        #[test]
        fn version_still_binds() {
            let v = sample_value();
            assert_ne!(
                hash_substate(NETWORK, &v, 0, Epoch::zero()),
                hash_substate(NETWORK, &v, 1, Epoch::zero())
            );
        }
    }
}
