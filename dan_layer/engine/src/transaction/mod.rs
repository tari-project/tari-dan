//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{ObjectClaim, ShardId, SubstateChange};
use tari_engine_types::{hashing::hasher, instruction::Instruction, signature::InstructionSignature};
use tari_template_lib::{models::TemplateAddress, Hash};
use tari_utilities::ByteArray;
mod builder;
pub use builder::TransactionBuilder;

mod error;
pub use error::TransactionError;

mod processor;
pub use processor::TransactionProcessor;
use tari_template_lib::models::ComponentAddress;

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
    meta: Option<TransactionMeta>,
}

impl Transaction {
    pub fn builder() -> TransactionBuilder {
        TransactionBuilder::new()
    }

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

    pub fn objects_for_shard(&self, shard_id: ShardId) -> Option<(SubstateChange, ObjectClaim)> {
        self.involved_objects.get(&shard_id).cloned()
    }

    pub fn max_outputs(&self) -> u32 {
        self.max_outputs
    }
}

impl Transaction {
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
            meta: Some(meta),
        };
        s.hash = s.calculate_hash();
        s
    }

    pub fn hash(&self) -> &Hash {
        &self.hash
    }

    pub fn fee(&self) -> u64 {
        self._fee
    }

    pub fn meta(&self) -> &TransactionMeta {
        self.meta.as_ref().unwrap()
    }

    fn calculate_hash(&self) -> Hash {
        let mut res = hasher("transaction")
            .chain(self.sender_public_key.as_bytes())
            .chain(self.signature.signature().get_public_nonce().as_bytes())
            .chain(self.signature.signature().get_signature().as_bytes());
        for instruction in &self.instructions {
            res.update(&instruction.hash())
        }
        res.result()
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
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
