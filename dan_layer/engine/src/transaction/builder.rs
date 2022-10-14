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

use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::{keys::PublicKey as PublicKeyTrait, ristretto::RistrettoPublicKey};
use tari_dan_common_types::{ObjectClaim, ObjectId, ShardId, SubstateChange};
use tari_engine_types::{instruction::Instruction, signature::InstructionSignature};

use super::Transaction;
use crate::{crypto::create_key_pair, runtime::IdProvider, transaction::TransactionMeta};

#[derive(Debug, Clone, Default)]
pub struct TransactionBuilder {
    instructions: Vec<Instruction>,
    signature: Option<InstructionSignature>,
    sender_public_key: Option<RistrettoPublicKey>,
    fee: u64,
    meta: TransactionMeta,
    new_components: u8,
    // max_outputs: u8,
}

impl TransactionBuilder {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            signature: None,
            sender_public_key: None,
            fee: 0,
            meta: TransactionMeta {
                involved_objects: HashMap::new(),
            },
            // max_outputs: 0,
            new_components: 0,
        }
    }

    pub fn fee(&mut self, fee: u64) {
        self.fee = fee;
    }

    pub fn add_instruction(&mut self, instruction: Instruction) -> &mut Self {
        self.instructions.push(instruction);
        // Reset the signature as it is no longer valid
        self.signature = None;
        self
    }

    pub fn with_instructions(&mut self, instructions: Vec<Instruction>) -> &mut Self {
        self.instructions.extend(instructions);
        // Reset the signature as it is no longer valid
        self.signature = None;
        self
    }

    pub fn signature(&mut self, signature: InstructionSignature) -> &mut Self {
        self.signature = Some(signature);
        self
    }

    pub fn sender_public_key(&mut self, sender_public_key: RistrettoPublicKey) -> &mut Self {
        self.sender_public_key = Some(sender_public_key);
        self
    }

    pub fn sign(&mut self, secret_key: &PrivateKey) -> &mut Self {
        let (nonce, _nonce_pk) = create_key_pair();
        self.signature = Some(InstructionSignature::sign(secret_key, nonce, &self.instructions));
        self.sender_public_key = Some(PublicKey::from_secret_key(secret_key));
        self
    }

    pub fn add_input_object(&mut self, input_object: (ShardId, ObjectId)) -> &mut Self {
        let entry = self.meta.involved_objects.entry(input_object.0).or_insert(vec![]);
        entry.push((input_object.1, SubstateChange::Destroy, ObjectClaim {}));
        self
    }

    // pub fn add_outputs(&mut self, max_outputs: u8) -> &mut Self {
    //     self.max_outputs += max_outputs;
    //     self
    // }

    pub fn with_new_components(&mut self, components: u8) -> &mut Self {
        self.new_components += components;
        self
    }

    pub fn build(mut self) -> Transaction {
        let mut t = Transaction::new(
            self.fee,
            self.instructions.drain(..).collect(),
            self.signature.take().expect("not signed"),
            self.sender_public_key.take().expect("not signed"),
            self.meta,
        );

        let id_provider = IdProvider::new(t.hash);
        // for o in 0..self.max_outputs {
        //     let value: [u8; 32] = Blake256::new().chain(base_hash).chain(&[o]).finalize_fixed().into();
        //     let object_id = ObjectId(value);
        //     let shard_id = ShardId(value);
        //     t.meta.involved_objects.entry(shard_id).or_insert(vec![]).push((
        //         object_id,
        //         SubstateChange::Create,
        //         ObjectClaim {},
        //     ));
        // }

        for _o in 0..self.new_components {
            // let value: [u8; 32] = Blake256::chain(Blake256::new(), base_hash).chain(&[o]).finalize_fixed().into();
            // let object_id = ObjectId(value);
            // let shard_id = ShardId(value);
            let id = id_provider.new_component_address();
            let shard_id = ShardId(id.into_array());
            let object_id = ObjectId(id.into_array());
            if t.meta.is_none() {
                t.meta = Some(TransactionMeta {
                    involved_objects: HashMap::new(),
                });
            }
            if let Some(m) = t.meta.as_mut() {
                m.involved_objects.entry(shard_id).or_insert(vec![]).push((
                    object_id,
                    SubstateChange::Create,
                    ObjectClaim {},
                ))
            }
        }
        t
    }
}
