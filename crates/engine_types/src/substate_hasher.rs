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

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub enum SubstateHashMessage<'a> {
    V0(SubstateValueHashMessageV0<'a>),
}

impl<'a> SubstateHashMessage<'a> {
    pub fn new(protocol_version: ProtocolVersion, value: &'a SubstateValue) -> Self {
        match protocol_version {
            // V1 changes the block header preimage only, so substates keep the V0 schema.
            ProtocolVersion::V0 | ProtocolVersion::V1 => Self::V0(value.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, borsh::BorshSerialize)]
pub enum SubstateValueHashMessageV0<'a> {
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

impl<'a> From<&'a SubstateValue> for SubstateValueHashMessageV0<'a> {
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
        BorshSerialize::serialize(&self.receipt.outcome, writer)?;
        BorshSerialize::serialize(&self.receipt.diff_summary, writer)?;
        BorshSerialize::serialize(&self.receipt.fee_withdrawals, writer)?;
        let events = hash(&self.receipt.events);
        BorshSerialize::serialize(&events, writer)?;
        BorshSerialize::serialize(&self.receipt.fee_receipt, writer)?;
        BorshSerialize::serialize(&self.receipt.epoch, writer)?;
        // Serialized in full: the commitment is already 32 bytes, so part-hashing it saves nothing.
        BorshSerialize::serialize(&self.receipt.intent_commitment, writer)?;
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
