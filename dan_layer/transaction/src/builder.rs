//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::convert::TryFrom;

use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::{keys::PublicKey as PublicKeyTrait, ristretto::RistrettoPublicKey};
use tari_dan_common_types::ShardId;
use tari_engine_types::{
    confidential::ConfidentialClaim,
    instruction::Instruction,
    substate::SubstateAddress,
    TemplateAddress,
};
use tari_template_lib::{
    args,
    args::Arg,
    models::{
        Amount,
        ComponentAddress,
        ConfidentialWithdrawProof,
        NonFungibleAddress,
        NonFungibleId,
        NonFungibleIndexAddress,
        ResourceAddress,
    },
};

use super::Transaction;
use crate::{
    change::SubstateChange,
    id_provider::IdProvider,
    transaction::TransactionMeta,
    InstructionSignature,
    ObjectClaim,
};

#[derive(Debug, Clone, Default)]
pub struct TransactionBuilder {
    instructions: Vec<Instruction>,
    fee_instructions: Vec<Instruction>,
    meta: TransactionMeta,
    signature: Option<InstructionSignature>,
    sender_public_key: Option<RistrettoPublicKey>,
    new_non_fungible_outputs: Vec<(ResourceAddress, u8)>,
    new_non_fungible_index_outputs: Vec<(ResourceAddress, u64)>,
}

impl TransactionBuilder {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            fee_instructions: Vec::new(),
            signature: None,
            sender_public_key: None,
            meta: TransactionMeta::default(),
            new_non_fungible_outputs: vec![],
            new_non_fungible_index_outputs: vec![],
        }
    }

    /// Adds a fee instruction that calls the "take_fee" method on a component.
    /// This method must exist and return a Bucket with containing revealed confidential XTR resource.
    /// This allows the fee to originate from sources other than the transaction sender's account.
    pub fn fee_transaction_pay_from_component(mut self, component_address: ComponentAddress, fee: Amount) -> Self {
        self.fee_instructions.push(Instruction::CallMethod {
            component_address,
            method: "pay_fee".to_string(),
            args: args![fee],
        });
        self
    }

    /// Adds a fee instruction that calls the "take_fee_confidential" method on a component.
    /// This method must exist and return a Bucket with containing revealed confidential XTR resource.
    /// This allows the fee to originate from sources other than the transaction sender's account.
    pub fn fee_transaction_pay_from_component_confidential(
        mut self,
        component_address: ComponentAddress,
        proof: ConfidentialWithdrawProof,
    ) -> Self {
        self.fee_instructions.push(Instruction::CallMethod {
            component_address,
            method: "pay_fee_confidential".to_string(),
            args: args![proof],
        });
        self
    }

    pub fn call_function(self, template_address: TemplateAddress, function: &str, args: Vec<Arg>) -> Self {
        self.add_instruction(Instruction::CallFunction {
            template_address,
            function: function.to_string(),
            args,
        })
    }

    pub fn call_method(self, component_address: ComponentAddress, method: &str, args: Vec<Arg>) -> Self {
        self.add_instruction(Instruction::CallMethod {
            component_address,
            method: method.to_string(),
            args,
        })
    }

    pub fn put_last_instruction_output_on_workspace<T: AsRef<[u8]>>(self, label: T) -> Self {
        self.add_instruction(Instruction::PutLastInstructionOutputOnWorkspace {
            key: label.as_ref().to_vec(),
        })
    }

    pub fn claim_burn(self, claim: ConfidentialClaim) -> Self {
        self.add_instruction(Instruction::ClaimBurn { claim: Box::new(claim) })
    }

    pub fn with_fee_instructions(mut self, instructions: Vec<Instruction>) -> Self {
        self.fee_instructions = instructions;
        self
    }

    pub fn add_instruction(mut self, instruction: Instruction) -> Self {
        self.instructions.push(instruction);
        // Reset the signature as it is no longer valid
        self.signature = None;
        self
    }

    pub fn with_instructions(mut self, instructions: Vec<Instruction>) -> Self {
        self.instructions.extend(instructions);
        // Reset the signature as it is no longer valid
        self.signature = None;
        self
    }

    pub fn with_signature(mut self, signature: InstructionSignature) -> Self {
        self.signature = Some(signature);
        self
    }

    pub fn with_sender_public_key(mut self, sender_public_key: RistrettoPublicKey) -> Self {
        self.sender_public_key = Some(sender_public_key);
        self
    }

    pub fn sign(mut self, secret_key: &PrivateKey) -> Self {
        self.signature = Some(InstructionSignature::sign(secret_key, &self.instructions));
        self.sender_public_key = Some(PublicKey::from_secret_key(secret_key));
        self
    }

    /// Add an input to be consumed
    pub fn add_input(mut self, input_object: ShardId) -> Self {
        self.meta
            .involved_objects_mut()
            .insert(input_object, (SubstateChange::Destroy, ObjectClaim {}));
        self
    }

    pub fn with_inputs(mut self, inputs: Vec<ShardId>) -> Self {
        for input in inputs {
            self = self.add_input(input);
        }
        self
    }

    pub fn with_outputs(mut self, outputs: Vec<ShardId>) -> Self {
        for output in outputs {
            self = self.add_output(output);
        }
        self
    }

    pub fn add_output(mut self, output_object: ShardId) -> Self {
        self.meta
            .involved_objects_mut()
            .insert(output_object, (SubstateChange::Create, ObjectClaim {}));
        self
    }

    pub fn with_new_outputs(mut self, num_outputs: u8) -> Self {
        self.meta.set_max_outputs(num_outputs.into());
        self
    }

    pub fn with_new_non_fungible_outputs(mut self, new_non_fungible_outputs: Vec<(ResourceAddress, u8)>) -> Self {
        self.new_non_fungible_outputs = new_non_fungible_outputs;
        self
    }

    pub fn with_new_non_fungible_index_outputs(
        mut self,
        new_non_fungible_index_outputs: Vec<(ResourceAddress, u64)>,
    ) -> Self {
        self.new_non_fungible_index_outputs = new_non_fungible_index_outputs;
        self
    }

    pub fn build(mut self) -> Transaction {
        let mut transaction = Transaction::new(
            self.fee_instructions.drain(..).collect(),
            self.instructions.drain(..).collect(),
            self.signature.take().expect("not signed"),
            self.sender_public_key.take().expect("not signed"),
            self.meta,
        );

        let max_outputs = transaction.meta().max_outputs();
        let total_new_nft_outputs = self
            .new_non_fungible_outputs
            .iter()
            .map(|(_, count)| u32::from(*count))
            .sum::<u32>();
        let id_provider = IdProvider::new(*transaction.hash(), max_outputs + total_new_nft_outputs);

        transaction
            .meta_mut()
            .involved_objects_mut()
            .extend((0..max_outputs).map(|_| {
                let new_hash = id_provider
                    .new_address_hash()
                    .expect("id provider provides num_outputs IDs");
                (
                    ShardId::from_hash(&new_hash, 0),
                    (SubstateChange::Create, ObjectClaim {}),
                )
            }));

        let mut new_nft_outputs =
            Vec::with_capacity(usize::try_from(total_new_nft_outputs).expect("too many new NFT outputs"));
        for (resource_addr, count) in self.new_non_fungible_outputs {
            new_nft_outputs.extend((0..count).map({
                |_| {
                    let new_hash = id_provider.new_uuid().expect("id provider provides num_outputs IDs");
                    let address = NonFungibleAddress::new(resource_addr, NonFungibleId::from_u256(new_hash));
                    let new_addr = SubstateAddress::NonFungible(address);
                    (
                        ShardId::from_hash(&new_addr.to_canonical_hash(), 0),
                        (SubstateChange::Create, ObjectClaim {}),
                    )
                }
            }));
        }

        transaction.meta_mut().involved_objects_mut().extend(new_nft_outputs);

        // add the involved objects for NFT indexes
        let new_nft_index_outputs: Vec<(ShardId, (SubstateChange, ObjectClaim))> = self
            .new_non_fungible_index_outputs
            .iter()
            .map(|(res_addr, index)| {
                let index_addr = NonFungibleIndexAddress::new(*res_addr, *index);
                let substate_addr = SubstateAddress::NonFungibleIndex(index_addr);
                let shard_id = ShardId::from_hash(&substate_addr.to_canonical_hash(), 0);

                (shard_id, (SubstateChange::Create, ObjectClaim {}))
            })
            .collect();
        transaction
            .meta_mut()
            .involved_objects_mut()
            .extend(new_nft_index_outputs);

        transaction
    }
}
