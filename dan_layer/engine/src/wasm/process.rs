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

use serde::{de::DeserializeOwned, Serialize};
use tari_bor::{decode_exact, encode, encode_into, encode_with_len};
use tari_engine_types::{indexed_value::IndexedValue, instruction_result::InstructionResult};
use tari_template_abi::{CallInfo, EngineOp, FunctionDef};
use tari_template_lib::{
    args::{
        BucketInvokeArg,
        BuiltinTemplateInvokeArg,
        CallInvokeArg,
        CallerContextInvokeArg,
        ComponentInvokeArg,
        ConsensusInvokeArg,
        EmitEventArg,
        EmitLogArg,
        GenerateRandomInvokeArg,
        LogLevel,
        NonFungibleInvokeArg,
        ProofInvokeArg,
        ResourceInvokeArg,
        VaultInvokeArg,
        WorkspaceInvokeArg,
    },
    AbiContext,
};
use wasmer::{Function, Instance, Module, Val, WasmerEnv};

use crate::{
    runtime::Runtime,
    traits::Invokable,
    wasm::{
        environment::{AllocPtr, WasmEnv},
        error::WasmExecutionError,
        LoadedWasmTemplate,
    },
};

use super::version::are_versions_compatible;

const LOG_TARGET: &str = "tari::dan::engine::wasm::process";
pub const ENGINE_TARI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
pub struct WasmProcess {
    module: LoadedWasmTemplate,
    env: WasmEnv<Runtime>,
    instance: Instance,
}

impl WasmProcess {
    pub fn start(module: LoadedWasmTemplate, state: Runtime) -> Result<Self, WasmExecutionError> {
        let mut env = WasmEnv::new(state);
        let store = module.wasm_module().store();
        let tari_engine = Function::new_native_with_env(store, env.clone(), Self::tari_engine_entrypoint);
        let resolver = env.create_resolver(store, tari_engine);
        let instance = Instance::new(module.wasm_module(), &resolver)?;
        Self::validate_template_tari_version(&module)?;
        env.init_with_instance(&instance)?;
        Ok(Self { module, env, instance })
    }

    fn alloc_and_write<T: Serialize>(&self, val: &T) -> Result<AllocPtr, WasmExecutionError> {
        let mut buf = Vec::with_capacity(512);
        encode_into(val, &mut buf).unwrap();
        let ptr = self.env.alloc(buf.len() as u32)?;
        self.env.write_to_memory(&ptr, &buf)?;

        Ok(ptr)
    }

    pub fn wasm_module(&self) -> &Module {
        self.module.wasm_module()
    }

    fn tari_engine_entrypoint(env: &WasmEnv<Runtime>, op: i32, arg_ptr: i32, arg_len: i32) -> i32 {
        let arg = match env.read_from_memory(arg_ptr as u32, arg_len as u32) {
            Ok(arg) => arg,
            Err(err) => {
                log::error!(target: LOG_TARGET, "Failed to read from memory: {}", err);
                return 0;
            },
        };

        let op = match EngineOp::from_i32(op) {
            Some(op) => op,
            None => {
                log::error!(target: LOG_TARGET, "Invalid opcode: {}", op);
                return 0;
            },
        };

        log::debug!(target: LOG_TARGET, "Engine call: {:?}", op);

        let result = match op {
            EngineOp::EmitLog => Self::handle(env, arg, |env, arg: EmitLogArg| {
                env.state().interface().emit_log(arg.level, arg.message)
            }),
            EngineOp::ComponentInvoke => Self::handle(env, arg, |env, arg: ComponentInvokeArg| {
                env.state()
                    .interface()
                    .component_invoke(arg.component_ref, arg.action, arg.args.into())
            }),
            EngineOp::ResourceInvoke => Self::handle(env, arg, |env, arg: ResourceInvokeArg| {
                env.state()
                    .interface()
                    .resource_invoke(arg.resource_ref, arg.action, arg.args.into())
            }),
            EngineOp::VaultInvoke => Self::handle(env, arg, |env, arg: VaultInvokeArg| {
                env.state()
                    .interface()
                    .vault_invoke(arg.vault_ref, arg.action, arg.args.into())
            }),
            EngineOp::BucketInvoke => Self::handle(env, arg, |env, arg: BucketInvokeArg| {
                env.state()
                    .interface()
                    .bucket_invoke(arg.bucket_ref, arg.action, arg.args.into())
            }),
            EngineOp::WorkspaceInvoke => Self::handle(env, arg, |env, arg: WorkspaceInvokeArg| {
                env.state().interface().workspace_invoke(arg.action, arg.args.into())
            }),
            EngineOp::NonFungibleInvoke => Self::handle(env, arg, |env, arg: NonFungibleInvokeArg| {
                env.state()
                    .interface()
                    .non_fungible_invoke(arg.address, arg.action, arg.args.into())
            }),
            EngineOp::GenerateUniqueId => {
                Self::handle(env, arg, |env, _arg: ()| env.state().interface().generate_uuid())
            },
            EngineOp::ConsensusInvoke => Self::handle(env, arg, |env, arg: ConsensusInvokeArg| {
                env.state().interface().consensus_invoke(arg.action)
            }),
            EngineOp::CallerContextInvoke => Self::handle(env, arg, |env, arg: CallerContextInvokeArg| {
                env.state().interface().caller_context_invoke(arg.action)
            }),
            EngineOp::GenerateRandomInvoke => Self::handle(env, arg, |env, arg: GenerateRandomInvokeArg| {
                env.state().interface().generate_random_invoke(arg.action)
            }),
            EngineOp::EmitEvent => Self::handle(env, arg, |env, arg: EmitEventArg| {
                env.state().interface().emit_event(arg.topic, arg.payload)
            }),
            EngineOp::CallInvoke => Self::handle(env, arg, |env, arg: CallInvokeArg| {
                env.state().interface().call_invoke(arg.action, arg.args.into())
            }),
            EngineOp::ProofInvoke => Self::handle(env, arg, |env, arg: ProofInvokeArg| {
                log::debug!(target: LOG_TARGET, "proof action = {:?}", arg.action);
                env.state()
                    .interface()
                    .proof_invoke(arg.proof_ref, arg.action, arg.args.into())
            }),
            EngineOp::BuiltinTemplateInvoke => Self::handle(env, arg, |env, arg: BuiltinTemplateInvokeArg| {
                env.state().interface().builtin_template_invoke(arg.action)
            }),
        };

        result.unwrap_or_else(|err| {
            if let Err(err) = env
                .state()
                .interface()
                .emit_log(LogLevel::Error, format!("Execution error: {}", err))
            {
                log::error!(target: LOG_TARGET, "Error emitting log: {}", err);
            }

            log::error!(target: LOG_TARGET, "{}", err);
            if let WasmExecutionError::RuntimeError(e) = err {
                env.set_last_engine_error(e);
            }
            0
        })
    }

    pub fn handle<T, U, E>(
        env: &WasmEnv<Runtime>,
        args: Vec<u8>,
        f: fn(&WasmEnv<Runtime>, T) -> Result<U, E>,
    ) -> Result<i32, WasmExecutionError>
    where
        T: DeserializeOwned,
        U: Serialize,
        WasmExecutionError: From<E>,
    {
        let decoded = decode_exact(&args).map_err(WasmExecutionError::EngineArgDecodeFailed)?;
        let resp = f(env, decoded)?;
        let encoded = encode_with_len(&resp);
        let ptr = env.alloc(encoded.len() as u32)?;
        env.write_to_memory(&ptr, &encoded)?;
        // TODO: It's not clear how/if this memory is freed. When I drop it on the WASM side I get an
        //       out-of-bounds access error.
        Ok(ptr.as_i32())
    }

    fn encoded_abi_context(&self) -> Vec<u8> {
        encode(&AbiContext {}).unwrap()
    }

    /// Determine if the version of the template_lib crate in the WASM is valid.
    /// This is just a placeholder that logs the result, as we don't manage version incompatiblities yet
    fn validate_template_tari_version(module: &LoadedWasmTemplate) -> Result<(), WasmExecutionError> {
        let template_tari_version = module.template_def().tari_version();

        if are_versions_compatible(template_tari_version, ENGINE_TARI_VERSION)? {
            log::info!(target: LOG_TARGET, "The Tari version in the template WASM (\"{}\") is compatible with the one used in the engine", template_tari_version);
        } else {
            log::error!(target: LOG_TARGET, "The Tari version in the template WASM (\"{}\") is incompatible with the one used in the engine (\"{}\")", template_tari_version, ENGINE_TARI_VERSION);
            return Err(WasmExecutionError::TemplateVersionMismatch { engine_version: ENGINE_TARI_VERSION.to_owned(), template_version: template_tari_version.to_owned() });
        }

        Ok(())
    }
}

impl Invokable for WasmProcess {
    type Error = WasmExecutionError;

    fn invoke(&self, func_def: &FunctionDef, args: Vec<tari_bor::Value>) -> Result<InstructionResult, Self::Error> {
        let call_info = CallInfo {
            abi_context: self.encoded_abi_context(),
            func_name: func_def.name.clone(),
            args,
        };

        let main_name = format!("{}_main", self.module.template_name());
        let func = self.instance.exports.get_function(&main_name)?;

        let call_info_ptr = self.alloc_and_write(&call_info)?;
        let res = func.call(&[Val::I32(call_info_ptr.as_i32()), Val::I32(call_info_ptr.len() as i32)]);
        self.env.free(call_info_ptr)?;

        let val = match res {
            Ok(res) => res,
            Err(err) => {
                if let Some(err) = self.env.take_last_engine_error() {
                    return Err(WasmExecutionError::RuntimeError(err));
                }
                if let Some(message) = self.env.take_last_panic_message() {
                    return Err(WasmExecutionError::Panic {
                        message,
                        runtime_error: err,
                    });
                }
                eprintln!("Error calling function: {}", err);
                return Err(err.into());
            },
        };
        let ptr = val
            .first()
            .and_then(|v| v.i32())
            .ok_or(WasmExecutionError::ExpectedPointerReturn { function: main_name })?;

        // Read response from memory
        let raw = self.env.read_memory_with_embedded_len(ptr as u32)?;

        let value = IndexedValue::from_raw(&raw)?;

        self.env
            .state()
            .interface()
            .set_last_instruction_output(value.clone())?;

        Ok(InstructionResult {
            indexed: value,
            return_type: func_def.output.clone(),
        })
    }
}
