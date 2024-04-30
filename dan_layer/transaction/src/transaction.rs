//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, collections::HashSet, fmt::Display, str::FromStr};

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{shard::Shard, Epoch, SubstateAddress};
use tari_engine_types::{
    hashing::{hasher32, EngineHashDomainLabel},
    indexed_value::{IndexedValue, IndexedValueError},
    instruction::Instruction,
    serde_with,
    substate::SubstateId,
};
use tari_template_lib::{models::ComponentAddress, Hash};

use crate::{builder::TransactionBuilder, transaction_id::TransactionId, TransactionSignature};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct Transaction {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    id: TransactionId,
    fee_instructions: Vec<Instruction>,
    instructions: Vec<Instruction>,
    signature: TransactionSignature,

    // TODO: Ideally we should ensure uniqueness and ordering invariants for each set.
    /// Input objects that may be downed (write) or referenced (read) by this transaction.
    inputs: IndexSet<SubstateRequirement>,
    /// Inputs filled by some authority. These are not part of the transaction hash nor the signature
    filled_inputs: IndexSet<VersionedSubstateId>,
    min_epoch: Option<Epoch>,
    max_epoch: Option<Epoch>,
}

impl Transaction {
    pub fn builder() -> TransactionBuilder {
        TransactionBuilder::new()
    }

    pub fn new(
        fee_instructions: Vec<Instruction>,
        instructions: Vec<Instruction>,
        signature: TransactionSignature,
        inputs: IndexSet<SubstateRequirement>,
        filled_inputs: IndexSet<VersionedSubstateId>,
        min_epoch: Option<Epoch>,
        max_epoch: Option<Epoch>,
    ) -> Self {
        let mut tx = Self {
            id: TransactionId::default(),
            fee_instructions,
            instructions,
            signature,
            inputs,
            filled_inputs,
            min_epoch,
            max_epoch,
        };
        tx.id = tx.calculate_hash();
        tx
    }

    fn calculate_hash(&self) -> TransactionId {
        hasher32(EngineHashDomainLabel::Transaction)
            .chain(&self.signature)
            .chain(&self.fee_instructions)
            .chain(&self.instructions)
            .chain(&self.inputs)
            .chain(&self.min_epoch)
            .chain(&self.max_epoch)
            .result()
            .into_array()
            .into()
    }

    pub fn id(&self) -> &TransactionId {
        &self.id
    }

    pub fn hash(&self) -> Hash {
        self.id.into_array().into()
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        &self.fee_instructions
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub fn signature(&self) -> &TransactionSignature {
        &self.signature
    }

    pub fn signer_public_key(&self) -> &PublicKey {
        self.signature.public_key()
    }

    pub fn involved_shards_iter(&self) -> impl Iterator<Item = SubstateAddress> + '_ {
        self.all_input_addresses_iter()
    }

    pub fn inputs(&self) -> &IndexSet<SubstateRequirement> {
        &self.inputs
    }

    fn input_addresses_iter(&self) -> impl Iterator<Item = SubstateAddress> + '_ {
        self.inputs
            .iter()
            .filter_map(|i: &SubstateRequirement| i.to_substate_address())
    }

    /// Returns (fee instructions, instructions)
    pub fn into_instructions(self) -> (Vec<Instruction>, Vec<Instruction>) {
        (self.fee_instructions, self.instructions)
    }

    pub fn all_inputs_iter(&self) -> impl Iterator<Item = SubstateRequirement> + '_ {
        self.inputs()
            .iter()
            // Filled inputs override other inputs as they are likely filled with versions
            .filter(|i| self.filled_inputs().iter().all(|fi| fi.substate_id() != i.substate_id()))
            .cloned()
            .chain(self.filled_inputs().iter().cloned().map(Into::into))
    }

    pub fn all_inputs_substate_ids_iter(&self) -> impl Iterator<Item = &SubstateId> + '_ {
        self.inputs()
            .iter()
            // Filled inputs override other inputs as they are likely filled with versions
            .filter(|i| self.filled_inputs().iter().all(|fi| fi.substate_id() != i.substate_id()))
            .map(|i| i.substate_id())
            .chain(self.filled_inputs().iter().map(|fi| fi.substate_id()))
    }

    pub fn num_unique_inputs(&self) -> usize {
        self.all_inputs_substate_ids_iter().count()
    }

    pub fn all_input_addresses_iter(&self) -> impl Iterator<Item = SubstateAddress> + '_ {
        self.input_addresses_iter().chain(self.filled_input_addresses_iter())
    }

    pub fn filled_inputs(&self) -> &IndexSet<VersionedSubstateId> {
        &self.filled_inputs
    }

    fn filled_input_addresses_iter(&self) -> impl Iterator<Item = SubstateAddress> + '_ {
        self.filled_inputs.iter().map(|i| i.to_substate_address())
    }

    pub fn filled_inputs_mut(&mut self) -> &mut IndexSet<VersionedSubstateId> {
        &mut self.filled_inputs
    }

    pub fn fee_claims(&self) -> impl Iterator<Item = (Epoch, PublicKey)> + '_ {
        self.instructions()
            .iter()
            .chain(self.fee_instructions())
            .filter_map(|instruction| {
                if let Instruction::ClaimValidatorFees {
                    epoch,
                    validator_public_key,
                } = instruction
                {
                    Some((Epoch(*epoch), validator_public_key.clone()))
                } else {
                    None
                }
            })
    }

    pub fn min_epoch(&self) -> Option<Epoch> {
        self.min_epoch
    }

    pub fn max_epoch(&self) -> Option<Epoch> {
        self.max_epoch
    }

    pub fn as_referenced_components(&self) -> impl Iterator<Item = &ComponentAddress> + '_ {
        self.instructions()
            .iter()
            .chain(self.fee_instructions())
            .filter_map(|instruction| {
                if let Instruction::CallMethod { component_address, .. } = instruction {
                    Some(component_address)
                } else {
                    None
                }
            })
    }

    /// Returns all substates addresses referenced by this transaction
    pub fn to_referenced_substates(&self) -> Result<HashSet<SubstateId>, IndexedValueError> {
        let all_instructions = self.instructions().iter().chain(self.fee_instructions());

        let mut substates = HashSet::new();
        for instruction in all_instructions {
            match instruction {
                Instruction::CallFunction { args, .. } => {
                    for arg in args.iter().filter_map(|a| a.as_literal_bytes()) {
                        let value = IndexedValue::from_raw(arg)?;
                        substates.extend(value.referenced_substates().filter(|id| !id.is_virtual()));
                    }
                },
                Instruction::CallMethod {
                    component_address,
                    args,
                    ..
                } => {
                    substates.insert(SubstateId::Component(*component_address));
                    for arg in args.iter().filter_map(|a| a.as_literal_bytes()) {
                        let value = IndexedValue::from_raw(arg)?;
                        substates.extend(value.referenced_substates().filter(|id| !id.is_virtual()));
                    }
                },
                Instruction::ClaimBurn { claim } => {
                    substates.insert(SubstateId::UnclaimedConfidentialOutput(claim.output_address));
                },
                _ => {},
            }
        }
        Ok(substates)
    }

    pub fn has_inputs_without_version(&self) -> bool {
        self.inputs().iter().any(|i| i.version().is_none())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq, Hash)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct SubstateRequirement {
    #[serde(with = "serde_with::string")]
    pub substate_id: SubstateId,
    pub version: Option<u32>,
}

impl SubstateRequirement {
    pub fn new(address: SubstateId, version: Option<u32>) -> Self {
        Self {
            substate_id: address,
            version,
        }
    }

    pub fn with_version(address: SubstateId, version: u32) -> Self {
        Self {
            substate_id: address,
            version: Some(version),
        }
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    pub fn version(&self) -> Option<u32> {
        self.version
    }

    pub fn to_substate_address(&self) -> Option<SubstateAddress> {
        Some(SubstateAddress::from_address(self.substate_id(), self.version()?))
    }

    /// Calculates and returns the shard number that this SubstateAddress belongs.
    /// A shard is a division of the 256-bit shard space.
    /// If the substate version is not known, None is returned.
    pub fn to_committee_shard(&self, num_committees: u32) -> Option<Shard> {
        Some(self.to_substate_address()?.to_committee_shard(num_committees))
    }

    pub fn to_versioned(&self) -> Option<VersionedSubstateId> {
        self.version.map(|v| VersionedSubstateId {
            substate_id: self.substate_id.clone(),
            version: v,
        })
    }
}

impl FromStr for SubstateRequirement {
    type Err = SubstateRequirementParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');

        // parse the substate id
        let address = parts
            .next()
            .ok_or_else(|| SubstateRequirementParseError(s.to_string()))?;
        let address = SubstateId::from_str(address).map_err(|_| SubstateRequirementParseError(s.to_string()))?;

        // parse the version (optional)
        let version = match parts.next() {
            Some(v) => {
                let parse_version = v.parse().map_err(|_| SubstateRequirementParseError(s.to_string()))?;
                Some(parse_version)
            },
            None => None,
        };

        Ok(Self {
            substate_id: address,
            version,
        })
    }
}

impl Display for SubstateRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.version {
            Some(v) => write!(f, "{}:{}", self.substate_id, v),
            None => write!(f, "{}", self.substate_id),
        }
    }
}

impl From<VersionedSubstateId> for SubstateRequirement {
    fn from(value: VersionedSubstateId) -> Self {
        Self::with_version(value.substate_id, value.version)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse substate requirement {0}")]
pub struct SubstateRequirementParseError(String);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct VersionedSubstateId {
    #[serde(with = "serde_with::string")]
    pub substate_id: SubstateId,
    pub version: u32,
}

impl VersionedSubstateId {
    pub fn new(substate_id: SubstateId, version: u32) -> Self {
        Self { substate_id, version }
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_address(self.substate_id(), self.version())
    }

    /// Calculates and returns the shard number that this SubstateAddress belongs.
    /// A shard is an equal division of the 256-bit shard space.
    pub fn to_committee_shard(&self, num_committees: u32) -> Shard {
        self.to_substate_address().to_committee_shard(num_committees)
    }
}

impl FromStr for VersionedSubstateId {
    type Err = SubstateRequirementParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');

        // parse the substate id
        let address = parts
            .next()
            .ok_or_else(|| SubstateRequirementParseError(s.to_string()))?;
        let address = SubstateId::from_str(address).map_err(|_| SubstateRequirementParseError(s.to_string()))?;

        // parse the version
        let version = parts
            .next()
            .ok_or_else(|| SubstateRequirementParseError(s.to_string()))
            .and_then(|v| v.parse().map_err(|_| SubstateRequirementParseError(s.to_string())))?;

        Ok(Self {
            substate_id: address,
            version,
        })
    }
}

impl Display for VersionedSubstateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.substate_id, self.version)
    }
}

impl TryFrom<SubstateRequirement> for VersionedSubstateId {
    type Error = VersionedSubstateIdError;

    fn try_from(value: SubstateRequirement) -> Result<Self, Self::Error> {
        match value.version {
            Some(v) => Ok(Self::new(value.substate_id, v)),
            None => Err(VersionedSubstateIdError::SubstateRequirementNotVersioned(
                value.substate_id,
            )),
        }
    }
}

// Only consider the substate id in maps. This means that duplicates found if the substate id is the same regardless of
// the version.
impl std::hash::Hash for VersionedSubstateId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.substate_id.hash(state);
    }
}

impl PartialEq for VersionedSubstateId {
    fn eq(&self, other: &Self) -> bool {
        self.substate_id == other.substate_id
    }
}

impl Eq for VersionedSubstateId {}

impl Borrow<SubstateId> for VersionedSubstateId {
    fn borrow(&self) -> &SubstateId {
        &self.substate_id
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VersionedSubstateIdError {
    #[error("Substate requirement {0} is not versioned")]
    SubstateRequirementNotVersioned(SubstateId),
}

#[cfg(test)]
mod tests {
    use tari_template_lib::models::ObjectKey;

    use super::*;

    #[test]
    fn it_hashes_identically_to_a_substate_id() {
        let substate_id = SubstateId::Component(ComponentAddress::new(ObjectKey::default()));
        let mut set = IndexSet::new();
        set.extend([VersionedSubstateId::new(substate_id.clone(), 0)]);
        assert!(set.contains(&substate_id));
    }
}
