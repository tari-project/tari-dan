// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use d3ne::WorkersBuilder;
use serde_json::Value as JsValue;
use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_engine_types::instruction_result::InstructionResult;
use tari_template_abi::{ArgDef, FunctionDef, TemplateDef, Type};
use tari_template_lib::args::Arg;

use crate::{
    flow::{FlowContext, FlowEngineError, FlowInstance},
    function_definitions::{FlowFunctionDefinition, FunctionArgDefinition},
    packager::LoadedTemplate,
    runtime::{AuthorizationScope, Runtime},
};

#[derive(Debug, Clone)]
pub struct FlowFactory {
    name: String,
    args: Vec<FunctionArgDefinition>,
    flow_definition: JsValue,
    template_def: TemplateDef,
}
impl FlowFactory {
    pub fn try_create<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
        flow_definition: FlowFunctionDefinition,
    ) -> Result<Self, FlowEngineError> {
        let template_def = TemplateDef {
            template_name: flow_definition.name.clone(),
            functions: vec![FunctionDef {
                name: "main".to_string(),
                arguments: flow_definition
                    .args
                    .iter()
                    .map(|a| ArgDef {
                        name: a.name.clone(),
                        arg_type: a.arg_type.to_type(),
                    })
                    .collect(),
                output: Type::Unit,
                is_mut: false,
            }],
        };

        let _test_build = FlowInstance::try_build(
            flow_definition.flow.clone(),
            WorkersBuilder::<FlowContext<TTemplateProvider>>::default().build(),
        )?;
        Ok(Self {
            name: flow_definition.name,
            args: flow_definition.args,
            flow_definition: flow_definition.flow,
            template_def,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn template_def(&self) -> &TemplateDef {
        &self.template_def
    }

    pub fn run_new_instance<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
        &self,
        template_provider: Arc<TTemplateProvider>,
        runtime: Runtime,
        auth_scope: AuthorizationScope,
        // In future we might allow calling different functions in a flow
        _function: &str,
        args: Vec<Arg>,
        recursion_depth: usize,
        max_recursion_depth: usize,
    ) -> Result<InstructionResult, FlowEngineError> {
        let new_instance = FlowInstance::try_build::<TTemplateProvider>(
            self.flow_definition.clone(),
            WorkersBuilder::default().build(),
        )?;
        new_instance.invoke(
            template_provider,
            runtime,
            auth_scope,
            &args,
            &self.args,
            recursion_depth,
            max_recursion_depth,
        )
    }
}
