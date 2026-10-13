//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::limits::{STEALTH_LIMITS, WASM_LIMITS};
use tari_template_lib_types::{OwnerRule, access_rules::MAX_ACCESS_RULE_NODES};

use crate::{Instruction, MAX_TRANSACTION_INSTRUCTIONS};

/// The most arguments one instruction may pass to a call: the most a template function takes.
pub const MAX_INSTRUCTION_ARGS: usize = WASM_LIMITS.max_function_arguments;

/// The most call arguments a list of instructions may pass in total.
///
/// An argument decodes to a fixed-size value however few bytes encode it, so, like the instruction count, this
/// bounds the memory decoding a transaction takes.
pub const MAX_TRANSACTION_INSTRUCTION_ARGS: usize = MAX_TRANSACTION_INSTRUCTIONS;

/// The most access rule nodes a list of instructions may carry in total, counted as
/// [`ComponentAccessRules::num_nodes`](tari_template_lib_types::access_rules::ComponentAccessRules::num_nodes)
/// and [`StealthTransferStatement::num_nodes`](tari_template_lib_types::stealth::StealthTransferStatement::num_nodes)
/// count them.
///
/// A node decodes to a fixed-size value however few bytes encode it, so, like the instruction count, this bounds
/// the memory decoding a transaction takes.
pub const MAX_TRANSACTION_ACCESS_RULE_NODES: usize = MAX_ACCESS_RULE_NODES;

/// The most stealth inputs a list of instructions may spend in total: the most a transaction may spend.
pub const MAX_TRANSACTION_STEALTH_INPUTS: usize = STEALTH_LIMITS.max_total_inputs_per_transaction;

/// The most stealth outputs a list of instructions may create in total: the most a transaction may create.
pub const MAX_TRANSACTION_STEALTH_OUTPUTS: usize = STEALTH_LIMITS.max_total_outputs_per_transaction;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstructionLimitError {
    #[error("An instruction passes {actual} arguments, but the maximum allowed is {max}")]
    TooManyCallArguments { max: usize, actual: usize },
    #[error("An instruction's access rules hold {actual} nodes, but the maximum allowed is {max}")]
    AccessRulesTooLarge { max: usize, actual: usize },
    #[error("The instructions pass {actual} arguments in total, but the maximum allowed is {max}")]
    TooManyArguments { max: usize, actual: usize },
    #[error("The instructions' access rules hold {actual} nodes in total, but the maximum allowed is {max}")]
    TooManyAccessRuleNodes { max: usize, actual: usize },
    #[error("A stealth transfer holds more inputs, outputs, covenant claims, conditions or proof hashes than it may")]
    StealthTransferTooLarge,
    #[error("The instructions spend {actual} stealth inputs in total, but the maximum allowed is {max}")]
    TooManyStealthInputs { max: usize, actual: usize },
    #[error("The instructions create {actual} stealth outputs in total, but the maximum allowed is {max}")]
    TooManyStealthOutputs { max: usize, actual: usize },
}

/// Tallies the call arguments, access rule nodes and stealth inputs and outputs a list of instructions carries,
/// refusing an instruction that takes the list past a limit.
///
/// Decoding an instruction list applies these limits, so a list that passes them is one every peer can decode.
/// Access rule nodes include the spend condition nodes of stealth transfers.
#[derive(Debug, Clone, Default)]
pub struct InstructionLimitTracker {
    num_args: usize,
    num_access_rule_nodes: usize,
    num_stealth_inputs: usize,
    num_stealth_outputs: usize,
}

impl InstructionLimitTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, instruction: &Instruction) -> Result<(), InstructionLimitError> {
        match instruction {
            Instruction::CallFunction { args, .. } | Instruction::CallMethod { args, .. } => self.add_args(args.len()),
            Instruction::UpdateComponentTemplate { migrate, .. } => {
                self.add_args(migrate.as_ref().map_or(0, |migrate| migrate.args.len()))
            },
            Instruction::CreateAccount {
                owner_rule,
                access_rules,
                ..
            } => {
                if let Some(OwnerRule::ByAccessRule(rule)) = owner_rule {
                    self.add_access_rule_nodes(rule.num_nodes())?;
                }
                if let Some(rules) = access_rules {
                    self.add_access_rule_nodes(rules.num_nodes())?;
                }
                Ok(())
            },
            Instruction::StealthTransfer { statement, .. } => {
                if !statement.is_within_decode_limits() {
                    return Err(InstructionLimitError::StealthTransferTooLarge);
                }
                self.add_access_rule_nodes(statement.num_nodes())?;
                self.add_stealth_inputs_and_outputs(statement.stealth_inputs().len(), statement.stealth_outputs().len())
            },
            Instruction::PutLastInstructionOutputOnWorkspace { .. } |
            Instruction::ClaimBurn { .. } |
            Instruction::ClaimValidatorFees { .. } |
            Instruction::DropAllProofsInWorkspace |
            Instruction::Assert { .. } |
            Instruction::TakeFromBucket { .. } |
            Instruction::PublishTemplate { .. } |
            Instruction::AllocateAddress { .. } |
            Instruction::PayFeeFromBucket { .. } |
            Instruction::PutIntoBucket { .. } => Ok(()),
        }
    }

    fn add_args(&mut self, num_args: usize) -> Result<(), InstructionLimitError> {
        if num_args > MAX_INSTRUCTION_ARGS {
            return Err(InstructionLimitError::TooManyCallArguments {
                max: MAX_INSTRUCTION_ARGS,
                actual: num_args,
            });
        }
        self.num_args += num_args;
        if self.num_args > MAX_TRANSACTION_INSTRUCTION_ARGS {
            return Err(InstructionLimitError::TooManyArguments {
                max: MAX_TRANSACTION_INSTRUCTION_ARGS,
                actual: self.num_args,
            });
        }
        Ok(())
    }

    fn add_access_rule_nodes(&mut self, num_nodes: usize) -> Result<(), InstructionLimitError> {
        if num_nodes > MAX_ACCESS_RULE_NODES {
            return Err(InstructionLimitError::AccessRulesTooLarge {
                max: MAX_ACCESS_RULE_NODES,
                actual: num_nodes,
            });
        }
        self.num_access_rule_nodes += num_nodes;
        if self.num_access_rule_nodes > MAX_TRANSACTION_ACCESS_RULE_NODES {
            return Err(InstructionLimitError::TooManyAccessRuleNodes {
                max: MAX_TRANSACTION_ACCESS_RULE_NODES,
                actual: self.num_access_rule_nodes,
            });
        }
        Ok(())
    }

    fn add_stealth_inputs_and_outputs(
        &mut self,
        num_inputs: usize,
        num_outputs: usize,
    ) -> Result<(), InstructionLimitError> {
        self.num_stealth_inputs += num_inputs;
        if self.num_stealth_inputs > MAX_TRANSACTION_STEALTH_INPUTS {
            return Err(InstructionLimitError::TooManyStealthInputs {
                max: MAX_TRANSACTION_STEALTH_INPUTS,
                actual: self.num_stealth_inputs,
            });
        }
        self.num_stealth_outputs += num_outputs;
        if self.num_stealth_outputs > MAX_TRANSACTION_STEALTH_OUTPUTS {
            return Err(InstructionLimitError::TooManyStealthOutputs {
                max: MAX_TRANSACTION_STEALTH_OUTPUTS,
                actual: self.num_stealth_outputs,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_template_lib_types::{
        AccessRule,
        Amount,
        access_rules::{ComponentAccessRules, RestrictedAccessRule},
        crypto::{PedersenCommitmentBytes, RangeProofBytes, RistrettoPublicKeyBytes},
        stealth::{
            MAX_STEALTH_INPUTS,
            MerkleProof,
            SpendCondition,
            SpendWitness,
            StealthInput,
            StealthInputsStatement,
            StealthOutputsStatement,
            StealthTransferStatement,
        },
    };

    use super::*;
    use crate::{
        ResourceAddressRef,
        args::{InstructionArg, WorkspaceOffsetId},
    };

    fn call_function(num_args: usize) -> Instruction {
        Instruction::CallFunction {
            address: Default::default(),
            function: Default::default(),
            args: (0..num_args).map(|_| InstructionArg::workspace(0, None)).collect(),
        }
    }

    fn create_account(num_rule_nodes: usize) -> Instruction {
        let rule = AccessRule::Restricted(RestrictedAccessRule::AnyOf(
            (1..num_rule_nodes)
                .map(|_| RestrictedAccessRule::AnyOf(Box::new([])))
                .collect(),
        ));
        Instruction::CreateAccount {
            owner_public_key: RistrettoPublicKeyBytes::default(),
            owner_rule: Some(OwnerRule::ByAccessRule(rule)),
            access_rules: None,
            bucket_workspace_id: None,
        }
    }

    fn add_all(instructions: &[Instruction]) -> Result<(), InstructionLimitError> {
        let mut tracker = InstructionLimitTracker::new();
        instructions.iter().try_for_each(|instruction| tracker.add(instruction))
    }

    #[test]
    fn it_accepts_instructions_up_to_the_limits() {
        add_all(&[call_function(MAX_INSTRUCTION_ARGS)]).unwrap();
        add_all(&vec![call_function(1); MAX_TRANSACTION_INSTRUCTION_ARGS]).unwrap();
        add_all(&[create_account(MAX_ACCESS_RULE_NODES)]).unwrap();
    }

    #[test]
    fn it_counts_method_rules_as_access_rule_nodes() {
        let rules = (0..MAX_ACCESS_RULE_NODES).fold(ComponentAccessRules::new(), |rules, i| {
            rules.method(format!("{i}"), AccessRule::AllowAll)
        });
        let mut instruction = create_account(1);
        let Instruction::CreateAccount { access_rules, .. } = &mut instruction else {
            unreachable!()
        };
        *access_rules = Some(rules);
        assert_eq!(
            add_all(&[instruction]),
            Err(InstructionLimitError::TooManyAccessRuleNodes {
                max: MAX_TRANSACTION_ACCESS_RULE_NODES,
                actual: MAX_TRANSACTION_ACCESS_RULE_NODES + 1
            })
        );
    }

    #[test]
    fn it_refuses_a_call_over_the_argument_limit() {
        assert_eq!(
            add_all(&[call_function(MAX_INSTRUCTION_ARGS + 1)]),
            Err(InstructionLimitError::TooManyCallArguments {
                max: MAX_INSTRUCTION_ARGS,
                actual: MAX_INSTRUCTION_ARGS + 1
            })
        );
    }

    #[test]
    fn it_refuses_arguments_over_the_list_limit() {
        assert_eq!(
            add_all(&vec![call_function(1); MAX_TRANSACTION_INSTRUCTION_ARGS + 1]),
            Err(InstructionLimitError::TooManyArguments {
                max: MAX_TRANSACTION_INSTRUCTION_ARGS,
                actual: MAX_TRANSACTION_INSTRUCTION_ARGS + 1
            })
        );
    }

    #[test]
    fn it_refuses_an_access_rule_over_the_node_limit() {
        assert_eq!(
            add_all(&[create_account(MAX_ACCESS_RULE_NODES + 1)]),
            Err(InstructionLimitError::AccessRulesTooLarge {
                max: MAX_ACCESS_RULE_NODES,
                actual: MAX_ACCESS_RULE_NODES + 1
            })
        );
    }

    fn stealth_transfer(inputs: Vec<StealthInput>) -> Instruction {
        Instruction::StealthTransfer {
            resource_address_ref: ResourceAddressRef::Workspace(WorkspaceOffsetId::new(0)),
            statement: StealthTransferStatement {
                inputs_statement: StealthInputsStatement {
                    inputs,
                    revealed_amount: Amount::ZERO,
                },
                outputs_statement: StealthOutputsStatement {
                    outputs: vec![],
                    revealed_output: None,
                    agg_range_proof: RangeProofBytes::empty(),
                },
                balance_proof: None,
                covenant_claims: vec![],
            },
            revealed_input_bucket: None,
        }
    }

    fn key_path_inputs(n: usize) -> Vec<StealthInput> {
        (0..n)
            .map(|_| StealthInput::new(PedersenCommitmentBytes::zero()))
            .collect()
    }

    fn script_path_input(rule: AccessRule) -> StealthInput {
        StealthInput::with_witness(
            PedersenCommitmentBytes::zero(),
            SpendWitness::script_path(SpendCondition::access_rule(rule), MerkleProof::empty()),
        )
    }

    #[test]
    fn it_refuses_a_stealth_transfer_outside_its_decode_limits() {
        assert_eq!(
            add_all(&[stealth_transfer(key_path_inputs(MAX_STEALTH_INPUTS + 1))]),
            Err(InstructionLimitError::StealthTransferTooLarge)
        );
    }

    #[test]
    fn it_refuses_stealth_inputs_over_the_list_limit() {
        let first = MAX_STEALTH_INPUTS;
        let second = MAX_TRANSACTION_STEALTH_INPUTS + 1 - first;
        assert_eq!(
            add_all(&[
                stealth_transfer(key_path_inputs(first)),
                stealth_transfer(key_path_inputs(second))
            ]),
            Err(InstructionLimitError::TooManyStealthInputs {
                max: MAX_TRANSACTION_STEALTH_INPUTS,
                actual: MAX_TRANSACTION_STEALTH_INPUTS + 1
            })
        );
    }

    #[test]
    fn it_counts_spend_conditions_as_access_rule_nodes() {
        let Instruction::CreateAccount {
            owner_rule: Some(OwnerRule::ByAccessRule(rule)),
            ..
        } = create_account(MAX_TRANSACTION_ACCESS_RULE_NODES - 1)
        else {
            unreachable!()
        };
        // The spend condition's atom is one node besides the rule's own.
        let transfer = stealth_transfer(vec![script_path_input(rule)]);
        add_all(std::slice::from_ref(&transfer)).unwrap();
        assert_eq!(
            add_all(&[create_account(1), transfer]),
            Err(InstructionLimitError::TooManyAccessRuleNodes {
                max: MAX_TRANSACTION_ACCESS_RULE_NODES,
                actual: MAX_TRANSACTION_ACCESS_RULE_NODES + 1
            })
        );
    }

    #[test]
    fn it_refuses_access_rule_nodes_over_the_list_limit() {
        assert_eq!(
            add_all(&[create_account(MAX_TRANSACTION_ACCESS_RULE_NODES), create_account(1)]),
            Err(InstructionLimitError::TooManyAccessRuleNodes {
                max: MAX_TRANSACTION_ACCESS_RULE_NODES,
                actual: MAX_TRANSACTION_ACCESS_RULE_NODES + 1
            })
        );
    }
}
