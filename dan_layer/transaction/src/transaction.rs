//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, ShardId};
use tari_engine_types::{
    hashing::{hasher, EngineHashDomainLabel},
    instruction::Instruction,
    serde_with,
    substate::SubstateAddress,
};
use tari_template_lib::Hash;

use crate::{builder::TransactionBuilder, transaction_id::TransactionId, TransactionSignature};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    id: TransactionId,
    fee_instructions: Vec<Instruction>,
    instructions: Vec<Instruction>,
    signature: TransactionSignature,

    // TODO: Ideally we should ensure uniqueness and ordering invariants for each set.
    /// Input objects that may be downed by this transaction
    inputs: Vec<ShardId>,
    /// Input objects that must exist but cannot be downed by this transaction
    input_refs: Vec<ShardId>,
    /// Output objects that will be created by this transaction
    outputs: Vec<ShardId>,
    /// Inputs filled by some authority. These are not part of the transaction hash. (TODO: Secure this somehow)
    filled_inputs: Vec<ShardId>,
}

impl Transaction {
    pub fn builder() -> TransactionBuilder {
        TransactionBuilder::new()
    }

    pub fn new(
        fee_instructions: Vec<Instruction>,
        instructions: Vec<Instruction>,
        signature: TransactionSignature,
        inputs: Vec<ShardId>,
        input_refs: Vec<ShardId>,
        outputs: Vec<ShardId>,
        filled_inputs: Vec<ShardId>,
    ) -> Self {
        let mut tx = Self {
            id: TransactionId::default(),
            fee_instructions,
            instructions,
            signature,
            inputs,
            input_refs,
            outputs,
            filled_inputs,
        };
        tx.id = TransactionId::new(tx.calculate_hash().into_array());
        tx
    }

    fn calculate_hash(&self) -> Hash {
        hasher(EngineHashDomainLabel::Transaction)
            .chain(&self.signature)
            .chain(&self.fee_instructions)
            .chain(&self.instructions)
            .chain(&self.inputs)
            .chain(&self.input_refs)
            .chain(&self.outputs)
            .result()
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

    pub fn involved_shards_iter(&self) -> impl Iterator<Item = &ShardId> + '_ {
        self.inputs()
            .iter()
            .chain(self.input_refs())
            .chain(self.outputs())
            .chain(self.filled_inputs())
    }

    pub fn num_involved_shards(&self) -> usize {
        self.inputs().len() + self.input_refs().len() + self.outputs().len() + self.filled_inputs().len()
    }

    pub fn input_refs(&self) -> &[ShardId] {
        &self.input_refs
    }

    pub fn inputs(&self) -> &[ShardId] {
        &self.inputs
    }

    /// Returns (fee instructions, instructions)
    pub fn into_instructions(self) -> (Vec<Instruction>, Vec<Instruction>) {
        (self.fee_instructions, self.instructions)
    }

    pub fn all_inputs_iter(&self) -> impl Iterator<Item = &ShardId> + '_ {
        self.inputs()
            .iter()
            .chain(self.input_refs())
            .chain(self.filled_inputs())
    }

    pub fn filled_inputs(&self) -> &[ShardId] {
        &self.filled_inputs
    }

    pub fn filled_inputs_mut(&mut self) -> &mut Vec<ShardId> {
        &mut self.filled_inputs
    }

    pub fn outputs(&self) -> &[ShardId] {
        &self.outputs
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
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct SubstateRequirement {
    #[serde(with = "serde_with::string")]
    address: SubstateAddress,
    version: Option<u32>,
}

impl SubstateRequirement {
    pub fn new(address: SubstateAddress, version: Option<u32>) -> Self {
        Self { address, version }
    }

    pub fn address(&self) -> &SubstateAddress {
        &self.address
    }

    pub fn version(&self) -> Option<u32> {
        self.version
    }
}

impl FromStr for SubstateRequirement {
    type Err = SubstateRequirementParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');

        // parse the substate address
        let address = parts
            .next()
            .ok_or_else(|| SubstateRequirementParseError(s.to_string()))?;
        let address = SubstateAddress::from_str(address).map_err(|_| SubstateRequirementParseError(s.to_string()))?;

        // parse the version (optional)
        let version = match parts.next() {
            Some(v) => {
                let parse_version = v.parse().map_err(|_| SubstateRequirementParseError(s.to_string()))?;
                Some(parse_version)
            },
            None => None,
        };

        Ok(Self { address, version })
    }
}

impl Display for SubstateRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.version {
            Some(v) => write!(f, "{}:{}", self.address, v),
            None => write!(f, "{}", self.address),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse substate requirement {0}")]
pub struct SubstateRequirementParseError(String);
