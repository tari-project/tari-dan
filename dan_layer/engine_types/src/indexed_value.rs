//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tari_bor::{decode, BorError, FromTagAndValue, ValueVisitor};
use tari_template_lib::{
    models::{BinaryTag, BucketId, NonFungibleAddressContents, ResourceAddress, VaultId},
    prelude::{ComponentAddress, Metadata, NonFungibleAddress},
    Hash,
};

use crate::{commit_result::TransactionReceiptAddress, serde_with, substate::SubstateAddress};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IndexedValue {
    buckets: Vec<BucketId>,
    #[serde(with = "serde_with::hex::vec")]
    component_addresses: Vec<ComponentAddress>,
    #[serde(with = "serde_with::hex::vec")]
    resource_addresses: Vec<ResourceAddress>,
    transaction_receipt_addresses: Vec<TransactionReceiptAddress>,
    // #[serde(with = "serde_with::hex::vec")]
    non_fungible_addresses: Vec<NonFungibleAddress>,
    #[serde(with = "serde_with::hex::vec")]
    vault_ids: Vec<VaultId>,
    metadata: Vec<Metadata>,
}

impl IndexedValue {
    pub fn from_raw(bytes: &[u8]) -> Result<Self, ValueVisitorError> {
        let mut visitor = IndexedValueVisitor::new();
        let value: tari_bor::Value = decode(bytes)?;
        tari_bor::walk_all(&value, &mut visitor)?;

        Ok(Self {
            buckets: visitor.buckets,
            resource_addresses: visitor.resource_addresses,
            component_addresses: visitor.component_addresses,
            transaction_receipt_addresses: visitor.transaction_receipt_addresses,
            non_fungible_addresses: visitor.non_fungible_addresses,
            vault_ids: visitor.vault_ids,
            metadata: visitor.metadata,
        })
    }

    pub fn contains_substate(&self, addr: &SubstateAddress) -> bool {
        match addr {
            SubstateAddress::Component(addr) => self.component_addresses.contains(addr),
            SubstateAddress::Resource(addr) => self.resource_addresses.contains(addr),
            SubstateAddress::TransactionReceipt(addr) => self.transaction_receipt_addresses.contains(addr),
            SubstateAddress::NonFungible(addr) => self.non_fungible_addresses.contains(addr),
            SubstateAddress::Vault(addr) => self.vault_ids.contains(addr),
            SubstateAddress::UnclaimedConfidentialOutput(_) => false,
            // TODO: should we index this value?
            SubstateAddress::NonFungibleIndex(_) => false,
        }
    }

    pub fn owned_substates(&self) -> impl Iterator<Item = SubstateAddress> + '_ {
        self.component_addresses
            .iter()
            .map(|a| (*a).into())
            .chain(self.resource_addresses.iter().map(|a| (*a).into()))
            .chain(self.non_fungible_addresses.iter().map(|a| a.clone().into()))
            .chain(self.vault_ids.iter().map(|a| (*a).into()))
    }

    pub fn buckets(&self) -> &[BucketId] {
        &self.buckets
    }

    pub fn component_addresses(&self) -> &[ComponentAddress] {
        &self.component_addresses
    }

    pub fn resource_addresses(&self) -> &[ResourceAddress] {
        &self.resource_addresses
    }

    pub fn non_fungible_addresses(&self) -> &[NonFungibleAddress] {
        &self.non_fungible_addresses
    }

    pub fn vault_ids(&self) -> &[VaultId] {
        &self.vault_ids
    }

    pub fn metadata(&self) -> &[Metadata] {
        &self.metadata
    }
}

pub enum TariValue {
    ComponentAddress(ComponentAddress),
    ResourceAddress(ResourceAddress),
    TransactionReceiptAddress(TransactionReceiptAddress),
    NonFungibleAddress(NonFungibleAddress),
    BucketId(BucketId),
    Metadata(Metadata),
    VaultId(VaultId),
}

impl FromTagAndValue for TariValue {
    type Error = ValueVisitorError;

    fn try_from_tag_and_value(tag: u64, value: &tari_bor::Value) -> Result<Self, Self::Error>
    where Self: Sized {
        let tag = BinaryTag::from_u64(tag).ok_or(ValueVisitorError::InvalidTag(tag))?;
        match tag {
            BinaryTag::ComponentAddress => {
                let component_address: Hash = value.deserialized().map_err(BorError::from)?;
                Ok(Self::ComponentAddress(component_address.into()))
            },
            BinaryTag::BucketId => {
                let bucket_id: u32 = value.deserialized().map_err(BorError::from)?;
                Ok(Self::BucketId(bucket_id.into()))
            },
            BinaryTag::ResourceAddress => {
                let resource_address: Hash = value.deserialized().map_err(BorError::from)?;
                Ok(Self::ResourceAddress(resource_address.into()))
            },
            BinaryTag::TransactionReceipt => {
                let execute_resource_address: Hash = value.deserialized().map_err(BorError::from)?;
                Ok(Self::TransactionReceiptAddress(execute_resource_address.into()))
            },
            BinaryTag::NonFungibleAddress => {
                let non_fungible_address: NonFungibleAddressContents = value.deserialized().map_err(BorError::from)?;
                Ok(Self::NonFungibleAddress(non_fungible_address.into()))
            },
            BinaryTag::Metadata => {
                let metadata: HashMap<String, String> = value.deserialized().map_err(BorError::from)?;
                Ok(Self::Metadata(metadata.into()))
            },
            BinaryTag::VaultId => {
                let vault_id: Hash = value.deserialized().map_err(BorError::from)?;
                Ok(Self::VaultId(vault_id.into()))
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IndexedValueVisitor {
    buckets: Vec<BucketId>,
    component_addresses: Vec<ComponentAddress>,
    resource_addresses: Vec<ResourceAddress>,
    transaction_receipt_addresses: Vec<TransactionReceiptAddress>,
    non_fungible_addresses: Vec<NonFungibleAddress>,
    vault_ids: Vec<VaultId>,
    metadata: Vec<Metadata>,
}

impl IndexedValueVisitor {
    pub fn new() -> Self {
        Self {
            buckets: vec![],
            component_addresses: vec![],
            resource_addresses: vec![],
            transaction_receipt_addresses: vec![],
            non_fungible_addresses: vec![],
            vault_ids: vec![],
            metadata: vec![],
        }
    }
}

impl ValueVisitor<TariValue> for IndexedValueVisitor {
    type Error = ValueVisitorError;

    fn visit(&mut self, value: TariValue) -> Result<(), Self::Error> {
        match value {
            TariValue::ComponentAddress(address) => {
                self.component_addresses.push(address);
            },
            TariValue::ResourceAddress(address) => {
                self.resource_addresses.push(address);
            },
            TariValue::TransactionReceiptAddress(address) => {
                self.transaction_receipt_addresses.push(address);
            },
            TariValue::BucketId(bucket_id) => {
                self.buckets.push(bucket_id);
            },
            TariValue::NonFungibleAddress(address) => {
                self.non_fungible_addresses.push(address);
            },
            TariValue::VaultId(vault_id) => {
                self.vault_ids.push(vault_id);
            },
            TariValue::Metadata(metadata) => {
                self.metadata.push(metadata);
            },
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValueVisitorError {
    #[error("Bor error: {0}")]
    BorError(#[from] tari_bor::BorError),
    #[error("Invalid tag: {0}")]
    InvalidTag(u64),
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rand::{rngs::OsRng, RngCore};
    use tari_template_lib::models::NonFungibleId;

    use super::*;
    use crate::hashing::{hasher, EngineHashDomainLabel};

    fn new_hash() -> Hash {
        hasher(EngineHashDomainLabel::ComponentAddress)
            .chain(&OsRng.next_u32())
            .result()
    }

    #[derive(Serialize, Deserialize)]
    struct SubStruct {
        buckets: Vec<BucketId>,
    }

    #[derive(Serialize, Deserialize)]
    struct TestStruct {
        name: String,
        component: ComponentAddress,
        components: Vec<ComponentAddress>,
        resource_map: HashMap<ResourceAddress, ComponentAddress>,
        sub_struct: SubStruct,
        sub_structs: Vec<SubStruct>,
        vault_ids: Vec<VaultId>,
        non_fungible_id: Option<NonFungibleAddress>,
        metadata: Metadata,
    }

    #[test]
    fn it_extracts_known_types_from_binary_data() {
        let addrs: [ComponentAddress; 3] = [new_hash().into(), new_hash().into(), new_hash().into()];
        let resx_addr = ResourceAddress::new(new_hash());

        let data = TestStruct {
            name: "John".to_string(),
            component: addrs[0],
            components: vec![addrs[1]],
            resource_map: {
                let mut m = HashMap::new();
                m.insert(resx_addr, addrs[2]);
                m
            },
            sub_struct: SubStruct {
                buckets: vec![1.into(), 2.into()],
            },
            sub_structs: vec![
                SubStruct {
                    buckets: vec![1.into(), 2.into()],
                },
                SubStruct {
                    buckets: vec![1.into(), 2.into()],
                },
            ],
            vault_ids: vec![VaultId::new(new_hash())],
            non_fungible_id: Some(NonFungibleAddress::new(resx_addr, NonFungibleId::Uint64(1))),
            metadata: Metadata::new(),
        };

        let bytes = tari_bor::encode(&data).unwrap();
        let indexed = IndexedValue::from_raw(&bytes).unwrap();

        assert!(indexed.component_addresses.contains(&addrs[0]));
        assert!(indexed.component_addresses.contains(&addrs[1]));
        assert!(indexed.component_addresses.contains(&addrs[2]));
        assert_eq!(indexed.component_addresses.len(), 3);
        assert_eq!(indexed.resource_addresses.len(), 1);

        assert_eq!(indexed.non_fungible_addresses.len(), 1);
        assert_eq!(indexed.vault_ids.len(), 1);
        assert_eq!(indexed.metadata.len(), 1);

        assert!(indexed.buckets.contains(&1.into()));
        assert!(indexed.buckets.contains(&2.into()));
        assert_eq!(indexed.buckets.len(), 6);
    }
}
