//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{PrivateKey, PublicKey};
use tari_dan_common_types::{Epoch, SubstateRequirement};
use tari_engine_types::{confidential::ConfidentialClaim, instruction::Instruction, TemplateAddress};
use tari_template_lib::{
    args,
    args::Arg,
    auth::OwnerRule,
    models::{Amount, ComponentAddress, ConfidentialWithdrawProof, ResourceAddress},
    prelude::AccessRules,
};

use crate::{unsigned_transaction::UnsignedTransaction, Transaction, TransactionSignature};

#[derive(Debug, Clone, Default)]
pub struct TransactionBuilder {
    unsigned_transaction: UnsignedTransaction,
    signatures: Vec<TransactionSignature>,
}

impl TransactionBuilder {
    pub fn new() -> Self {
        Self {
            unsigned_transaction: UnsignedTransaction::default(),
            signatures: vec![],
        }
    }

    pub fn with_unsigned_transaction(self, unsigned_transaction: UnsignedTransaction) -> Self {
        Self {
            unsigned_transaction,
            signatures: vec![],
        }
    }

    /// Adds a fee instruction that calls the "take_fee" method on a component.
    /// This method must exist and return a Bucket with containing revealed confidential XTR resource.
    /// This allows the fee to originate from sources other than the transaction sender's account.
    /// The fee instruction will lock up the "max_fee" amount for the duration of the transaction.
    pub fn fee_transaction_pay_from_component(self, component_address: ComponentAddress, max_fee: Amount) -> Self {
        self.add_fee_instruction(Instruction::CallMethod {
            component_address,
            method: "pay_fee".to_string(),
            args: args![max_fee],
        })
    }

    /// Adds a fee instruction that calls the "take_fee_confidential" method on a component.
    /// This method must exist and return a Bucket with containing revealed confidential XTR resource.
    /// This allows the fee to originate from sources other than the transaction sender's account.
    pub fn fee_transaction_pay_from_component_confidential(
        self,
        component_address: ComponentAddress,
        proof: ConfidentialWithdrawProof,
    ) -> Self {
        self.add_fee_instruction(Instruction::CallMethod {
            component_address,
            method: "pay_fee_confidential".to_string(),
            args: args![proof],
        })
    }

    pub fn create_account(self, owner_public_key: PublicKey) -> Self {
        self.add_instruction(Instruction::CreateAccount {
            public_key_address: owner_public_key,
            owner_rule: None,
            access_rules: None,
            workspace_bucket: None,
        })
    }

    pub fn create_account_with_bucket<T: Into<String>>(self, owner_public_key: PublicKey, workspace_bucket: T) -> Self {
        self.add_instruction(Instruction::CreateAccount {
            public_key_address: owner_public_key,
            owner_rule: None,
            access_rules: None,
            workspace_bucket: Some(workspace_bucket.into()),
        })
    }

    pub fn create_account_with_custom_rules<T: Into<String>>(
        self,
        public_key_address: PublicKey,
        owner_rule: Option<OwnerRule>,
        access_rules: Option<AccessRules>,
        workspace_bucket: Option<T>,
    ) -> Self {
        self.add_instruction(Instruction::CreateAccount {
            public_key_address,
            owner_rule,
            access_rules,
            workspace_bucket: workspace_bucket.map(|b| b.into()),
        })
    }

    pub fn call_function<T: ToString>(self, template_address: TemplateAddress, function: T, args: Vec<Arg>) -> Self {
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

    pub fn drop_all_proofs_in_workspace(self) -> Self {
        self.add_instruction(Instruction::DropAllProofsInWorkspace)
    }

    pub fn put_last_instruction_output_on_workspace<T: AsRef<[u8]>>(self, label: T) -> Self {
        self.add_instruction(Instruction::PutLastInstructionOutputOnWorkspace {
            key: label.as_ref().to_vec(),
        })
    }

    pub fn assert_bucket_contains<T: AsRef<[u8]>>(
        self,
        label: T,
        resource_address: ResourceAddress,
        min_amount: Amount,
    ) -> Self {
        self.add_instruction(Instruction::AssertBucketContains {
            key: label.as_ref().to_vec(),
            resource_address,
            min_amount,
        })
    }

    pub fn claim_burn(self, claim: ConfidentialClaim) -> Self {
        self.add_instruction(Instruction::ClaimBurn { claim: Box::new(claim) })
    }

    pub fn create_proof(self, account: ComponentAddress, resource_addr: ResourceAddress) -> Self {
        // We may want to make this a native instruction
        self.add_instruction(Instruction::CallMethod {
            component_address: account,
            method: "create_proof_for_resource".to_string(),
            args: args![resource_addr],
        })
    }

    pub fn with_fee_instructions(mut self, instructions: Vec<Instruction>) -> Self {
        self.unsigned_transaction.fee_instructions = instructions;
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    pub fn with_fee_instructions_builder<F: FnOnce(TransactionBuilder) -> TransactionBuilder>(mut self, f: F) -> Self {
        let builder = f(TransactionBuilder::new());
        self.unsigned_transaction.fee_instructions = builder.unsigned_transaction.instructions;
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    pub fn add_fee_instruction(mut self, instruction: Instruction) -> Self {
        self.unsigned_transaction.fee_instructions.push(instruction);
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    pub fn add_instruction(mut self, instruction: Instruction) -> Self {
        self.unsigned_transaction.instructions.push(instruction);
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    pub fn with_instructions(mut self, instructions: Vec<Instruction>) -> Self {
        self.unsigned_transaction.instructions.extend(instructions);
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    /// Add an input to use in the transaction
    pub fn add_input<I: Into<SubstateRequirement>>(mut self, input_object: I) -> Self {
        self.unsigned_transaction.inputs.insert(input_object.into());
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    pub fn with_inputs<I: IntoIterator<Item = SubstateRequirement>>(mut self, inputs: I) -> Self {
        self.unsigned_transaction.inputs.extend(inputs);
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    pub fn with_min_epoch(mut self, min_epoch: Option<Epoch>) -> Self {
        self.unsigned_transaction.min_epoch = min_epoch;
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    pub fn with_max_epoch(mut self, max_epoch: Option<Epoch>) -> Self {
        self.unsigned_transaction.max_epoch = max_epoch;
        // Reset the signatures as they are no longer valid
        self.signatures = vec![];
        self
    }

    pub fn build_unsigned_transaction(self) -> UnsignedTransaction {
        self.unsigned_transaction
    }

    pub fn sign(mut self, secret_key: &PrivateKey) -> Self {
        self.signatures
            .push(TransactionSignature::sign(secret_key, &self.unsigned_transaction));
        self
    }

    pub fn build(self) -> Transaction {
        Transaction::new(self.unsigned_transaction, self.signatures)
    }
}
