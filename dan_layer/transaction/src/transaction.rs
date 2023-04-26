//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::ShardId;
use tari_engine_types::{
    hashing::{hasher, EngineHashDomainLabel},
    instruction::Instruction,
};
use tari_template_lib::{
    models::{ComponentAddress, TemplateAddress},
    Hash,
};

use crate::{change::SubstateChange, InstructionSignature, TransactionBuilder};

#[derive(Debug, Clone)]
pub struct BalanceProof {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    hash: Hash,
    fee_instructions: Vec<Instruction>,
    instructions: Vec<Instruction>,
    signature: InstructionSignature,
    sender_public_key: PublicKey,
    // Not part of signature. TODO: Should it be?
    meta: TransactionMeta,
}

impl Transaction {
    pub fn builder() -> TransactionBuilder {
        TransactionBuilder::new()
    }

    pub fn new(
        fee_instructions: Vec<Instruction>,
        instructions: Vec<Instruction>,
        signature: InstructionSignature,
        sender_public_key: PublicKey,
        meta: TransactionMeta,
    ) -> Self {
        let mut s = Self {
            hash: Hash::default(),
            fee_instructions,
            instructions,
            signature,
            sender_public_key,
            meta,
        };
        s.hash = s.calculate_hash();
        s
    }

    /// Returns the template addresses that are statically known to be executed by this transaction.
    /// This does not include templates for component invocation as that data is not contained within the transaction.
    pub fn required_templates(&self) -> BTreeSet<TemplateAddress> {
        self.fee_instructions
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::CallFunction { template_address, .. } => Some(*template_address),
                _ => None,
            })
            .chain(self.instructions.iter().filter_map(|instruction| match instruction {
                Instruction::CallFunction { template_address, .. } => Some(*template_address),
                _ => None,
            }))
            .collect()
    }

    pub fn required_components(&self) -> BTreeSet<ComponentAddress> {
        self.fee_instructions
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::CallMethod { component_address, .. } => Some(*component_address),
                _ => None,
            })
            .chain(self.instructions.iter().filter_map(|instruction| match instruction {
                Instruction::CallMethod { component_address, .. } => Some(*component_address),
                _ => None,
            }))
            .collect()
    }

    pub fn hash(&self) -> &Hash {
        &self.hash
    }

    pub fn meta(&self) -> &TransactionMeta {
        &self.meta
    }

    pub fn meta_mut(&mut self) -> &mut TransactionMeta {
        &mut self.meta
    }

    fn calculate_hash(&self) -> Hash {
        hasher(EngineHashDomainLabel::Transaction)
            .chain(&self.sender_public_key)
            .chain(self.signature.signature().get_public_nonce())
            .chain(self.signature.signature().get_signature())
            .chain(&self.fee_instructions)
            .chain(&self.instructions)
            .result()
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        &self.fee_instructions
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub fn into_instructions(self) -> Vec<Instruction> {
        self.instructions
    }

    pub fn signature(&self) -> &InstructionSignature {
        &self.signature
    }

    pub fn sender_public_key(&self) -> &PublicKey {
        &self.sender_public_key
    }

    pub fn destruct(self) -> (Vec<Instruction>, Vec<Instruction>, InstructionSignature, PublicKey) {
        (
            self.instructions,
            self.fee_instructions,
            self.signature,
            self.sender_public_key,
        )
    }
}

impl PartialEq for Transaction {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}
impl Eq for Transaction {}

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct TransactionMeta {
    involved_objects: HashMap<ShardId, SubstateChange>,
    max_outputs: u32,
}

impl TransactionMeta {
    pub fn new(involved_objects: HashMap<ShardId, SubstateChange>, max_outputs: u32) -> Self {
        Self {
            involved_objects,
            max_outputs,
        }
    }

    pub fn involved_objects_iter(&self) -> impl Iterator<Item = (&ShardId, &SubstateChange)> + '_ {
        self.involved_objects.iter()
    }

    pub fn involved_shards(&self) -> Vec<ShardId> {
        self.involved_objects.keys().copied().collect()
    }

    pub fn involved_objects_mut(&mut self) -> &mut HashMap<ShardId, SubstateChange> {
        &mut self.involved_objects
    }

    pub fn change_for_shard(&self, shard_id: ShardId) -> Option<SubstateChange> {
        self.involved_objects.get(&shard_id).copied()
    }

    pub fn set_max_outputs(&mut self, max_outputs: u32) -> &mut Self {
        self.max_outputs = max_outputs;
        self
    }

    pub fn max_outputs(&self) -> u32 {
        self.max_outputs
    }
}
