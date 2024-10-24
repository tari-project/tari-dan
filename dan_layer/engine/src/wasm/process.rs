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
use tari_bor::{decode_exact, encode, encode_into_writer, encode_with_len_to_writer, encoded_len};
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
use wasmer::{imports, AsStoreMut, Function, FunctionEnv, FunctionEnvMut, Instance, Store, StoreMut, WasmPtr};

use super::version::are_versions_compatible;
use crate::{
    runtime::Runtime,
    traits::Invokable,
    wasm::{
        environment::{AllocPtr, WasmEnv},
        error::WasmExecutionError,
        module::MainFunction,
        LoadedWasmTemplate,
    },
};

const LOG_TARGET: &str = "tari::dan::engine::wasm::process";
pub const ENGINE_TARI_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct WasmProcess {
    module: LoadedWasmTemplate,
    env: WasmEnv<Runtime>,
    instance: Instance,
}

impl WasmProcess {
    pub fn init(store: &mut Store, module: LoadedWasmTemplate, state: Runtime) -> Result<Self, WasmExecutionError> {
        Self::validate_template_tari_version(&module)?;

        let mut env = WasmEnv::new(state);
        let fn_env = FunctionEnv::new(store, env.clone());
        let tari_engine = Function::new_typed_with_env(store, &fn_env, Self::tari_engine_entrypoint);

        let imports = imports! {
            "env" => {
                "tari_engine" => tari_engine,
                "debug" => Function::new_typed_with_env(store, &fn_env, debug_handler),
                "on_panic" => Function::new_typed_with_env(store,&fn_env, on_panic_handler),
            }
        };
        let instance = Instance::new(store, module.wasm_module(), &imports)?;
        let memory = instance.exports.get_memory("memory")?.clone();
        let mem_alloc = instance.exports.get_typed_function(store, "tari_alloc")?;
        fn_env
            .as_mut(store)
            .set_memory(memory.clone())
            .set_alloc_funcs(mem_alloc.clone());

        // Also set these for the local copy
        env.set_memory(memory).set_alloc_funcs(mem_alloc);

        Ok(Self { module, env, instance })
    }

    fn alloc_and_write<T: Serialize, S: AsStoreMut>(
        &self,
        store: &mut S,
        val: &T,
    ) -> Result<AllocPtr, WasmExecutionError> {
        let len = encoded_len(val).unwrap();
        let len = u32::try_from(len).map_err(|_| WasmExecutionError::MemoryAllocationTooLarge)?;

        let ptr = self.env.alloc(store, len)?;
        if ptr.is_null() {
            return Err(WasmExecutionError::MemoryAllocationFailed);
        }
        let mut writer = self.env.memory_writer(store, ptr)?;
        encode_into_writer(val, &mut writer).unwrap();

        Ok(AllocPtr::new(ptr.offset(), len))
    }

    fn tari_engine_entrypoint(
        mut env: FunctionEnvMut<WasmEnv<Runtime>>,
        op: i32,
        arg_ptr: WasmPtr<u8>,
        arg_len: u32,
    ) -> WasmPtr<u8> {
        let op = match EngineOp::from_i32(op) {
            Some(op) => op,
            None => {
                log::error!(target: LOG_TARGET, "Invalid opcode: {}", op);
                return WasmPtr::null();
            },
        };

        let (env_mut, mut store) = env.data_and_store_mut();
        let arg = match env_mut.read_from_memory(&mut store, arg_ptr, arg_len) {
            Ok(arg) => arg,
            Err(err) => {
                log::error!(target: LOG_TARGET, "Failed to read from memory: {}", err);
                return WasmPtr::null();
            },
        };

        log::debug!(target: LOG_TARGET, "Engine call: {:?}", op);

        let result = match op {
            EngineOp::EmitLog => Self::handle(store, env_mut, arg, |env, arg: EmitLogArg| {
                env.interface().emit_log(arg.level, arg.message)
            }),
            EngineOp::ComponentInvoke => Self::handle(store, env_mut, arg, |env, arg: ComponentInvokeArg| {
                env.interface()
                    .component_invoke(arg.component_ref, arg.action, arg.args.into())
            }),
            EngineOp::ResourceInvoke => Self::handle(store, env_mut, arg, |env, arg: ResourceInvokeArg| {
                env.interface()
                    .resource_invoke(arg.resource_ref, arg.action, arg.args.into())
            }),
            EngineOp::VaultInvoke => Self::handle(store, env_mut, arg, |env, arg: VaultInvokeArg| {
                env.interface().vault_invoke(arg.vault_ref, arg.action, arg.args.into())
            }),
            EngineOp::BucketInvoke => Self::handle(store, env_mut, arg, |env, arg: BucketInvokeArg| {
                env.interface()
                    .bucket_invoke(arg.bucket_ref, arg.action, arg.args.into())
            }),
            EngineOp::WorkspaceInvoke => Self::handle(store, env_mut, arg, |env, arg: WorkspaceInvokeArg| {
                env.interface().workspace_invoke(arg.action, arg.args.into())
            }),
            EngineOp::NonFungibleInvoke => Self::handle(store, env_mut, arg, |env, arg: NonFungibleInvokeArg| {
                env.interface()
                    .non_fungible_invoke(arg.address, arg.action, arg.args.into())
            }),
            EngineOp::GenerateUniqueId => {
                Self::handle(store, env_mut, arg, |env, _arg: ()| env.interface().generate_uuid())
            },
            EngineOp::ConsensusInvoke => Self::handle(store, env_mut, arg, |env, arg: ConsensusInvokeArg| {
                env.interface().consensus_invoke(arg.action)
            }),
            EngineOp::CallerContextInvoke => Self::handle(store, env_mut, arg, |env, arg: CallerContextInvokeArg| {
                env.interface().caller_context_invoke(arg.action, arg.args.into())
            }),
            EngineOp::GenerateRandomInvoke => Self::handle(store, env_mut, arg, |env, arg: GenerateRandomInvokeArg| {
                env.interface().generate_random_invoke(arg.action)
            }),
            EngineOp::EmitEvent => Self::handle(store, env_mut, arg, |env, arg: EmitEventArg| {
                env.interface().emit_event(arg.topic, arg.payload)
            }),
            EngineOp::CallInvoke => Self::handle(store, env_mut, arg, |env, arg: CallInvokeArg| {
                env.interface().call_invoke(arg.action, arg.args.into())
            }),
            EngineOp::ProofInvoke => Self::handle(store, env_mut, arg, |env, arg: ProofInvokeArg| {
                log::debug!(target: LOG_TARGET, "proof action = {:?}", arg.action);
                env.interface().proof_invoke(arg.proof_ref, arg.action, arg.args.into())
            }),
            EngineOp::BuiltinTemplateInvoke => {
                Self::handle(store, env_mut, arg, |env, arg: BuiltinTemplateInvokeArg| {
                    env.interface().builtin_template_invoke(arg.action)
                })
            },
        };

        result.unwrap_or_else(|err| {
            if let Err(err) = env
                .data()
                .state()
                .interface()
                .emit_log(LogLevel::Error, format!("Execution error: {}", err))
            {
                log::error!(target: LOG_TARGET, "Error emitting log: {}", err);
            }

            log::error!(target: LOG_TARGET, "{}", err);
            if let WasmExecutionError::RuntimeError(e) = err {
                env.data_mut().set_last_engine_error(e);
            }
            WasmPtr::null()
        })
    }

    pub fn handle<T, U, E>(
        mut store: StoreMut,
        env_mut: &mut WasmEnv<Runtime>,
        args: Vec<u8>,
        f: fn(&mut Runtime, T) -> Result<U, E>,
    ) -> Result<WasmPtr<u8>, WasmExecutionError>
    where
        T: DeserializeOwned,
        U: Serialize,
        WasmExecutionError: From<E>,
    {
        let decoded = decode_exact(&args).map_err(WasmExecutionError::EngineArgDecodeFailed)?;
        let resp = f(env_mut.state_mut(), decoded)?;
        let len = encoded_len(&resp)?;
        let ptr = env_mut.alloc(&mut store, len as u32)?;
        let mut writer = env_mut.memory_writer(&mut store, ptr)?;
        encode_with_len_to_writer(&mut writer, &resp)?;
        Ok(ptr)
    }

    fn encoded_abi_context(&self) -> Vec<u8> {
        encode(&AbiContext {}).unwrap()
    }

    /// Determine if the version of the template_lib crate in the WASM is valid.
    /// This is just a placeholder that logs the result, as we don't manage version incompatibilities yet
    fn validate_template_tari_version(module: &LoadedWasmTemplate) -> Result<(), WasmExecutionError> {
        let template_tari_version = module.template_def().tari_version();

        if are_versions_compatible(template_tari_version, ENGINE_TARI_VERSION)? {
            log::debug!(target: LOG_TARGET, "The Tari version in the template WASM (\"{}\") is compatible with the one used in the engine", template_tari_version);
        } else {
            log::error!(target: LOG_TARGET, "The Tari version in the template WASM (\"{}\") is incompatible with the one used in the engine (\"{}\")", template_tari_version, ENGINE_TARI_VERSION);
            return Err(WasmExecutionError::TemplateVersionMismatch {
                engine_version: ENGINE_TARI_VERSION.to_owned(),
                template_version: template_tari_version.to_owned(),
            });
        }

        Ok(())
    }
}

impl Invokable<Store> for WasmProcess {
    type Error = WasmExecutionError;

    fn invoke(
        &mut self,
        store: &mut Store,
        func_def: &FunctionDef,
        args: Vec<tari_bor::Value>,
    ) -> Result<InstructionResult, Self::Error> {
        let call_info = CallInfo {
            abi_context: self.encoded_abi_context(),
            func_name: func_def.name.clone(),
            args,
        };

        let main_name = format!("{}_main", self.module.template_name());
        let func: MainFunction = self.instance.exports.get_typed_function(store, &main_name)?;

        let call_info_ptr = self.alloc_and_write(store, &call_info)?;
        let res = func.call(store, call_info_ptr.as_wasm_ptr(), call_info_ptr.len());
        // No need to free since the exported function should free the memory by dropping it at the end - however, if it
        // does not the memory will be freed once the VM is destructed
        // self.env.as_ref(store).free(store, call_info_ptr)?;

        let ptr = match res {
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

        // Read response from memory
        let raw = self.env.read_memory_with_embedded_len(store, ptr.offset())?;

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

fn debug_handler<T: Send + 'static>(mut env: FunctionEnvMut<WasmEnv<T>>, arg_ptr: WasmPtr<u8>, arg_len: u32) {
    const WASM_DEBUG_LOG_TARGET: &str = "tari::dan::wasm";
    let (state, mut store) = env.data_and_store_mut();

    match state.read_from_memory(&mut store, arg_ptr, arg_len) {
        Ok(arg) => {
            eprintln!("DEBUG: {}", String::from_utf8_lossy(&arg));
        },
        Err(err) => {
            log::error!(target: WASM_DEBUG_LOG_TARGET, "Failed to read from memory: {}", err);
        },
    }
}

fn on_panic_handler<T: Send + 'static>(
    mut env: FunctionEnvMut<WasmEnv<T>>,
    msg_ptr: WasmPtr<u8>,
    msg_len: i32,
    line: i32,
    col: i32,
) {
    const WASM_DEBUG_LOG_TARGET: &str = "tari::dan::wasm";
    let (state, mut store) = env.data_and_store_mut();

    match state.read_from_memory(&mut store, msg_ptr, msg_len as u32) {
        Ok(msg) => {
            let msg = String::from_utf8_lossy(&msg);
            eprintln!("📣 PANIC: ({}:{}) {}", line, col, msg);
            log::error!(target: WASM_DEBUG_LOG_TARGET, "📣 PANIC: ({}:{}) {}", line, col, msg);
            state.set_last_panic(msg.to_string());
        },
        Err(err) => {
            log::error!(
                target: WASM_DEBUG_LOG_TARGET,
                "📣 PANIC: WASM template panicked but did not provide a valid memory pointer to on_panic \
                 callback: {}",
                err
            );
            state.set_last_panic(format!(
                "WASM panicked but did not provide a valid message pointer to on_panic callback: {}",
                err
            ));
        },
    }
}
