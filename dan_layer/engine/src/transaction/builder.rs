//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::{keys::PublicKey as PublicKeyTrait, ristretto::RistrettoPublicKey};
use tari_dan_common_types::{ObjectClaim, ShardId, SubstateChange};
use tari_engine_types::{instruction::Instruction, signature::InstructionSignature};

use super::Transaction;
use crate::{crypto::create_key_pair, runtime::IdProvider, transaction::TransactionMeta};

#[derive(Debug, Clone, Default)]
pub struct TransactionBuilder {
    instructions: Vec<Instruction>,
    fee: u64,
    meta: TransactionMeta,
    signature: Option<InstructionSignature>,
    sender_public_key: Option<RistrettoPublicKey>,
}

impl TransactionBuilder {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            signature: None,
            sender_public_key: None,
            fee: 0,
            meta: TransactionMeta::default(),
        }
    }

    pub fn with_fee(&mut self, fee: u64) -> &mut Self {
        self.fee = fee;
        self
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

    pub fn with_signature(&mut self, signature: InstructionSignature) -> &mut Self {
        self.signature = Some(signature);
        self
    }

    pub fn with_sender_public_key(&mut self, sender_public_key: RistrettoPublicKey) -> &mut Self {
        self.sender_public_key = Some(sender_public_key);
        self
    }

    pub fn sign(&mut self, secret_key: &PrivateKey) -> &mut Self {
        let (nonce, _nonce_pk) = create_key_pair();
        self.signature = Some(InstructionSignature::sign(secret_key, nonce, &self.instructions));
        self.sender_public_key = Some(PublicKey::from_secret_key(secret_key));
        self
    }

    /// Add an input to be consumed
    pub fn add_input(&mut self, input_object: ShardId) -> &mut Self {
        self.meta
            .involved_objects
            .insert(input_object, (SubstateChange::Destroy, ObjectClaim {}));
        self
    }

    pub fn with_inputs(&mut self, inputs: Vec<ShardId>) -> &mut Self {
        for input in inputs {
            self.add_input(input);
        }
        self
    }

    pub fn with_outputs(&mut self, outputs: Vec<ShardId>) -> &mut Self {
        for output in outputs {
            self.add_output(output);
        }
        self
    }

    pub fn add_output(&mut self, output_object: ShardId) -> &mut Self {
        self.meta
            .involved_objects
            .insert(output_object, (SubstateChange::Create, ObjectClaim {}));
        self
    }

    pub fn with_new_outputs(&mut self, num_outputs: u8) -> &mut Self {
        self.meta.max_outputs = num_outputs.into();
        self
    }

    pub fn build(mut self) -> Transaction {
        let mut transaction = Transaction::new(
            self.fee,
            self.instructions.drain(..).collect(),
            self.signature.take().expect("not signed"),
            self.sender_public_key.take().expect("not signed"),
            self.meta,
        );

        let max_outputs = transaction.meta().max_outputs;
        let id_provider = IdProvider::new(transaction.hash, max_outputs);

        transaction.meta.involved_objects.extend((0..max_outputs).map(|_| {
            let new_hash = id_provider
                .new_address_hash()
                .expect("id provider provides num_outputs IDs");
            (
                ShardId::from_hash(&new_hash, 0),
                (SubstateChange::Create, ObjectClaim {}),
            )
        }));

        transaction
    }
}
