//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::sync::Arc;

use log::*;
use tari_engine_types::{commit_result::FinalizeResult, execution_result::ExecutionResult, instruction::Instruction};
use tari_template_lib::{
    arg,
    args::{Arg, WorkspaceAction},
    invoke_args,
};

use crate::{
    packager::{LoadedTemplate, Package},
    runtime::{Runtime, RuntimeInterface, RuntimeState},
    traits::Invokable,
    transaction::{Transaction, TransactionError},
    wasm::WasmProcess,
};

const LOG_TARGET: &str = "dan::engine::instruction_processor";

#[derive(Debug, Clone)]
pub struct TransactionProcessor<TRuntimeInterface> {
    package: Package,
    runtime_interface: TRuntimeInterface,
}

impl<TRuntimeInterface> TransactionProcessor<TRuntimeInterface>
where TRuntimeInterface: RuntimeInterface + Clone + 'static
{
    pub fn new(runtime_interface: TRuntimeInterface, package: Package) -> Self {
        Self {
            package,
            runtime_interface,
        }
    }

    pub fn execute(&self, transaction: Transaction) -> Result<FinalizeResult, TransactionError> {
        let runtime = Runtime::new(Arc::new(self.runtime_interface.clone()));
        let exec_results = transaction
            .instructions
            .into_iter()
            .map(|instruction| self.process_instruction(&runtime, instruction))
            .collect::<Result<Vec<_>, _>>()?;

        let mut finalize_result = runtime.interface().finalize()?;
        finalize_result.execution_results = exec_results;
        Ok(finalize_result)
    }

    fn process_instruction(
        &self,
        runtime: &Runtime,
        instruction: Instruction,
    ) -> Result<ExecutionResult, TransactionError> {
        debug!(target: LOG_TARGET, "instruction = {:?}", instruction);
        match instruction {
            Instruction::CallFunction {
                template_address,
                function,
                args,
            } => {
                runtime
                    .interface()
                    .set_current_runtime_state(RuntimeState { template_address });

                let template = self.package.get_template_by_address(&template_address).ok_or(
                    TransactionError::TemplateNotFound {
                        address: template_address,
                    },
                )?;

                let result = self.invoke_template(template.clone(), runtime.clone(), &function, args)?;
                Ok(result)
            },
            Instruction::CallMethod {
                component_address,
                method,
                args,
            } => {
                let component = self.runtime_interface.get_component(&component_address)?;
                let template = self
                    .package
                    .get_template_by_address(&component.template_address)
                    .ok_or(TransactionError::TemplateNotFound {
                        address: component.template_address,
                    })?;

                runtime.interface().set_current_runtime_state(RuntimeState {
                    template_address: component.template_address,
                });

                let mut final_args = Vec::with_capacity(args.len() + 1);
                final_args.push(arg![component]);
                final_args.extend(args);

                let result = self.invoke_template(template.clone(), runtime.clone(), &method, final_args)?;
                Ok(result)
            },
            Instruction::PutLastInstructionOutputOnWorkspace { key } => {
                let _result = runtime
                    .interface()
                    .workspace_invoke(WorkspaceAction::PutLastInstructionOutput, invoke_args![key])?;
                Ok(ExecutionResult::empty())
            },
            Instruction::EmitLog { level, message } => {
                runtime.interface().emit_log(level, message);
                Ok(ExecutionResult::empty())
            },
            Instruction::ClaimBurn {
                commitment_address,
                range_proof,
                proof_of_knowledge,
            } => {
                // todo: Check signature. Where should that fail?

                // Need to call it on the runtime so that a bucket is created.
                runtime
                    .interface()
                    .claim_burn(commitment_address, range_proof, proof_of_knowledge)?;
                Ok(ExecutionResult::empty())
            },
        }
    }

    fn invoke_template(
        &self,
        module: LoadedTemplate,
        runtime: Runtime,
        function: &str,
        args: Vec<Arg>,
    ) -> Result<ExecutionResult, TransactionError> {
        let result = match module {
            LoadedTemplate::Wasm(wasm_module) => {
                // TODO: implement intelligent instance caching
                let process = WasmProcess::start(wasm_module, runtime)?;
                process.invoke_by_name(function, args)?
            },
        };
        Ok(result)
    }
}
