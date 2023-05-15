//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use std::collections::{HashMap, HashSet};

use syn::Lit;
use tari_engine_types::{instruction::Instruction, substate::SubstateAddress, TemplateAddress};
use tari_template_lib::{
    arg,
    args::Arg,
    models::{Amount, NonFungibleId},
};

use crate::{
    ast::ManifestAst,
    error::ManifestError,
    parser::{InvokeIntent, ManifestIntent, ManifestLiteral, SpecialLiteral},
    ManifestValue,
};

pub struct ManifestInstructionGenerator {
    imported_templates: HashMap<String, TemplateAddress>,
    global_aliases: HashMap<String, ManifestValue>,
    globals: HashMap<String, ManifestValue>,
    variables: HashSet<String>,
}

impl ManifestInstructionGenerator {
    pub fn new(globals: HashMap<String, ManifestValue>) -> Self {
        Self {
            imported_templates: HashMap::new(),
            global_aliases: HashMap::new(),
            globals,
            variables: HashSet::new(),
        }
    }

    pub fn generate_instructions(&mut self, ast: ManifestAst) -> Result<Vec<Instruction>, ManifestError> {
        let mut instructions = Vec::with_capacity(ast.intents.len());
        for intent in ast.intents {
            instructions.extend(self.translate_intent(intent)?);
        }

        Ok(instructions)
    }

    fn translate_intent(&mut self, intent: ManifestIntent) -> Result<Vec<Instruction>, ManifestError> {
        match intent {
            ManifestIntent::DefineTemplate {
                template_address,
                alias,
            } => {
                self.imported_templates.insert(alias.to_string(), template_address);
                Ok(vec![])
            },
            ManifestIntent::InvokeTemplate(InvokeIntent {
                output_variable,
                template_variable,
                function_name,
                arguments,
                ..
            }) => {
                let template_ident = template_variable
                    .as_ref()
                    .expect("AST parse should have failed: no template ident for TemplateInvoke statement");
                let mut instructions = vec![Instruction::CallFunction {
                    template_address: self.get_imported_template(&template_ident.to_string())?,
                    function: function_name.to_string(),
                    args: self.process_args(arguments)?,
                }];
                if let Some(var_name) = output_variable {
                    self.variables.insert(var_name.to_string());
                    instructions.push(Instruction::PutLastInstructionOutputOnWorkspace {
                        key: var_name.to_string().into_bytes(),
                    });
                }
                Ok(instructions)
            },
            ManifestIntent::InvokeComponent(InvokeIntent {
                output_variable,
                component_variable,
                function_name,
                arguments,
                ..
            }) => {
                let component_ident = component_variable
                    .as_ref()
                    .expect("AST parse should have failed: no component ident for ComponentInvoke statement")
                    .to_string();
                let component_address = self
                    .get_variable(&component_ident)?
                    .as_address()
                    .and_then(|addr| addr.as_component_address())
                    .ok_or_else(|| {
                        ManifestError::InvalidVariableType(format!(
                            "Expected component variable but got {:?}",
                            self.get_variable(&component_ident)
                        ))
                    })?;
                let mut instructions = vec![Instruction::CallMethod {
                    component_address,
                    method: function_name.to_string(),
                    args: self.process_args(arguments)?,
                }];
                if let Some(var_name) = output_variable {
                    self.variables.insert(var_name.to_string());
                    instructions.push(Instruction::PutLastInstructionOutputOnWorkspace {
                        key: var_name.to_string().into_bytes(),
                    });
                }
                Ok(instructions)
            },
            ManifestIntent::AssignInput(assign) => {
                self.global_aliases.insert(
                    assign.variable_name.to_string(),
                    self.get_global(&assign.global_variable_name.value())?.clone(),
                );
                Ok(vec![])
            },
            ManifestIntent::Log(log) => Ok(vec![Instruction::EmitLog {
                level: log.level,
                message: log.message,
            }]),
        }
    }

    fn process_args(&self, args: Vec<ManifestLiteral>) -> Result<Vec<Arg>, ManifestError> {
        args.into_iter()
            .map(|arg| match arg {
                ManifestLiteral::Lit(lit) => lit_to_arg(&lit),
                ManifestLiteral::Variable(ident) => {
                    // Is it a global?
                    self.globals
                        .get(&ident.to_string())
                        .or_else(|| self.global_aliases.get(&ident.to_string()))
                        .map(|v| match v {
                            ManifestValue::SubstateAddress(addr) => match addr {
                                SubstateAddress::Component(addr) => Ok(arg!(*addr)),
                                SubstateAddress::Resource(addr) => Ok(arg!(*addr)),
                                // TODO: should tx receipt addresses be allowed to be reference ?
                                SubstateAddress::TransactionReceipt(addr) => Ok(arg!(*addr)),
                                SubstateAddress::Vault(addr) => Ok(arg!(*addr)),
                                SubstateAddress::NonFungible(addr) => Ok(arg!(addr)),
                                SubstateAddress::UnclaimedConfidentialOutput(addr) => Ok(arg!(*addr)),
                                SubstateAddress::NonFungibleIndex(addr) => Ok(arg!(addr)),
                            },
                            ManifestValue::Literal(lit) => lit_to_arg(lit),
                            ManifestValue::NonFungibleId(id) => Ok(arg!(id.clone())),
                            ManifestValue::Value(blob) => Ok(Arg::Literal(blob.clone())),
                        })
                        .or_else(|| {
                            // Or is it a variable on the worktop?
                            if self.variables.contains(&ident.to_string()) {
                                Some(Ok(Arg::Workspace(ident.to_string().into_bytes())))
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| {
                            // Or undefined
                            ManifestError::UndefinedVariable {
                                name: ident.to_string(),
                            }
                        })?
                },
                ManifestLiteral::Special(SpecialLiteral::Amount(amount)) => Ok(arg!(Amount(amount))),
                ManifestLiteral::Special(SpecialLiteral::NonFungibleId(lit)) => {
                    let id = lit_to_nonfungible_id(&lit)?;
                    Ok(arg!(id))
                },
            })
            .collect()
    }

    fn get_imported_template(&self, name: &str) -> Result<TemplateAddress, ManifestError> {
        self.imported_templates
            .get(name)
            .copied()
            .ok_or_else(|| ManifestError::TemplateNotImported { name: name.to_string() })
    }

    fn get_variable(&self, name: &str) -> Result<&ManifestValue, ManifestError> {
        self.global_aliases
            .get(name)
            .ok_or_else(|| ManifestError::UndefinedVariable { name: name.to_string() })
    }

    fn get_global(&self, name: &str) -> Result<&ManifestValue, ManifestError> {
        self.globals
            .get(name)
            .ok_or_else(|| ManifestError::UndefinedGlobal { name: name.to_string() })
    }
}

fn lit_to_arg(lit: &Lit) -> Result<Arg, ManifestError> {
    match lit {
        Lit::Str(s) => Ok(arg!(s.value())),
        Lit::Int(i) => match i.suffix() {
            "u8" => Ok(arg!(i.base10_parse::<u8>()?)),
            "u16" => Ok(arg!(i.base10_parse::<u16>()?)),
            "u32" => Ok(arg!(i.base10_parse::<u32>()?)),
            "u64" => Ok(arg!(i.base10_parse::<u64>()?)),
            "u128" => Ok(arg!(i.base10_parse::<u128>()?)),
            "i8" => Ok(arg!(i.base10_parse::<i8>()?)),
            "i16" => Ok(arg!(i.base10_parse::<i16>()?)),
            "" | "i32" => Ok(arg!(i.base10_parse::<i32>()?)),
            "i64" => Ok(arg!(i.base10_parse::<i64>()?)),
            "i128" => Ok(arg!(i.base10_parse::<i128>()?)),
            _ => Err(ManifestError::UnsupportedExpr(format!(
                r#"Unsupported integer suffix "{}""#,
                i.suffix()
            ))),
        },
        Lit::Bool(b) => Ok(arg!(b.value())),
        Lit::ByteStr(v) => Ok(arg!(v.value())),
        Lit::Byte(v) => Ok(arg!(v.value())),
        Lit::Char(v) => Ok(arg!(v.value().to_string())),
        Lit::Float(v) => Err(ManifestError::UnsupportedExpr(format!(
            "Float literals not supported ({})",
            v
        ))),
        Lit::Verbatim(v) => Err(ManifestError::UnsupportedExpr(format!(
            "Raw token literals not supported ({})",
            v
        ))),
    }
}

fn lit_to_nonfungible_id(lit: &Lit) -> Result<NonFungibleId, ManifestError> {
    match lit {
        Lit::Str(s) => Ok(NonFungibleId::try_from_string(s.value()).map_err(|e| {
            ManifestError::UnsupportedExpr(format!(
                "Invalid non-fungible ID string literal ({:?}) ({})",
                e,
                s.value()
            ))
        })?),
        Lit::ByteStr(v) => {
            let bytes = v.value();
            if bytes.len() != 32 {
                return Err(ManifestError::UnsupportedExpr(
                    "Non-fungible ID byte string literal length must be less than 32 bytes".to_string(),
                ));
            }

            let mut id = [0u8; 32];
            id.copy_from_slice(&bytes);
            Ok(NonFungibleId::from_u256(id))
        },
        Lit::Int(v) => match v.suffix() {
            "u8" | "u16" | "u32" => Ok(NonFungibleId::from_u32(v.base10_parse()?)),
            "u64" => Ok(NonFungibleId::from_u64(v.base10_parse()?)),
            "" => Err(ManifestError::UnsupportedExpr(
                "Non-fungible ID integer literal must have a type suffix specified (1u32, 2u64 etc)".to_string(),
            )),
            _ => Err(ManifestError::UnsupportedExpr(format!(
                "Invalid non-fungible ID integer literal suffix ({})",
                v.suffix()
            ))),
        },
        _ => Err(ManifestError::UnsupportedExpr(format!(
            "Unsupported non-fungible ID literal ({:?})",
            lit
        ))),
    }
}
