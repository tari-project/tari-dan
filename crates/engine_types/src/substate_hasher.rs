//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use borsh::{BorshSerialize, io};
use tari_template_lib::types::Hash32;

use crate::{
    ProtocolVersion,
    component::Component,
    confidential::ClaimedOutputTombstone,
    confidential_output::ConfidentialOutput,
    fees::FeeReceipt,
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

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub enum SubstateHashMessage<'a> {
    V0(SubstateValueHashMessageV0<'a>),
    V1(SubstateValueHashMessageV1<'a>),
}

impl<'a> SubstateHashMessage<'a> {
    pub fn new(protocol_version: ProtocolVersion, value: &'a SubstateValue) -> Self {
        match protocol_version {
            ProtocolVersion::V0 => Self::V0(value.into()),
            ProtocolVersion::V1 => Self::V1(value.into()),
        }
    }
}

pub type SubstateValueHashMessageV0<'a> =
    SubstateValueHashMessage<'a, ResourceHashMessageV0<'a>, TransactionReceiptHashMessageV0<'a>>;

/// Version 1 changes the transaction receipt's preimage, which covers `FeeReceipt::exhaust_burn`, and the
/// resource's, which covers `ResourceAccessRules::auth_hook_updater` - and, through the leading
/// `SubstateHashMessage` tag, the preimage of every substate.
pub type SubstateValueHashMessageV1<'a> =
    SubstateValueHashMessage<'a, ResourceHashMessageV1<'a>, TransactionReceiptHashMessageV1<'a>>;

/// The per-type preimage, shared by every protocol version so that the borsh tag of each variant is
/// the same under all of them. Variant order is consensus-bound: a new variant goes at the end.
#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub enum SubstateValueHashMessage<'a, Res, Rec> {
    Component(ComponentHashMessage<'a>),
    Resource(Res),
    Vault(VaultHashMessage<'a>),
    NonFungible(NonFungibleContainerHashMessage<'a>),
    ClaimedOutputTombstone(ClaimedOutputTombstoneHashMessage<'a>),
    TransactionReceipt(Rec),
    Template(PublishedTemplateHashMessage<'a>),
    ValidatorFeePool(ValidatorFeePoolHashMessage<'a>),
    Utxo(UtxoHashMessage<'a>),
    ConfidentialOutput(ConfidentialOutputHashMessage<'a>),
}

impl<'a, Res: From<&'a Resource>, Rec: From<&'a TransactionReceipt>> From<&'a SubstateValue>
    for SubstateValueHashMessage<'a, Res, Rec>
{
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

/// The resource preimage under protocol version 0. `ResourceAccessRules` is written without
/// `auth_hook_updater`, the shape every version 0 resource was hashed with.
#[derive(Debug, Clone, Copy)]
pub struct ResourceHashMessageV0<'a>(pub &'a Resource);

impl<'a> From<&'a Resource> for ResourceHashMessageV0<'a> {
    fn from(resource: &'a Resource) -> Self {
        Self(resource)
    }
}

impl borsh::BorshSerialize for ResourceHashMessageV0<'_> {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        self.0.borsh_serialize_v0(writer)
    }
}

/// The resource preimage from protocol version 1: the whole `Resource`, `auth_hook_updater` included.
#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub struct ResourceHashMessageV1<'a>(pub &'a Resource);

impl<'a> From<&'a Resource> for ResourceHashMessageV1<'a> {
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

/// The receipt preimage under protocol version 0. `FeeReceipt` is written without `exhaust_burn`,
/// the shape every version 0 receipt was hashed with.
#[derive(Debug, Clone, Copy)]
pub struct TransactionReceiptHashMessageV0<'a> {
    pub receipt: &'a TransactionReceipt,
}

impl<'a> From<&'a TransactionReceipt> for TransactionReceiptHashMessageV0<'a> {
    fn from(receipt: &'a TransactionReceipt) -> Self {
        Self { receipt }
    }
}

impl borsh::BorshSerialize for TransactionReceiptHashMessageV0<'_> {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        serialize_receipt(self.receipt, writer, |fee_receipt, writer| {
            fee_receipt.borsh_serialize_v0(writer)
        })
    }
}

/// The receipt preimage from protocol version 1: the whole `FeeReceipt`, `exhaust_burn` included.
#[derive(Debug, Clone, Copy)]
pub struct TransactionReceiptHashMessageV1<'a> {
    pub receipt: &'a TransactionReceipt,
}

impl<'a> From<&'a TransactionReceipt> for TransactionReceiptHashMessageV1<'a> {
    fn from(receipt: &'a TransactionReceipt) -> Self {
        Self { receipt }
    }
}

impl borsh::BorshSerialize for TransactionReceiptHashMessageV1<'_> {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        serialize_receipt(self.receipt, writer, |fee_receipt, writer| {
            BorshSerialize::serialize(fee_receipt, writer)
        })
    }
}

fn serialize_receipt<W: io::Write>(
    receipt: &TransactionReceipt,
    writer: &mut W,
    serialize_fee_receipt: impl FnOnce(&FeeReceipt, &mut W) -> io::Result<()>,
) -> io::Result<()> {
    BorshSerialize::serialize(&receipt.outcome, writer)?;
    BorshSerialize::serialize(&receipt.diff_summary, writer)?;
    BorshSerialize::serialize(&receipt.fee_withdrawals, writer)?;
    let events = hash(&receipt.events);
    BorshSerialize::serialize(&events, writer)?;
    serialize_fee_receipt(&receipt.fee_receipt, writer)?;
    BorshSerialize::serialize(&receipt.epoch, writer)?;
    // Serialized in full: the commitment is already 32 bytes, so part-hashing it saves nothing.
    BorshSerialize::serialize(&receipt.intent_commitment, writer)?;
    Ok(())
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
        fees::{FeeBreakdown, FeeSource},
        resource_container::ResourceContainer,
        substate::hash_substate,
        transaction_receipt::{DiffSummary, FinalizeOutcome},
    };

    fn resource(auth_hook_updater: UpdateRule) -> SubstateValue {
        SubstateValue::Resource(Box::new(Resource::new(
            ResourceType::Fungible,
            SubstateOwnerRule::None,
            ResourceAccessRules::new().set_auth_hook_updater(auth_hook_updater),
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
            diff_summary: DiffSummary { upped: Box::new([]) },
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
            .chain(&0u32)
            .chain(&Epoch(3))
            .result()
    }

    fn hex(hash: Hash32) -> String {
        hex::encode(hash.as_ref() as &[u8])
    }

    /// The hash a binary that predates `FeeReceipt::exhaust_burn` produced for this receipt (captured
    /// from `hash_substate` at 2cc729b95). Version 0 must keep producing it, or no node can re-derive
    /// the state roots that committed such receipts.
    #[test]
    fn version_0_reproduces_the_pre_exhaust_burn_hash() {
        assert_eq!(
            hex(hash_at(ProtocolVersion::V0, &receipt(0))),
            "061d838f149c767043152d6362afd71f18d58ac41c1c7b0a7f130066e9e1efcb"
        );
        // Esmeralda is the network whose history was hashed under version 0.
        assert_eq!(ProtocolVersion::at(Network::Esmeralda, Epoch(3)), ProtocolVersion::V0);
        assert_eq!(
            hash_substate(Network::Esmeralda, &receipt(0), 0, Epoch(3)),
            hash_at(ProtocolVersion::V0, &receipt(0))
        );
    }

    /// Networks launched at version 1 commit to this from their first block.
    #[test]
    fn version_1_hash_is_pinned() {
        assert_eq!(
            hex(hash_at(ProtocolVersion::V1, &receipt(123))),
            "78a85877d39682b55d299b5c3fad89b4261603bc2e261f5af728caa876fbbc78"
        );
    }

    /// The hash a binary that predates `ResourceAccessRules::auth_hook_updater` produced for this
    /// resource (captured from `hash_substate` at 4da937665). Version 0 must keep producing it, or no
    /// node can re-derive the state roots that committed such resources.
    #[test]
    fn version_0_reproduces_the_pre_auth_hook_updater_hash() {
        assert_eq!(
            hex(hash_at(ProtocolVersion::V0, &resource(UpdateRule::Locked))),
            "e70d065a427a57fb18d8d03ff858411b7c542081863042b0753b2459759ecead"
        );
    }

    #[test]
    fn version_0_does_not_cover_auth_hook_updater() {
        assert_eq!(
            hash_at(ProtocolVersion::V0, &resource(UpdateRule::Locked)),
            hash_at(ProtocolVersion::V0, &resource(UpdateRule::Owner))
        );
    }

    #[test]
    fn version_1_covers_auth_hook_updater() {
        assert_ne!(
            hash_at(ProtocolVersion::V1, &resource(UpdateRule::Locked)),
            hash_at(ProtocolVersion::V1, &resource(UpdateRule::Owner))
        );
        // Past the leading version tag, so that wiring V1 to the version 0 preimage would fail here.
        let value = resource(UpdateRule::Locked);
        let v0 = borsh::to_vec(&SubstateHashMessage::new(ProtocolVersion::V0, &value)).unwrap();
        let v1 = borsh::to_vec(&SubstateHashMessage::new(ProtocolVersion::V1, &value)).unwrap();
        assert_ne!(v0[1..], v1[1..]);
    }

    #[test]
    fn version_0_does_not_cover_exhaust_burn() {
        assert_eq!(
            hash_at(ProtocolVersion::V0, &receipt(0)),
            hash_at(ProtocolVersion::V0, &receipt(123))
        );
    }

    #[test]
    fn version_1_covers_exhaust_burn() {
        assert_ne!(
            hash_at(ProtocolVersion::V1, &receipt(0)),
            hash_at(ProtocolVersion::V1, &receipt(123))
        );
        assert_ne!(
            hash_at(ProtocolVersion::V0, &receipt(0)),
            hash_at(ProtocolVersion::V1, &receipt(0))
        );
    }

    /// Pins the borsh tag of every cheaply constructed variant, which pins the rest transitively: a
    /// variant inserted anywhere but after `Utxo` shifts one of these tags. `ConfidentialOutput` is
    /// last, where an append is the only move and is safe. The two leading bytes are the
    /// `SubstateHashMessage` version tag and the value tag.
    #[test]
    fn value_tags_are_stable_and_shared_by_both_versions() {
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
            let v1 = borsh::to_vec(&SubstateHashMessage::new(ProtocolVersion::V1, value)).unwrap();
            assert_eq!(&v0[..2], &[0, *tag], "V0 tag for {value:?}");
            assert_eq!(&v1[..2], &[1, *tag], "V1 tag for {value:?}");
            // Receipts and resources carry a version-dependent preimage; every other value is written
            // identically under both versions.
            if value.as_transaction_receipt().is_none() && value.as_resource().is_none() {
                assert_eq!(
                    v0[1..],
                    v1[1..],
                    "V1 must differ from V0 only in the version tag for {value:?}"
                );
            }
        }
    }
}
