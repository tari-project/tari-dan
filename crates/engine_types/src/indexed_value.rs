//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{any::type_name, collections::HashSet, hash::Hash, ops::ControlFlow};

use serde::{Deserialize, Serialize};
use tari_bor::{BorError, FromTagAndValue, ValueVisitor, decode};
use tari_template_lib::{
    models::{BucketId, ComponentAddressAllocation, ProofId, ResourceAddressAllocation},
    types::{
        BinaryTag,
        ClaimedOutputTombstoneAddress,
        ComponentAddress,
        ConfidentialOutputAddress,
        ConfidentialOutputAddressContents,
        Hash32,
        NonFungibleAddress,
        NonFungibleAddressContents,
        ObjectKey,
        ResourceAddress,
        TransactionReceiptAddress,
        UtxoAddress,
        UtxoAddressContents,
        ValidatorFeePoolAddress,
        VaultId,
    },
};

use crate::{published_template::PublishedTemplateAddress, substate::SubstateId};

const MAX_VISITOR_DEPTH: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct IndexedValue {
    #[n(0)]
    indexed: IndexedWellKnownTypes,
    #[serde(with = "ootle_serde::cbor_value")]
    #[cfg_attr(feature = "ts", ts(type = "any"))]
    #[n(1)]
    value: tari_bor::Value,
}

impl IndexedValue {
    pub fn from_type<T: tari_bor::Encode<()> + ?Sized>(v: &T) -> Result<Self, IndexedValueError> {
        let value = tari_bor::to_value(v)
            .map_err(|e| IndexedValueError::Custom(format!("from_type<{}>: {}", type_name::<T>(), e)))?;
        Self::from_value(value)
    }

    pub fn from_raw(bytes: &[u8]) -> Result<Self, IndexedValueError> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        let value: tari_bor::Value =
            decode(bytes).map_err(|e| IndexedValueError::Custom(format!("from_raw: {}", e)))?;
        Self::from_value(value)
    }

    pub fn from_value(value: tari_bor::Value) -> Result<Self, IndexedValueError> {
        let indexed = IndexedWellKnownTypes::from_value(&value)
            .map_err(|e| IndexedValueError::Custom(format!("from_value: {}", e)))?;
        Ok(Self { indexed, value })
    }

    pub fn referenced_substates(&self) -> impl Iterator<Item = SubstateId> + '_ {
        self.indexed.referenced_substates()
    }

    pub fn well_known_types(&self) -> &IndexedWellKnownTypes {
        &self.indexed
    }

    pub fn bucket_ids(&self) -> &[BucketId] {
        &self.indexed.bucket_ids
    }

    pub fn proof_ids(&self) -> &[ProofId] {
        &self.indexed.proof_ids
    }

    pub fn component_addresses(&self) -> &[ComponentAddress] {
        &self.indexed.component_addresses
    }

    pub fn resource_addresses(&self) -> &[ResourceAddress] {
        &self.indexed.resource_addresses
    }

    pub fn non_fungible_addresses(&self) -> &[NonFungibleAddress] {
        &self.indexed.non_fungible_addresses
    }

    pub fn vault_ids(&self) -> &[VaultId] {
        &self.indexed.vault_ids
    }

    pub fn value(&self) -> &tari_bor::Value {
        &self.value
    }

    pub fn into_value(self) -> tari_bor::Value {
        self.value
    }

    pub fn decoded<T>(&self) -> Result<T, IndexedValueError>
    where T: for<'b> tari_bor::Decode<'b, ()> {
        tari_bor::from_value(&self.value).map_err(Into::into)
    }

    pub fn get_value<T>(&self, path: &str) -> Result<Option<T>, IndexedValueError>
    where T: for<'b> tari_bor::Decode<'b, ()> {
        decode_value_at_path(&self.value, path)
    }

    pub const fn empty() -> Self {
        Self {
            indexed: IndexedWellKnownTypes::new(),
            value: tari_bor::Value::Null,
        }
    }
}

impl Default for IndexedValue {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(
    Debug, Clone, Default, Serialize, Deserialize, PartialEq, minicbor::Encode, minicbor::Decode, minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct IndexedWellKnownTypes {
    #[n(0)]
    bucket_ids: Vec<BucketId>,
    #[n(1)]
    proof_ids: Vec<ProofId>,
    #[n(2)]
    component_addresses: Vec<ComponentAddress>,
    #[n(3)]
    resource_addresses: Vec<ResourceAddress>,
    #[n(4)]
    transaction_receipt_addresses: Vec<TransactionReceiptAddress>,
    #[n(5)]
    non_fungible_addresses: Vec<NonFungibleAddress>,
    #[n(6)]
    vault_ids: Vec<VaultId>,
    #[n(8)]
    unclaimed_confidential_output_address: Vec<ClaimedOutputTombstoneAddress>,
    #[n(9)]
    published_template_addresses: Vec<PublishedTemplateAddress>,
    #[n(10)]
    validator_node_fee_pools: Vec<ValidatorFeePoolAddress>,
    #[serde(default)]
    #[cbor(default)]
    #[n(11)]
    utxos: Vec<UtxoAddress>,
    #[cfg_attr(feature = "ts", ts(type = "number[]"))]
    #[n(12)]
    component_address_allocations: Vec<ComponentAddressAllocation>,
    #[cfg_attr(feature = "ts", ts(type = "number[]"))]
    #[n(13)]
    resource_address_allocations: Vec<ResourceAddressAllocation>,
    #[serde(default)]
    #[cbor(default)]
    #[n(14)]
    confidential_outputs: Vec<ConfidentialOutputAddress>,
}

impl IndexedWellKnownTypes {
    pub const fn new() -> Self {
        Self {
            bucket_ids: vec![],
            proof_ids: vec![],
            component_addresses: vec![],
            resource_addresses: vec![],
            transaction_receipt_addresses: vec![],
            non_fungible_addresses: vec![],
            vault_ids: vec![],
            unclaimed_confidential_output_address: vec![],
            published_template_addresses: vec![],
            validator_node_fee_pools: vec![],
            utxos: vec![],
            component_address_allocations: vec![],
            resource_address_allocations: vec![],
            confidential_outputs: vec![],
        }
    }

    pub fn from_value(value: &tari_bor::Value) -> Result<Self, IndexedValueError> {
        Self::from_value_with_max_depth(value, MAX_VISITOR_DEPTH)
    }

    /// Whether [`Self::from_value`] decodes the value under `tag` as a well-known type of its own,
    /// which costs far more than walking an item.
    pub fn indexes_tag(tag: u64) -> bool {
        BinaryTag::from_u64(tag).is_some_and(|tag| !matches!(tag, BinaryTag::Metadata))
    }

    fn from_value_with_max_depth(value: &tari_bor::Value, max_depth: usize) -> Result<Self, IndexedValueError> {
        let mut visitor = IndexedValueVisitor::new();
        tari_bor::walk_all(value, &mut visitor, max_depth)?;

        Ok(Self {
            bucket_ids: visitor.buckets,
            proof_ids: visitor.proofs,
            resource_addresses: visitor.resource_addresses,
            component_addresses: visitor.component_addresses,
            transaction_receipt_addresses: visitor.transaction_receipt_addresses,
            non_fungible_addresses: visitor.non_fungible_addresses,
            vault_ids: visitor.vault_ids,
            unclaimed_confidential_output_address: visitor.unclaimed_confidential_output_addresses,
            published_template_addresses: visitor.published_templates,
            validator_node_fee_pools: visitor.validator_node_fee_pools,
            utxos: visitor.utxos,
            component_address_allocations: visitor.component_address_allocations,
            resource_address_allocations: visitor.resource_address_allocations,
            confidential_outputs: visitor.confidential_outputs,
        })
    }

    /// Checks if a value contains a substate with the given address. This function does not allocate.
    pub fn value_contains_substate(value: &tari_bor::Value, id: &SubstateId) -> Result<bool, IndexedValueError> {
        let mut found = false;
        tari_bor::walk_all(
            value,
            &mut |value: WellKnownTariValue| {
                match value {
                    WellKnownTariValue::ComponentAddress(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::ResourceAddress(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::TransactionReceiptAddress(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::NonFungibleAddress(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::VaultId(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::ClaimedOutputTombstoneAddress(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::PublishedTemplateAddress(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::ValidatorNodeFeePool(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::Utxo(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::ConfidentialOutput(addr) => {
                        found = *id == addr;
                    },
                    WellKnownTariValue::BucketId(_) |
                    WellKnownTariValue::ComponentAddressAllocation(_) |
                    WellKnownTariValue::ResourceAddressAllocation(_) |
                    WellKnownTariValue::ProofId(_) => {},
                }

                if found {
                    Ok(ControlFlow::Break(()))
                } else {
                    Ok(ControlFlow::Continue(()))
                }
            },
            MAX_VISITOR_DEPTH,
        )?;
        Ok(found)
    }

    pub fn referenced_substates(&self) -> impl Iterator<Item = SubstateId> + '_ {
        self.component_addresses
            .iter()
            .map(|a| (*a).into())
            .chain(self.resource_addresses.iter().map(|a| (*a).into()))
            .chain(self.transaction_receipt_addresses.iter().map(|a| (*a).into()))
            .chain(self.non_fungible_addresses.iter().map(|a| a.clone().into()))
            .chain(self.vault_ids.iter().map(|a| (*a).into()))
            .chain(self.unclaimed_confidential_output_address.iter().map(|a| (*a).into()))
            .chain(self.utxos.iter().map(|a| a.clone().into()))
            .chain(self.confidential_outputs.iter().map(|a| a.clone().into()))
            .chain(self.published_template_addresses.iter().map(|a| (*a).into()))
            .chain(self.validator_node_fee_pools.iter().map(|a| (*a).into()))
    }

    pub fn into_referenced_substates(self) -> impl Iterator<Item = SubstateId> {
        self.component_addresses
            .into_iter()
            .map(Into::into)
            .chain(self.resource_addresses.into_iter().map(Into::into))
            .chain(self.transaction_receipt_addresses.into_iter().map(Into::into))
            .chain(self.non_fungible_addresses.into_iter().map(Into::into))
            .chain(self.vault_ids.into_iter().map(Into::into))
            .chain(self.unclaimed_confidential_output_address.into_iter().map(Into::into))
            .chain(self.utxos.into_iter().map(Into::into))
            .chain(self.confidential_outputs.into_iter().map(Into::into))
            .chain(self.published_template_addresses.into_iter().map(Into::into))
            .chain(self.validator_node_fee_pools.into_iter().map(Into::into))
    }

    pub fn bucket_ids(&self) -> &[BucketId] {
        &self.bucket_ids
    }

    pub fn proof_ids(&self) -> &[ProofId] {
        &self.proof_ids
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

    pub fn component_address_allocations(&self) -> &[ComponentAddressAllocation] {
        &self.component_address_allocations
    }

    pub fn resource_address_allocations(&self) -> &[ResourceAddressAllocation] {
        &self.resource_address_allocations
    }

    pub fn diff(&self, other: &Self) -> Self {
        Self {
            bucket_ids: diff_vec(&self.bucket_ids, &other.bucket_ids),
            proof_ids: diff_vec(&self.proof_ids, &other.proof_ids),
            component_addresses: diff_vec(&self.component_addresses, &other.component_addresses),
            resource_addresses: diff_vec(&self.resource_addresses, &other.resource_addresses),
            transaction_receipt_addresses: diff_vec(
                &self.transaction_receipt_addresses,
                &other.transaction_receipt_addresses,
            ),
            non_fungible_addresses: diff_vec(&self.non_fungible_addresses, &other.non_fungible_addresses),
            vault_ids: diff_vec(&self.vault_ids, &other.vault_ids),
            unclaimed_confidential_output_address: diff_vec(
                &self.unclaimed_confidential_output_address,
                &other.unclaimed_confidential_output_address,
            ),
            published_template_addresses: diff_vec(
                &self.published_template_addresses,
                &other.published_template_addresses,
            ),
            validator_node_fee_pools: diff_vec(&self.validator_node_fee_pools, &other.validator_node_fee_pools),
            utxos: diff_vec(&self.utxos, &other.utxos),
            confidential_outputs: diff_vec(&self.confidential_outputs, &other.confidential_outputs),
            component_address_allocations: diff_vec(
                &self.component_address_allocations,
                &other.component_address_allocations,
            ),
            resource_address_allocations: diff_vec(
                &self.resource_address_allocations,
                &other.resource_address_allocations,
            ),
        }
    }
}

fn diff_vec<T: Eq + Hash + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let b = b.iter().collect::<HashSet<_>>();
    a.iter().filter(|x| !b.contains(x)).cloned().collect()
}

impl FromIterator<IndexedWellKnownTypes> for IndexedWellKnownTypes {
    fn from_iter<T: IntoIterator<Item = IndexedWellKnownTypes>>(iter: T) -> Self {
        let mut indexed = Self::default();
        for value in iter {
            indexed.bucket_ids.extend(value.bucket_ids);
            indexed.proof_ids.extend(value.proof_ids);
            indexed.component_addresses.extend(value.component_addresses);
            indexed.resource_addresses.extend(value.resource_addresses);
            indexed
                .transaction_receipt_addresses
                .extend(value.transaction_receipt_addresses);
            indexed.non_fungible_addresses.extend(value.non_fungible_addresses);
            indexed.vault_ids.extend(value.vault_ids);
            indexed
                .unclaimed_confidential_output_address
                .extend(value.unclaimed_confidential_output_address);
            indexed
                .published_template_addresses
                .extend(value.published_template_addresses);
            indexed.validator_node_fee_pools.extend(value.validator_node_fee_pools);
            indexed.utxos.extend(value.utxos);
            indexed
                .component_address_allocations
                .extend(value.component_address_allocations);
            indexed
                .resource_address_allocations
                .extend(value.resource_address_allocations);
            indexed.confidential_outputs.extend(value.confidential_outputs);
        }
        indexed
    }
}

pub enum WellKnownTariValue {
    ComponentAddress(ComponentAddress),
    ResourceAddress(ResourceAddress),
    TransactionReceiptAddress(TransactionReceiptAddress),
    NonFungibleAddress(NonFungibleAddress),
    BucketId(BucketId),
    VaultId(VaultId),
    ProofId(ProofId),
    ClaimedOutputTombstoneAddress(ClaimedOutputTombstoneAddress),
    PublishedTemplateAddress(PublishedTemplateAddress),
    ValidatorNodeFeePool(ValidatorFeePoolAddress),
    ComponentAddressAllocation(ComponentAddressAllocation),
    ResourceAddressAllocation(ResourceAddressAllocation),
    Utxo(UtxoAddress),
    ConfidentialOutput(ConfidentialOutputAddress),
}

impl FromTagAndValue for WellKnownTariValue {
    type Error = IndexedValueError;

    fn try_from_tag_and_value(tag: u64, value: &tari_bor::Value) -> Result<Option<Self>, Self::Error>
    where Self: Sized {
        // Standard CBOR tags share the value tree with Tari's: an `Amount` above `u64::MAX` encodes
        // as a bignum, tag 2. Those carry no substate reference, so they are not this type's to
        // claim.
        let Some(tag) = BinaryTag::from_u64(tag) else {
            return Ok(None);
        };
        match tag {
            BinaryTag::ComponentAddress => {
                let component_address: ObjectKey = value.decoded()?;
                Ok(Some(Self::ComponentAddress(component_address.into())))
            },
            BinaryTag::BucketId => {
                let bucket_id: u32 = value.decoded()?;
                Ok(Some(Self::BucketId(bucket_id.into())))
            },
            BinaryTag::ResourceAddress => {
                let resource_address: ObjectKey = value.decoded()?;
                Ok(Some(Self::ResourceAddress(resource_address.into())))
            },
            BinaryTag::TransactionReceipt => {
                let tx_receipt_hash: Hash32 = value.decoded()?;
                Ok(Some(Self::TransactionReceiptAddress(tx_receipt_hash.into())))
            },
            BinaryTag::NonFungibleAddress => {
                let non_fungible_address: NonFungibleAddressContents = value.decoded()?;
                Ok(Some(Self::NonFungibleAddress(non_fungible_address.into())))
            },
            // Metadata values are arbitrary CBOR that may hold any of the types above, so the walker
            // descends into it like any other value.
            BinaryTag::Metadata => Ok(None),
            BinaryTag::VaultId => {
                let vault_id: ObjectKey = value.decoded()?;
                Ok(Some(Self::VaultId(vault_id.into())))
            },
            BinaryTag::ProofId => {
                let value: u32 = value.decoded()?;
                Ok(Some(Self::ProofId(value.into())))
            },
            BinaryTag::ClaimedOutputTombstoneAddress => {
                let value: ObjectKey = value.decoded()?;
                Ok(Some(Self::ClaimedOutputTombstoneAddress(value.into())))
            },
            BinaryTag::TemplateAddress => {
                let value: Hash32 = value.decoded()?;
                Ok(Some(Self::PublishedTemplateAddress(value.into())))
            },
            BinaryTag::ValidatorNodeFeePool => {
                let value: ObjectKey = value.decoded()?;
                Ok(Some(Self::ValidatorNodeFeePool(value.into_array().into())))
            },
            BinaryTag::AllocatedComponentAddress => {
                let value = value.decoded()?;
                Ok(Some(Self::ComponentAddressAllocation(ComponentAddressAllocation::new(
                    value,
                ))))
            },
            BinaryTag::AllocatedResourceAddress => {
                let value = value.decoded()?;
                Ok(Some(Self::ResourceAddressAllocation(ResourceAddressAllocation::new(
                    value,
                ))))
            },
            BinaryTag::Utxo => {
                let value: UtxoAddressContents = value.decoded()?;
                Ok(Some(Self::Utxo(value.into())))
            },
            BinaryTag::ConfidentialOutput => {
                let value: ConfidentialOutputAddressContents = value.decoded()?;
                Ok(Some(Self::ConfidentialOutput(value.into())))
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IndexedValueVisitor {
    buckets: Vec<BucketId>,
    proofs: Vec<ProofId>,
    component_addresses: Vec<ComponentAddress>,
    resource_addresses: Vec<ResourceAddress>,
    transaction_receipt_addresses: Vec<TransactionReceiptAddress>,
    non_fungible_addresses: Vec<NonFungibleAddress>,
    vault_ids: Vec<VaultId>,
    unclaimed_confidential_output_addresses: Vec<ClaimedOutputTombstoneAddress>,
    published_templates: Vec<PublishedTemplateAddress>,
    validator_node_fee_pools: Vec<ValidatorFeePoolAddress>,
    utxos: Vec<UtxoAddress>,
    confidential_outputs: Vec<ConfidentialOutputAddress>,
    component_address_allocations: Vec<ComponentAddressAllocation>,
    resource_address_allocations: Vec<ResourceAddressAllocation>,
}

impl IndexedValueVisitor {
    pub fn new() -> Self {
        Self {
            buckets: vec![],
            proofs: vec![],
            component_addresses: vec![],
            resource_addresses: vec![],
            transaction_receipt_addresses: vec![],
            non_fungible_addresses: vec![],
            vault_ids: vec![],
            unclaimed_confidential_output_addresses: vec![],
            published_templates: vec![],
            validator_node_fee_pools: vec![],
            utxos: vec![],
            confidential_outputs: vec![],
            component_address_allocations: vec![],
            resource_address_allocations: vec![],
        }
    }
}

impl ValueVisitor<WellKnownTariValue> for IndexedValueVisitor {
    type Error = IndexedValueError;

    fn visit(&mut self, value: WellKnownTariValue) -> Result<ControlFlow<()>, Self::Error> {
        match value {
            WellKnownTariValue::ComponentAddress(address) => {
                self.component_addresses.push(address);
            },
            WellKnownTariValue::ResourceAddress(address) => {
                self.resource_addresses.push(address);
            },
            WellKnownTariValue::TransactionReceiptAddress(address) => {
                self.transaction_receipt_addresses.push(address);
            },
            WellKnownTariValue::BucketId(bucket_id) => {
                self.buckets.push(bucket_id);
            },
            WellKnownTariValue::NonFungibleAddress(address) => {
                self.non_fungible_addresses.push(address);
            },
            WellKnownTariValue::VaultId(vault_id) => {
                self.vault_ids.push(vault_id);
            },
            WellKnownTariValue::ProofId(proof_id) => {
                self.proofs.push(proof_id);
            },
            WellKnownTariValue::ClaimedOutputTombstoneAddress(address) => {
                self.unclaimed_confidential_output_addresses.push(address);
            },
            WellKnownTariValue::PublishedTemplateAddress(template) => {
                self.published_templates.push(template);
            },
            WellKnownTariValue::ValidatorNodeFeePool(address) => {
                self.validator_node_fee_pools.push(address);
            },
            WellKnownTariValue::ComponentAddressAllocation(allocation) => {
                self.component_address_allocations.push(allocation);
            },
            WellKnownTariValue::ResourceAddressAllocation(allocation) => {
                self.resource_address_allocations.push(allocation);
            },
            WellKnownTariValue::Utxo(utxo) => {
                self.utxos.push(utxo);
            },
            WellKnownTariValue::ConfidentialOutput(output) => {
                self.confidential_outputs.push(output);
            },
        }
        Ok(ControlFlow::Continue(()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IndexedValueError {
    #[error("Bor error: {0}")]
    BorError(#[from] BorError),
    #[error("{0}")]
    Custom(String),
}

impl From<&str> for IndexedValueError {
    fn from(s: &str) -> Self {
        Self::Custom(s.to_string())
    }
}

pub fn decode_value_at_path<T>(value: &tari_bor::Value, path: &str) -> Result<Option<T>, IndexedValueError>
where T: for<'b> tari_bor::Decode<'b, ()> {
    get_value_by_path(value, path)
        .map(tari_bor::from_value)
        .transpose()
        .map_err(Into::into)
}

fn get_value_by_path<'a>(value: &'a tari_bor::Value, path: &str) -> Option<&'a tari_bor::Value> {
    let mut value = value;
    for part in path.split('.') {
        if part == "$" {
            continue;
        }
        match value {
            tari_bor::Value::Map(map) => {
                value = &map
                    .iter()
                    .find(|(k, _)| k.as_text().map(|s| s == part).unwrap_or(false))?
                    .1;
            },
            tari_bor::Value::Array(list) => {
                // With minicbor's integer-tagged encoding, struct fields land in an Array indexed by `#[n(N)]`.
                // Non-numeric path segments simply have no match (rather than panicking) so callers using
                // legacy string-keyed paths get a clean None instead of a crash.
                let index: usize = part.parse().ok()?;
                value = list.get(index)?;
            },
            _ => return None,
        }
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rand::Rng;
    use tari_bor::cbor;
    use tari_template_lib::types::{Metadata, NonFungibleId, UtxoId, crypto::PedersenCommitmentBytes, metadata};

    use super::*;
    use crate::hashing::{EngineHashDomainLabel, hasher32};

    fn new_object_key() -> ObjectKey {
        hasher32(EngineHashDomainLabel::ComponentAddress)
            .chain(&rand::rng().next_u32())
            .result()
            .trailing_bytes()
            .into()
    }

    #[derive(Serialize, Deserialize, minicbor::Encode, minicbor::Decode)]
    struct SubStruct {
        #[n(0)]
        buckets: Vec<BucketId>,
    }

    #[derive(Serialize, Deserialize, minicbor::Encode, minicbor::Decode)]
    struct TestStruct {
        #[n(0)]
        name: String,
        #[n(1)]
        component: ComponentAddress,
        #[n(2)]
        components: Vec<ComponentAddress>,
        #[n(3)]
        resource_map: HashMap<ResourceAddress, ComponentAddress>,
        #[n(4)]
        sub_struct: SubStruct,
        #[n(5)]
        sub_structs: Vec<SubStruct>,
        #[n(6)]
        vault_ids: Vec<VaultId>,
        #[n(7)]
        non_fungible_id: Option<NonFungibleAddress>,
        #[n(8)]
        metadata: Metadata,
    }

    #[test]
    fn it_returns_empty_indexed_value_for_empty_bytes() {
        let value = IndexedValue::from_raw(&[]).unwrap();
        assert_eq!(value, IndexedValue::default());
    }

    #[test]
    fn it_extracts_known_types_from_binary_data() {
        let addrs: [ComponentAddress; 3] = [
            new_object_key().into(),
            new_object_key().into(),
            new_object_key().into(),
        ];
        let resx_addr = ResourceAddress::new(new_object_key());

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
            vault_ids: vec![VaultId::new(new_object_key())],
            non_fungible_id: Some(NonFungibleAddress::new(resx_addr, NonFungibleId::Uint64(1))),
            metadata: metadata!("owner" => addrs[0]),
        };

        let value = tari_bor::to_value(&data).unwrap();
        let indexed = IndexedValue::from_value(value).unwrap();

        assert!(indexed.component_addresses().contains(&addrs[0]));
        assert!(indexed.component_addresses().contains(&addrs[1]));
        assert!(indexed.component_addresses().contains(&addrs[2]));
        // addrs[0] a second time, from the metadata
        assert_eq!(indexed.component_addresses().len(), 4);
        assert_eq!(indexed.resource_addresses().len(), 1);

        assert_eq!(indexed.non_fungible_addresses().len(), 1);
        assert_eq!(indexed.vault_ids().len(), 1);

        assert!(indexed.bucket_ids().contains(&1.into()));
        assert!(indexed.bucket_ids().contains(&2.into()));
        assert_eq!(indexed.bucket_ids().len(), 6);

        // TestStruct.sub_structs = #[n(5)], Vec index 1, then SubStruct.buckets = #[n(0)]
        let buckets: Vec<BucketId> = indexed.get_value("$.5.1.0").unwrap().unwrap();
        assert_eq!(buckets, vec![1.into(), 2.into()]);
    }

    /// An `Amount` above `u64::MAX` encodes as a CBOR bignum, which is a tagged value the indexer
    /// has no claim on. It must walk past it rather than reject the whole value.
    #[test]
    fn a_standard_cbor_tag_is_walked_past() {
        use tari_template_lib::types::Amount;

        let huge = Amount::new(u128::from(u64::MAX) + 1);
        let value = tari_bor::to_value(&huge).unwrap();
        assert!(
            matches!(value, tari_bor::Value::Tag(2, _)),
            "the test needs a bignum to be meaningful: {value:?}"
        );

        let indexed = IndexedValue::from_value(value).unwrap();
        assert!(indexed.component_addresses().is_empty());
    }

    /// Walking past an unclaimed tag must still descend into it, or a substate reference nested
    /// under one would go unlocked.
    #[test]
    fn a_reference_under_an_unclaimed_tag_is_still_found() {
        let addr = ComponentAddress::new(new_object_key());
        // Tag 42 is not a `BinaryTag`, so nothing claims it.
        let value = tari_bor::Value::Tag(42, Box::new(tari_bor::to_value(&addr).unwrap()));

        let indexed = IndexedValue::from_value(value).unwrap();
        assert!(indexed.component_addresses().contains(&addr));
    }

    /// A transaction's inputs are detected from `IndexedValue::referenced_substates`, and the engine rejects an
    /// argument referencing any substate kind that is not an input, so every kind it indexes must be listed.
    #[test]
    fn it_lists_every_referenced_substate_kind() {
        let receipt = TransactionReceiptAddress::from_array([1; 32]);
        let template = PublishedTemplateAddress::from_hash(Hash32::from_array([2; 32]));
        let fee_pool = ValidatorFeePoolAddress::from_array([3; 32]);
        let tombstone = ClaimedOutputTombstoneAddress::new(ObjectKey::from_array([4; 32]));
        let resource = ResourceAddress::new(new_object_key());
        let utxo = UtxoAddress::new(resource, UtxoId::from_array([5; 32]));
        let confidential_output =
            ConfidentialOutputAddress::new(resource, PedersenCommitmentBytes::from_array([6; 32]));
        let value = cbor!([
            tari_bor::to_value(&receipt).unwrap(),
            tari_bor::to_value(&template).unwrap(),
            tari_bor::to_value(&fee_pool).unwrap(),
            tari_bor::to_value(&tombstone).unwrap(),
            tari_bor::to_value(&utxo).unwrap(),
            tari_bor::to_value(&confidential_output).unwrap(),
        ]);

        let indexed = IndexedValue::from_value(value).unwrap();
        let referenced = indexed.referenced_substates().collect::<Vec<_>>();

        assert_eq!(referenced.len(), 6, "{referenced:?}");
        assert!(referenced.contains(&SubstateId::TransactionReceipt(receipt)));
        assert!(referenced.contains(&SubstateId::Template(template)));
        assert!(referenced.contains(&SubstateId::ValidatorFeePool(fee_pool)));
        assert!(referenced.contains(&SubstateId::ClaimedOutputTombstone(tombstone)));
        assert!(referenced.contains(&SubstateId::Utxo(utxo)));
        assert!(referenced.contains(&SubstateId::ConfidentialOutput(confidential_output)));
    }

    #[test]
    fn it_diffs_two_indexed_values() {
        let v1 = IndexedWellKnownTypes::from_value(&cbor!({
            "bucket" => tari_bor::to_value(&BucketId::from(1)).unwrap(),
            "proof1" => tari_bor::to_value(&ProofId::from(1)).unwrap(),
            "proof2" => tari_bor::to_value(&ProofId::from(2)).unwrap(),
        }))
        .unwrap();
        let v2 = IndexedWellKnownTypes::from_value(&cbor!({
            "buckets" => tari_bor::to_value(&vec![BucketId::from(1), BucketId::from(2)]).unwrap(),
            "proofs" => tari_bor::to_value(&vec![ProofId::from(2), ProofId::from(3), ProofId::from(4)]).unwrap(),
        }))
        .unwrap();

        let diff = v2.diff(&v1);

        assert_eq!(diff.bucket_ids, [BucketId::from(2)]);
        assert_eq!(diff.proof_ids, [ProofId::from(3), ProofId::from(4)]);
    }

    #[test]
    fn diff_keeps_order_and_duplicates_of_new_values() {
        let [a, b, c] = [(); 3].map(|_| ResourceAddress::new(new_object_key()));
        let prev = IndexedWellKnownTypes::from_value(&tari_bor::to_value(&vec![a, a, b]).unwrap()).unwrap();
        let next = IndexedWellKnownTypes::from_value(&tari_bor::to_value(&vec![c, b, a, c, c]).unwrap()).unwrap();

        let diff = next.diff(&prev);

        assert_eq!(diff.resource_addresses, [c, c, c]);
        assert!(prev.diff(&next).resource_addresses.is_empty());
    }

    #[test]
    fn values_inside_metadata_are_indexed() {
        let vault_id = VaultId::new(new_object_key());
        let resource = ResourceAddress::new(new_object_key());
        let metadata = metadata!(
            "name" => "text is not indexed",
            "vault" => vault_id,
            "nested" => vec![resource],
            "bucket" => BucketId::from(7),
        );

        let indexed = IndexedValue::from_type(&metadata).unwrap();

        assert_eq!(indexed.vault_ids(), &[vault_id]);
        assert_eq!(indexed.resource_addresses(), &[resource]);
        assert_eq!(indexed.bucket_ids(), &[BucketId::from(7)]);
    }
}
