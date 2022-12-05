// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use d3ne::WorkersBuilder;
use tari_engine_types::instruction::Instruction;

use crate::{
    flow::{FlowEngineError, FlowInstance},
    function_definitions::{FlowFunctionDefinition, FunctionArgDefinition},
};

#[derive(Clone, Default)]
pub struct FlowFactory {
    _flows: HashMap<String, (Vec<FunctionArgDefinition>, FlowInstance)>,
}
impl FlowFactory {
    pub fn new(flow_functions: &[FlowFunctionDefinition]) -> Self {
        let mut flows = HashMap::new();
        for func_def in flow_functions {
            // build_instance(&mut instance, &func_def);
            flows.insert(
                func_def.name.clone(),
                (
                    func_def.args.clone(),
                    FlowInstance::try_build(func_def.flow.clone(), WorkersBuilder::new().build())
                        .expect("Could not build flow"),
                ),
            );
        }
        Self { _flows: flows }
    }

    pub fn invoke_write_method<TUnitOfWork>(
        &self,
        _name: String,
        _instruction: &Instruction,
        _state_db: TUnitOfWork,
    ) -> Result<TUnitOfWork, FlowEngineError> {
        todo!()
        // if let Some((args, engine)) = self.flows.get(&name) {
        //     engine.process(instruction.args(), args, instruction.sender(), state_db)
        // } else {
        //     todo!("could not find engine")
        // }
    }
}
