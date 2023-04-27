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
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason},
    instruction::Instruction,
    instruction_result::InstructionResult,
};
use tari_template_lib::{
    arg,
    args::{Arg, WorkspaceAction},
    invoke_args,
    models::{Amount, ComponentAddress},
};
use tari_transaction::{id_provider::IdProvider, Transaction};
use tari_utilities::ByteArray;

use crate::{
    packager::LoadedTemplate,
    runtime::{
        AuthParams,
        AuthorizationScope,
        ConsensusContext,
        FunctionIdent,
        Runtime,
        RuntimeInterfaceImpl,
        RuntimeModule,
        RuntimeState,
        StateTracker,
    },
    state_store::memory::MemoryStateStore,
    traits::Invokable,
    transaction::TransactionError,
    wasm::WasmProcess,
};

const LOG_TARGET: &str = "tari::dan::engine::instruction_processor";

pub struct TransactionProcessor<TTemplateProvider> {
    // package: Package,
    template_provider: Arc<TTemplateProvider>,
    state_db: MemoryStateStore,
    auth_params: AuthParams,
    consensus: ConsensusContext,
    modules: Vec<Box<dyn RuntimeModule<TTemplateProvider>>>,
    fee_loan: Amount,
}

impl<TTemplateProvider: TemplateProvider<Template = LoadedTemplate> + 'static> TransactionProcessor<TTemplateProvider> {
    pub fn new(
        template_provider: Arc<TTemplateProvider>,
        state_db: MemoryStateStore,
        auth_params: AuthParams,
        consensus: ConsensusContext,
        modules: Vec<Box<dyn RuntimeModule<TTemplateProvider>>>,
        fee_loan: Amount,
    ) -> Self {
        Self {
            template_provider,
            state_db,
            auth_params,
            consensus,
            modules,
            fee_loan,
        }
    }

    pub fn execute(self, transaction: Transaction) -> Result<ExecuteResult, TransactionError> {
        let id_provider = IdProvider::new(*transaction.hash(), 1000);
        // TODO: We can avoid this for each execution with improved design
        let tracker = StateTracker::new(self.state_db.clone(), id_provider, self.template_provider.clone());
        let initial_proofs = self.auth_params.initial_ownership_proofs.clone();
        let template_provider = self.template_provider.clone();
        let runtime_interface = RuntimeInterfaceImpl::initialize(
            tracker,
            self.auth_params,
            self.consensus,
            transaction.sender_public_key().clone(),
            self.modules,
            self.fee_loan,
        )?;

        let auth_scope = AuthorizationScope::new(Arc::new(initial_proofs));
        let runtime = Runtime::new(Arc::new(runtime_interface));
        let transaction_hash = *transaction.hash();

        let (instructions, fee_instructions, _sig, _pk) = transaction.destruct();

        let fee_exec_results = fee_instructions
            .into_iter()
            .map(|instruction| {
                Self::process_instruction(template_provider.clone(), &runtime, auth_scope.clone(), instruction)
            })
            .collect::<Result<Vec<_>, _>>();

        let fee_exec_result = match fee_exec_results {
            Ok(execution_results) => {
                // Checkpoint the tracker state after the fee instructions have been executed in case of transaction
                // failure.
                if let Err(err) = runtime.interface().fee_checkpoint() {
                    let mut finalize =
                        FinalizeResult::reject(transaction_hash, RejectReason::ExecutionFailure(err.to_string()));
                    finalize.execution_results = execution_results;
                    return Ok(ExecuteResult {
                        fee_receipt: None,
                        finalize,
                        transaction_failure: Some(RejectReason::FeeTransactionFailed),
                    });
                }
                execution_results
            },
            Err(err) => {
                return Ok(ExecuteResult {
                    fee_receipt: None,
                    finalize: FinalizeResult::reject(transaction_hash, RejectReason::ExecutionFailure(err.to_string())),
                    transaction_failure: Some(RejectReason::FeeTransactionFailed),
                });
            },
        };

        let instruction_result = instructions
            .into_iter()
            .map(|instruction| {
                Self::process_instruction(template_provider.clone(), &runtime, auth_scope.clone(), instruction)
            })
            .collect::<Result<Vec<_>, _>>();

        match instruction_result {
            Ok(execution_results) => {
                let (mut finalize, fee_receipt) = runtime.interface().finalize()?;

                if !fee_receipt.is_paid_in_full() && fee_receipt.total_fees_charged() > self.fee_loan {
                    return Ok(ExecuteResult {
                        finalize,
                        transaction_failure: Some(RejectReason::FeesNotPaid(format!(
                            "Required fees {} but {} paid",
                            fee_receipt.total_fees_charged(),
                            fee_receipt.total_fees_paid()
                        ))),
                        fee_receipt: Some(fee_receipt),
                    });
                }
                finalize.execution_results = execution_results;

                Ok(ExecuteResult {
                    finalize,
                    fee_receipt: Some(fee_receipt),
                    transaction_failure: None,
                })
            },
            // This can happen e.g if you have dangling buckets after running the instructions
            Err(err) => {
                // Reset the state to when the state at the end of the fee instructions. The fee charges for the
                // successful instructions are still charged even though the transaction failed.
                runtime.interface().reset_to_fee_checkpoint()?;
                // Finalize will now contain the fee payments and vault refunds only
                let (mut finalize, fee_payment) = runtime.interface().finalize()?;
                finalize.execution_results = fee_exec_result;

                Ok(ExecuteResult {
                    finalize,
                    fee_receipt: Some(fee_payment),
                    transaction_failure: Some(RejectReason::ExecutionFailure(err.to_string())),
                })
            },
        }
    }

    fn process_instruction(
        template_provider: Arc<TTemplateProvider>,
        runtime: &Runtime,
        auth_scope: AuthorizationScope,
        instruction: Instruction,
    ) -> Result<InstructionResult, TransactionError> {
        debug!(target: LOG_TARGET, "instruction = {:?}", instruction);
        match instruction {
            Instruction::CallFunction {
                template_address,
                function,
                args,
            } => {
                runtime
                    .interface()
                    .set_current_runtime_state(RuntimeState { template_address })?;

                let template = template_provider.get_template_module(&template_address).map_err(|_e| {
                    TransactionError::TemplateNotFound {
                        address: template_address,
                    }
                })?;

                let result = Self::invoke_template(
                    template,
                    template_provider,
                    runtime.clone(),
                    auth_scope,
                    &function,
                    args,
                    0,
                    1,
                )?;
                Ok(result)
            },
            Instruction::CallMethod {
                component_address,
                method,
                args,
            } => Self::call_method(
                template_provider,
                runtime,
                auth_scope,
                &component_address,
                &method,
                args,
                0,
                1,
            ),
            // Basically names an output on the workspace so that you can refer to it as an
            // Arg::Variable
            Instruction::PutLastInstructionOutputOnWorkspace { key } => {
                Self::put_output_on_workspace_with_name(runtime, key)?;
                Ok(InstructionResult::empty())
            },
            Instruction::EmitLog { level, message } => {
                runtime.interface().emit_log(level, message)?;
                Ok(InstructionResult::empty())
            },
            Instruction::EmitEvent { message } => {
                runtime.interface().emit_event(message)?;
                Ok(InstructionResult::empty())
            },
            Instruction::ClaimBurn { claim } => {
                // Need to call it on the runtime so that a bucket is created.
                runtime.interface().claim_burn(*claim)?;
                Ok(InstructionResult::empty())
            },
            Instruction::CreateFreeTestCoins { amount, private_key } => {
                runtime
                    .interface()
                    .create_free_test_coins(amount, RistrettoSecretKey::from_bytes(&private_key)?)?;
                Ok(InstructionResult::empty())
            },
        }
    }

    pub fn put_output_on_workspace_with_name(runtime: &Runtime, key: Vec<u8>) -> Result<(), TransactionError> {
        runtime
            .interface()
            .workspace_invoke(WorkspaceAction::PutLastInstructionOutput, invoke_args![key].into())?;
        Ok(())
    }

    pub fn call_method(
        template_provider: Arc<TTemplateProvider>,
        runtime: &Runtime,
        auth_scope: AuthorizationScope,
        component_address: &ComponentAddress,
        method: &str,
        args: Vec<Arg>,
        recursion_depth: usize,
        max_recursion_depth: usize,
    ) -> Result<InstructionResult, TransactionError> {
        let component = runtime.interface().get_component(component_address)?;
        // TODO: In this very basic auth system, you can only call on owned objects (because
        // initial_ownership_proofs is       usually set to include the owner token).
        auth_scope.check_access_rules(
            &FunctionIdent::Template {
                module_name: component.module_name.clone(),
                function: method.to_string(),
            },
            &component.access_rules,
        )?;

        let template = template_provider
            .get_template_module(&component.template_address)
            .map_err(|_e| TransactionError::TemplateNotFound {
                address: component.template_address,
            })?;

        runtime.interface().set_current_runtime_state(RuntimeState {
            template_address: component.template_address,
        })?;

        let mut final_args = Vec::with_capacity(args.len() + 1);
        final_args.push(arg![component_address]);
        final_args.extend(args);

        let result = Self::invoke_template(
            template,
            template_provider,
            runtime.clone(),
            auth_scope,
            method,
            final_args,
            recursion_depth,
            max_recursion_depth,
        )?;
        Ok(result)
    }

    fn invoke_template(
        module: LoadedTemplate,
        template_provider: Arc<TTemplateProvider>,
        runtime: Runtime,
        auth_scope: AuthorizationScope,
        function: &str,
        args: Vec<Arg>,
        recursion_depth: usize,
        max_recursion_depth: usize,
    ) -> Result<InstructionResult, TransactionError> {
        if recursion_depth > max_recursion_depth {
            return Err(TransactionError::RecursionLimitExceeded);
        }
        let result = match module {
            LoadedTemplate::Wasm(wasm_module) => {
                let process = WasmProcess::start(wasm_module, runtime)?;
                process.invoke_by_name(function, args)?
            },
            LoadedTemplate::Flow(flow_factory) => flow_factory.run_new_instance(
                template_provider,
                runtime,
                auth_scope,
                function,
                args,
                recursion_depth,
                max_recursion_depth,
            )?,
        };
        Ok(result)
    }
}
