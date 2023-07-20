//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::borrow::Borrow;

use tari_common_types::types::PrivateKey;
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
    models::{Amount, ComponentAddress, ConfidentialWithdrawProof},
};

use crate::{Transaction, TransactionSignature};

#[derive(Debug, Clone, Default)]
pub struct TransactionBuilder {
    instructions: Vec<Instruction>,
    fee_instructions: Vec<Instruction>,
    signature: Option<TransactionSignature>,
    inputs: Vec<ShardId>,
    input_refs: Vec<ShardId>,
    outputs: Vec<ShardId>,
}

impl TransactionBuilder {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            fee_instructions: Vec::new(),
            signature: None,
            inputs: Vec::new(),
            input_refs: Vec::new(),
            outputs: Vec::new(),
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

    pub fn with_signature(mut self, signature: TransactionSignature) -> Self {
        self.signature = Some(signature);
        self
    }

    pub fn sign(mut self, secret_key: &PrivateKey) -> Self {
        // TODO: create proper challenge that signs everything
        self.signature = Some(TransactionSignature::sign(secret_key, &self.instructions));
        self
    }

    /// Add an input to be consumed
    pub fn add_input(mut self, input_object: ShardId) -> Self {
        self.inputs.push(input_object);
        self
    }

    pub fn with_substate_inputs<I: IntoIterator<Item = (B, u32)>, B: Borrow<SubstateAddress>>(self, inputs: I) -> Self {
        self.with_inputs(inputs.into_iter().map(|(a, v)| ShardId::from_address(a.borrow(), v)))
    }

    pub fn with_inputs<I: IntoIterator<Item = ShardId>>(mut self, inputs: I) -> Self {
        self.inputs.extend(inputs);
        self
    }

    /// Add an input to be used without mutation
    pub fn add_input_ref(mut self, input_object: ShardId) -> Self {
        self.input_refs.push(input_object);
        self
    }

    pub fn with_substate_input_refs<I: IntoIterator<Item = (B, u32)>, B: Borrow<SubstateAddress>>(
        self,
        inputs: I,
    ) -> Self {
        self.with_input_refs(inputs.into_iter().map(|(a, v)| ShardId::from_address(a.borrow(), v)))
    }

    pub fn with_input_refs<I: IntoIterator<Item = ShardId>>(mut self, inputs: I) -> Self {
        self.input_refs.extend(inputs);
        self
    }

    pub fn add_output(mut self, output_object: ShardId) -> Self {
        self.outputs.push(output_object);
        self
    }

    pub fn with_substate_outputs<I: IntoIterator<Item = (B, u32)>, B: Borrow<SubstateAddress>>(
        self,
        outputs: I,
    ) -> Self {
        self.with_outputs(outputs.into_iter().map(|(a, v)| ShardId::from_address(a.borrow(), v)))
    }

    pub fn with_outputs<I: IntoIterator<Item = ShardId>>(mut self, outputs: I) -> Self {
        self.outputs.extend(outputs);
        self
    }

    pub fn add_output_ref(mut self, output_object: ShardId) -> Self {
        self.outputs.push(output_object);
        self
    }

    pub fn build(mut self) -> Transaction {
        Transaction::new(
            self.fee_instructions.drain(..).collect(),
            self.instructions.drain(..).collect(),
            self.signature.take().expect("not signed"),
            self.inputs,
            self.input_refs,
            self.outputs,
            vec![],
            vec![],
        )
    }
}
