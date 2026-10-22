//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use borsh::{BorshSerialize, io};
use tari_template_lib::types::Hash32;

use crate::{
    ProtocolVersion,
    component::Component,
    confidential::ClaimedOutputTombstone,
    confidential_output::ConfidentialOutput,
    hashing::{EngineHashDomainLabel, hasher32},
    non_fungible::NonFungibleContainer,
    published_template::PublishedTemplate,
    resource::Resource,
    substate::SubstateValue,
    transaction_receipt::TransactionReceipt,
    utxo::Utxo,
    validator_fee::ValidatorFeePool,
    vault::Vault,
};

/// The substate hash preimage, tagged with the protocol version it was written under so that no two versions
/// share a preimage.
#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub enum SubstateHashMessage<'a> {
    V0(SubstateValueHashMessage<'a>),
}

impl<'a> SubstateHashMessage<'a> {
    pub fn new(protocol_version: ProtocolVersion, value: &'a SubstateValue) -> Self {
        match protocol_version {
            ProtocolVersion::V0 => Self::V0(value.into()),
        }
    }
}

/// The per-type preimage. Variant order is consensus-bound: a new variant goes at the end.
#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub enum SubstateValueHashMessage<'a> {
    Component(ComponentHashMessage<'a>),
    Resource(ResourceHashMessage<'a>),
    Vault(VaultHashMessage<'a>),
    NonFungible(NonFungibleContainerHashMessage<'a>),
    ClaimedOutputTombstone(ClaimedOutputTombstoneHashMessage<'a>),
    TransactionReceipt(TransactionReceiptHashMessage<'a>),
    Template(PublishedTemplateHashMessage<'a>),
    ValidatorFeePool(ValidatorFeePoolHashMessage<'a>),
    Utxo(UtxoHashMessage<'a>),
    ConfidentialOutput(ConfidentialOutputHashMessage<'a>),
}

impl<'a> From<&'a SubstateValue> for SubstateValueHashMessage<'a> {
    fn from(value: &'a SubstateValue) -> Self {
        match value {
            SubstateValue::Component(component) => Self::Component(component.into()),
            SubstateValue::Resource(resource) => Self::Resource(resource.as_ref().into()),
            SubstateValue::Vault(vault) => Self::Vault(vault.into()),
            SubstateValue::NonFungible(nf) => Self::NonFungible(nf.into()),
            SubstateValue::ClaimedOutputTombstone(tombstone) => Self::ClaimedOutputTombstone(tombstone.into()),
            SubstateValue::TransactionReceipt(receipt) => Self::TransactionReceipt(receipt.into()),
            SubstateValue::Template(template) => Self::Template(template.into()),
            SubstateValue::ValidatorFeePool(pool) => Self::ValidatorFeePool(pool.into()),
            SubstateValue::Utxo(utxo) => Self::Utxo(utxo.into()),
            SubstateValue::ConfidentialOutput(output) => Self::ConfidentialOutput(output.into()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ComponentHashMessage<'a>(pub &'a Component);

impl<'a> From<&'a Component> for ComponentHashMessage<'a> {
    fn from(component: &'a Component) -> Self {
        Self(component)
    }
}

impl borsh::BorshSerialize for ComponentHashMessage<'_> {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        BorshSerialize::serialize(&self.0.header, writer)?;
        // Split the body hash so that the body could be pruned
        let body_hash = hash(&self.0.body);
        BorshSerialize::serialize(&body_hash, writer)?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub struct ResourceHashMessage<'a>(pub &'a Resource);

impl<'a> From<&'a Resource> for ResourceHashMessage<'a> {
    fn from(resource: &'a Resource) -> Self {
        Self(resource)
    }
}

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub struct VaultHashMessage<'a>(&'a Vault);

impl<'a> From<&'a Vault> for VaultHashMessage<'a> {
    fn from(vault: &'a Vault) -> Self {
        Self(vault)
    }
}

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub struct NonFungibleContainerHashMessage<'a>(&'a NonFungibleContainer);

impl<'a> From<&'a NonFungibleContainer> for NonFungibleContainerHashMessage<'a> {
    fn from(non_fungible: &'a NonFungibleContainer) -> Self {
        Self(non_fungible)
    }
}

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub struct ClaimedOutputTombstoneHashMessage<'a>(&'a ClaimedOutputTombstone);

impl<'a> From<&'a ClaimedOutputTombstone> for ClaimedOutputTombstoneHashMessage<'a> {
    fn from(tombstone: &'a ClaimedOutputTombstone) -> Self {
        Self(tombstone)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TransactionReceiptHashMessage<'a> {
    pub receipt: &'a TransactionReceipt,
}

impl<'a> From<&'a TransactionReceipt> for TransactionReceiptHashMessage<'a> {
    fn from(receipt: &'a TransactionReceipt) -> Self {
        Self { receipt }
    }
}

impl borsh::BorshSerialize for TransactionReceiptHashMessage<'_> {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        let receipt = self.receipt;
        BorshSerialize::serialize(&receipt.outcome, writer)?;
        BorshSerialize::serialize(&receipt.diff_summary, writer)?;
        BorshSerialize::serialize(&receipt.fee_withdrawals, writer)?;
        let events = hash(&receipt.events);
        BorshSerialize::serialize(&events, writer)?;
        BorshSerialize::serialize(&receipt.fee_receipt, writer)?;
        BorshSerialize::serialize(&receipt.epoch, writer)?;
        // Serialized in full: the commitment is already 32 bytes, so part-hashing it saves nothing.
        BorshSerialize::serialize(&receipt.intent_commitment, writer)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PublishedTemplateHashMessage<'a> {
    pub template: &'a PublishedTemplate,
}

impl<'a> From<&'a PublishedTemplate> for PublishedTemplateHashMessage<'a> {
    fn from(template: &'a PublishedTemplate) -> Self {
        Self { template }
    }
}

impl borsh::BorshSerialize for PublishedTemplateHashMessage<'_> {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        BorshSerialize::serialize(&self.template.template_name, writer)?;
        BorshSerialize::serialize(&self.template.at_epoch, writer)?;
        BorshSerialize::serialize(&self.template.author, writer)?;
        BorshSerialize::serialize(&self.template.metadata_hash, writer)?;

        let binary_hash = hash(&self.template.binary);
        BorshSerialize::serialize(&binary_hash, writer)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub struct ValidatorFeePoolHashMessage<'a>(&'a ValidatorFeePool);

impl<'a> From<&'a ValidatorFeePool> for ValidatorFeePoolHashMessage<'a> {
    fn from(pool: &'a ValidatorFeePool) -> Self {
        Self(pool)
    }
}

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub struct UtxoHashMessage<'a>(&'a Utxo);

impl<'a> From<&'a Utxo> for UtxoHashMessage<'a> {
    fn from(utxo: &'a Utxo) -> Self {
        Self(utxo)
    }
}

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub struct ConfidentialOutputHashMessage<'a>(&'a ConfidentialOutput);

impl<'a> From<&'a ConfidentialOutput> for ConfidentialOutputHashMessage<'a> {
    fn from(output: &'a ConfidentialOutput) -> Self {
        Self(output)
    }
}

fn hash<T: borsh::BorshSerialize>(value: &T) -> Hash32 {
    hasher32(EngineHashDomainLabel::SubstateValuePart)
        .chain(&value)
        .result()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use ootle_network::Network;
    use tari_template_lib::types::{
        AccessRule,
        Metadata,
        ObjectKey,
        ResourceAddress,
        ResourceType,
        SubstateOwnerRule,
        access_rules::{ResourceAccessRules, UpdateRule},
        crypto::RistrettoPublicKeyBytes,
    };

    use super::*;
    use crate::{
        Epoch,
        SubstateVersion,
        fees::{FeeBreakdown, FeeReceipt, FeeSource},
        resource_container::ResourceContainer,
        substate::hash_substate,
        transaction_receipt::{DiffSummary, FinalizeOutcome},
    };

    /// The non-fungible data rule is spelled out so that the pinned hash moves only with the preimage format.
    fn resource(auth_hook_updater: UpdateRule) -> SubstateValue {
        SubstateValue::Resource(Box::new(Resource::new(
            ResourceType::Fungible,
            SubstateOwnerRule::None,
            ResourceAccessRules::new()
                .update_non_fungible_data(AccessRule::AllowAll, UpdateRule::Owner)
                .set_auth_hook_updater(auth_hook_updater),
            Metadata::from_iter([("name", "baseline")]),
            None,
            None,
            6,
            true,
        )))
    }

    fn receipt(exhaust_burn: u64) -> SubstateValue {
        let mut breakdown = FeeBreakdown::default();
        breakdown.add(FeeSource::Initial, 10);
        breakdown.add(FeeSource::Storage, 30);
        breakdown.add(FeeSource::WasmExecution, 20);
        let fee_receipt = FeeReceipt::builder()
            .with_total_fee_payment(1000)
            .with_total_fees_paid(60)
            .with_total_fee_overcharge(0)
            .with_cost_breakdown(breakdown)
            .with_exhaust_burn(exhaust_burn)
            .build();
        SubstateValue::TransactionReceipt(TransactionReceipt {
            outcome: FinalizeOutcome::Commit,
            diff_summary: DiffSummary {
                upped: Box::new([]),
                downed: Box::new([]),
            },
            fee_withdrawals: Box::new([]),
            events: Box::new([]),
            fee_receipt,
            epoch: Epoch(3),
            intent_commitment: Hash32::from_array([7u8; 32]),
        })
    }

    fn hash_at(version: ProtocolVersion, value: &SubstateValue) -> Hash32 {
        hasher32(EngineHashDomainLabel::SubstateValue)
            .chain(&SubstateHashMessage::new(version, value))
            .chain(&0u64)
            .chain(&Epoch(3))
            .result()
    }

    fn hex(hash: Hash32) -> String {
        hex::encode(hash.as_ref() as &[u8])
    }

    /// Pins the version 0 preimage for a receipt. Every substate value hash, and so every state root,
    /// is derived from it: a change to this hash is a change to all of them, and nodes carrying state
    /// hashed under the old preimage cannot re-derive their roots.
    #[test]
    fn version_0_receipt_hash_is_pinned() {
        assert_eq!(
            hex(hash_at(ProtocolVersion::V0, &receipt(123))),
            "3ed616d2ebdc09d0bf480368e2a0812b358f8c16bfad772c69baf565c77bbd89"
        );
        assert_eq!(
            hash_substate(Network::Esmeralda, &receipt(123), SubstateVersion::ZERO, Epoch(3)),
            hash_at(ProtocolVersion::V0, &receipt(123))
        );
    }

    /// Pins the version 0 preimage for a resource, as [`version_0_receipt_hash_is_pinned`] does for a
    /// receipt.
    #[test]
    fn version_0_resource_hash_is_pinned() {
        assert_eq!(
            hex(hash_at(ProtocolVersion::V0, &resource(UpdateRule::Locked))),
            "30d55dd7f25674254559dbcb8af6130df44501f1c3b76f790e72d23aa0a9e840"
        );
    }

    #[test]
    fn version_0_covers_auth_hook_updater() {
        assert_ne!(
            hash_at(ProtocolVersion::V0, &resource(UpdateRule::Locked)),
            hash_at(ProtocolVersion::V0, &resource(UpdateRule::Owner))
        );
    }

    #[test]
    fn version_0_covers_exhaust_burn() {
        assert_ne!(
            hash_at(ProtocolVersion::V0, &receipt(0)),
            hash_at(ProtocolVersion::V0, &receipt(123))
        );
    }

    /// Pins the borsh tag of every cheaply constructed variant, which pins the rest transitively: a
    /// variant inserted anywhere but after `Utxo` shifts one of these tags. `ConfidentialOutput` is
    /// last, where an append is the only move and is safe. The two leading bytes are the
    /// `SubstateHashMessage` version tag and the value tag.
    #[test]
    fn value_tags_are_stable() {
        let values: Vec<(u8, SubstateValue)> = vec![
            (
                2,
                SubstateValue::Vault(Vault::new(ResourceContainer::non_fungible(
                    ResourceAddress::new(ObjectKey::from_array([1u8; ObjectKey::LENGTH])),
                    BTreeSet::new(),
                ))),
            ),
            (
                3,
                SubstateValue::NonFungible(NonFungibleContainer::new(tari_bor::Value::Null, tari_bor::Value::Null)),
            ),
            (
                4,
                SubstateValue::ClaimedOutputTombstone(ClaimedOutputTombstone { value: 1 }),
            ),
            (1, resource(UpdateRule::Locked)),
            (5, receipt(0)),
            (
                7,
                SubstateValue::ValidatorFeePool(ValidatorFeePool::new(RistrettoPublicKeyBytes::default(), 5)),
            ),
            (
                8,
                SubstateValue::Utxo(Utxo {
                    output: None,
                    is_frozen: false,
                }),
            ),
        ];
        for (tag, value) in &values {
            let v0 = borsh::to_vec(&SubstateHashMessage::new(ProtocolVersion::V0, value)).unwrap();
            assert_eq!(&v0[..2], &[0, *tag], "V0 tag for {value:?}");
        }
    }
}
