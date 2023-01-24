//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use std::collections::{HashMap, HashSet};

use syn::Lit;
use tari_engine_types::{instruction::Instruction, substate::SubstateAddress, TemplateAddress};
use tari_template_lib::{arg, args::Arg};

use crate::{
    ast::ManifestAst,
    error::ManifestError,
    parser::{InvokeIntent, LiteralOrVariable, ManifestIntent},
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
                    // TODO: Replace this instruction with workspace assignments built into the CallFunction/CallMethod
                    // instructions
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
                    .address()
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
                    // TODO: Replace this instruction with workspace assignments built into the CallFunction/CallMethod
                    // instructions
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

    fn process_args(&self, args: Vec<LiteralOrVariable>) -> Result<Vec<Arg>, ManifestError> {
        fn lit_to_arg(lit: &Lit) -> Result<Arg, ManifestError> {
            match lit {
                Lit::Str(s) => Ok(arg!(s.value())),
                Lit::Int(i) => {
                    let suffix = Some(i.suffix()).filter(|s| s.is_empty()).unwrap_or("i32");
                    match suffix {
                        "u8" => return Ok(arg!(i.base10_parse::<u8>()?)),
                        "u16" => return Ok(arg!(i.base10_parse::<u16>()?)),
                        "u32" => return Ok(arg!(i.base10_parse::<u32>()?)),
                        "u64" => return Ok(arg!(i.base10_parse::<u64>()?)),
                        "u128" => return Ok(arg!(i.base10_parse::<u128>()?)),
                        "i8" => return Ok(arg!(i.base10_parse::<i8>()?)),
                        "i16" => return Ok(arg!(i.base10_parse::<i16>()?)),
                        "i32" => return Ok(arg!(i.base10_parse::<i32>()?)),
                        "i64" => return Ok(arg!(i.base10_parse::<i64>()?)),
                        "i128" => return Ok(arg!(i.base10_parse::<i128>()?)),
                        _ => {
                            return Err(ManifestError::UnsupportedExpr(format!(
                                "Unsupported integer suffix ({})",
                                suffix
                            )));
                        },
                    }
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

        args.into_iter()
            .map(|arg| match arg {
                LiteralOrVariable::Lit(lit) => lit_to_arg(&lit),
                LiteralOrVariable::Variable(ident) => {
                    // Is it a global?
                    self.globals
                        .get(&ident.to_string())
                        .or_else(|| self.global_aliases.get(&ident.to_string()))
                        .map(|v| match v {
                            ManifestValue::Address(addr) => match addr {
                                SubstateAddress::Component(addr) => Ok(arg!(*addr)),
                                SubstateAddress::Resource(addr) => Ok(arg!(*addr)),
                                SubstateAddress::Vault(addr) => Ok(arg!(*addr)),
                                SubstateAddress::NonFungible(_, id) => Ok(arg!(*id)),
                            },
                            ManifestValue::Literal(lit) => lit_to_arg(lit),
                        })
                        .or_else(|| {
                            // Or is it a variable on the worktop?
                            if self.variables.contains(&ident.to_string()) {
                                Some(Ok(Arg::Variable(ident.to_string().into_bytes())))
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
