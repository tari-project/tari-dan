//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, SubstateAddress};
use tari_engine_types::{
    hashing::{hasher32, EngineHashDomainLabel},
    indexed_value::{IndexedValue, IndexedValueError},
    instruction::Instruction,
    serde_with,
    substate::SubstateId,
};
use tari_template_lib::{models::ComponentAddress, Hash};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{builder::TransactionBuilder, transaction_id::TransactionId, TransactionSignature};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Transaction {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    id: TransactionId,
    fee_instructions: Vec<Instruction>,
    instructions: Vec<Instruction>,
    signature: TransactionSignature,

    // TODO: Ideally we should ensure uniqueness and ordering invariants for each set.
    /// Input objects that may be downed by this transaction
    inputs: Vec<SubstateAddress>,
    /// Input objects that must exist but cannot be downed by this transaction
    input_refs: Vec<SubstateAddress>,
    /// Inputs filled by some authority. These are not part of the transaction hash nor the signature
    filled_inputs: Vec<SubstateAddress>,
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
        inputs: Vec<SubstateAddress>,
        input_refs: Vec<SubstateAddress>,
        filled_inputs: Vec<SubstateAddress>,
        min_epoch: Option<Epoch>,
        max_epoch: Option<Epoch>,
    ) -> Self {
        let mut tx = Self {
            id: TransactionId::default(),
            fee_instructions,
            instructions,
            signature,
            inputs,
            input_refs,
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
            .chain(&self.input_refs)
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

    pub fn involved_shards_iter(&self) -> impl Iterator<Item = &SubstateAddress> + '_ {
        self.all_inputs_iter()
    }

    pub fn num_involved_shards(&self) -> usize {
        self.inputs().len() + self.input_refs().len() + self.filled_inputs().len()
    }

    pub fn input_refs(&self) -> &[SubstateAddress] {
        &self.input_refs
    }

    pub fn inputs(&self) -> &[SubstateAddress] {
        &self.inputs
    }

    /// Returns (fee instructions, instructions)
    pub fn into_instructions(self) -> (Vec<Instruction>, Vec<Instruction>) {
        (self.fee_instructions, self.instructions)
    }

    pub fn all_inputs_iter(&self) -> impl Iterator<Item = &SubstateAddress> + '_ {
        self.inputs()
            .iter()
            .chain(self.input_refs())
            .chain(self.filled_inputs())
    }

    pub fn filled_inputs(&self) -> &[SubstateAddress] {
        &self.filled_inputs
    }

    pub fn filled_inputs_mut(&mut self) -> &mut Vec<SubstateAddress> {
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
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct SubstateRequirement {
    #[serde(with = "serde_with::string")]
    substate_id: SubstateId,
    version: Option<u32>,
}

impl SubstateRequirement {
    pub fn new(address: SubstateId, version: Option<u32>) -> Self {
        Self {
            substate_id: address,
            version,
        }
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    pub fn version(&self) -> Option<u32> {
        self.version
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

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse substate requirement {0}")]
pub struct SubstateRequirementParseError(String);
