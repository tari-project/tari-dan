// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use d3ne::WorkersBuilder;
use tari_engine_types::instruction::Instruction;
use tari_template_abi::{FunctionDef, TemplateDef, Type};

use crate::{
    flow::{FlowEngineError, FlowInstance},
    function_definitions::{FlowFunctionDefinition, FunctionArgDefinition},
};

#[derive(Debug, Clone)]
pub struct FlowFactory {
    name: String,
    args: Vec<FunctionArgDefinition>,
    flow: FlowInstance,
    template_def: TemplateDef,
}
impl FlowFactory {
    pub fn try_create(flow_definition: FlowFunctionDefinition) -> Result<Self, FlowEngineError> {
        let template_def = TemplateDef {
            template_name: flow_definition.name.clone(),
            functions: vec![FunctionDef {
                name: "main".to_string(),
                arguments: flow_definition.args.iter().map(|a| a.arg_type.to_type()).collect(),
                output: Type::Unit,
                is_mut: false,
            }],
        };
        Ok(Self {
            name: flow_definition.name,
            args: flow_definition.args,
            flow: FlowInstance::try_build(flow_definition.flow.clone(), WorkersBuilder::new().build())?,
            template_def,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn template_def(&self) -> &TemplateDef {
        &self.template_def
    }
}
