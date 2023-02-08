//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::ShardId;
use tari_engine_types::{hashing::hasher, instruction::Instruction, signature::InstructionSignature};
use tari_template_lib::{
    models::{ComponentAddress, TemplateAddress},
    Hash,
};

use crate::{change::SubstateChange, ObjectClaim, TransactionBuilder};

#[derive(Debug, Clone)]
pub struct BalanceProof {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Transaction {
    hash: Hash,
    instructions: Vec<Instruction>,
    signature: InstructionSignature,
    _fee: u64,
    sender_public_key: PublicKey,
    // Not part of signature. TODO: Should it be?
    meta: TransactionMeta,
}

impl Transaction {
    pub fn builder() -> TransactionBuilder {
        TransactionBuilder::new()
    }

    pub fn new(
        fee: u64,
        instructions: Vec<Instruction>,
        signature: InstructionSignature,
        sender_public_key: PublicKey,
        meta: TransactionMeta,
    ) -> Self {
        let mut s = Self {
            hash: Hash::default(),
            instructions,
            signature,
            _fee: fee,
            sender_public_key,
            meta,
        };
        s.hash = s.calculate_hash();
        s
    }

    /// Returns the template addresses that are statically known to be executed by this transaction.
    /// This does not include templates for component invocation as that data is not contained within the transaction.
    pub fn required_templates(&self) -> Vec<TemplateAddress> {
        self.instructions
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::CallFunction {
                    template_address: package_address,
                    ..
                } => Some(*package_address),
                _ => None,
            })
            .collect()
    }

    pub fn required_components(&self) -> Vec<ComponentAddress> {
        self.instructions
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::CallMethod { component_address, .. } => Some(*component_address),
                _ => None,
            })
            .collect()
    }

    pub fn hash(&self) -> &Hash {
        &self.hash
    }

    pub fn fee(&self) -> u64 {
        self._fee
    }

    pub fn meta(&self) -> &TransactionMeta {
        &self.meta
    }

    pub fn meta_mut(&mut self) -> &mut TransactionMeta {
        &mut self.meta
    }

    fn calculate_hash(&self) -> Hash {
        let mut res = hasher("transaction")
            .chain(&self.sender_public_key)
            .chain(self.signature.signature().get_public_nonce())
            .chain(self.signature.signature().get_signature());
        for instruction in &self.instructions {
            res.update(&instruction.hash())
        }
        res.result()
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

    pub fn destruct(self) -> (Vec<Instruction>, InstructionSignature, PublicKey) {
        (self.instructions, self.signature, self.sender_public_key)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct TransactionMeta {
    involved_objects: HashMap<ShardId, (SubstateChange, ObjectClaim)>,
    max_outputs: u32,
}

impl TransactionMeta {
    pub fn new(involved_objects: HashMap<ShardId, (SubstateChange, ObjectClaim)>, max_outputs: u32) -> Self {
        Self {
            involved_objects,
            max_outputs,
        }
    }

    pub fn involved_objects_iter(&self) -> impl Iterator<Item = (&ShardId, &(SubstateChange, ObjectClaim))> + '_ {
        self.involved_objects.iter()
    }

    pub fn involved_shards(&self) -> Vec<ShardId> {
        self.involved_objects.keys().copied().collect()
    }

    pub(crate) fn involved_objects_mut(&mut self) -> &mut HashMap<ShardId, (SubstateChange, ObjectClaim)> {
        &mut self.involved_objects
    }

    pub fn objects_for_shard(&self, shard_id: ShardId) -> Option<(SubstateChange, ObjectClaim)> {
        self.involved_objects.get(&shard_id).cloned()
    }

    pub fn set_max_outputs(&mut self, max_outputs: u32) -> &mut Self {
        self.max_outputs = max_outputs;
        self
    }

    pub fn max_outputs(&self) -> u32 {
        self.max_outputs
    }
}
