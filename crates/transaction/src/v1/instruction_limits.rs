//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::limits::WASM_LIMITS;
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
/// counts them.
///
/// A node decodes to a fixed-size value however few bytes encode it, so, like the instruction count, this bounds
/// the memory decoding a transaction takes.
pub const MAX_TRANSACTION_ACCESS_RULE_NODES: usize = MAX_ACCESS_RULE_NODES;

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
}

/// Tallies the call arguments and access rule nodes a list of instructions carries, refusing an instruction that
/// takes the list past a limit.
///
/// Decoding an instruction list applies these limits, so a list that passes them is one every peer can decode.
#[derive(Debug, Clone, Default)]
pub struct InstructionLimitTracker {
    num_args: usize,
    num_access_rule_nodes: usize,
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
            Instruction::PutLastInstructionOutputOnWorkspace { .. } |
            Instruction::ClaimBurn { .. } |
            Instruction::ClaimValidatorFees { .. } |
            Instruction::DropAllProofsInWorkspace |
            Instruction::Assert { .. } |
            Instruction::TakeFromBucket { .. } |
            Instruction::PublishTemplate { .. } |
            Instruction::AllocateAddress { .. } |
            Instruction::StealthTransfer { .. } |
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
}

#[cfg(test)]
mod tests {
    use tari_template_lib_types::{
        AccessRule,
        access_rules::{ComponentAccessRules, RestrictedAccessRule},
        crypto::RistrettoPublicKeyBytes,
    };

    use super::*;
    use crate::args::InstructionArg;

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
