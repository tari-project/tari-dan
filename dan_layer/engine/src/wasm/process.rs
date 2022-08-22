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

use std::io;

use borsh::{BorshDeserialize, BorshSerialize};
use tari_template_abi::{decode, encode, encode_into, encode_with_len, CallInfo, EngineOp, Type};
use tari_template_lib::{
    abi_context::AbiContext,
    args::{
        Arg,
        BucketInvokeArg,
        CreateComponentArg,
        EmitLogArg,
        GetComponentArg,
        ResourceInvokeArg,
        SetComponentStateArg,
        VaultInvokeArg,
    },
    models::{Contract, ContractAddress, Package, PackageAddress},
};
use wasmer::{Function, Instance, Module, Store, Val, WasmerEnv};

use crate::{
    runtime::Runtime,
    traits::Invokable,
    wasm::{
        environment::{AllocPtr, WasmEnv},
        error::WasmExecutionError,
        LoadedWasmModule,
    },
};

const LOG_TARGET: &str = "tari::dan::wasm::process";

#[derive(Debug)]
pub struct Process {
    module: LoadedWasmModule,
    env: WasmEnv<Runtime>,
    instance: Instance,
    package_address: PackageAddress,
    contract_address: ContractAddress,
}

impl Process {
    pub fn start(
        module: LoadedWasmModule,
        state: Runtime,
        package_address: PackageAddress,
    ) -> Result<Self, WasmExecutionError> {
        let store = Store::default();
        let mut env = WasmEnv::new(state);
        let tari_engine = Function::new_native_with_env(&store, env.clone(), Self::tari_engine_entrypoint);
        let resolver = env.create_resolver(&store, tari_engine);
        let instance = Instance::new(module.wasm_module(), &resolver)?;
        env.init_with_instance(&instance)?;
        Ok(Self {
            module,
            env,
            instance,
            package_address,
            // TODO:
            contract_address: ContractAddress::default(),
        })
    }

    fn alloc_and_write<T: BorshSerialize>(&self, val: &T) -> Result<AllocPtr, WasmExecutionError> {
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

        let result = match op {
            EngineOp::EmitLog => Self::handle(env, arg, |env, arg: EmitLogArg| {
                env.state().interface().emit_log(arg.level, arg.message);
                Result::<_, WasmExecutionError>::Ok(())
            }),
            EngineOp::CreateComponent => Self::handle(env, arg, |env, arg: CreateComponentArg| {
                env.state().interface().create_component(arg)
            }),
            EngineOp::GetComponent => Self::handle(env, arg, |env, arg: GetComponentArg| {
                env.state().interface().get_component(&arg.component_address)
            }),
            EngineOp::SetComponentState => Self::handle(env, arg, |env, arg: SetComponentStateArg| {
                env.state()
                    .interface()
                    .set_component_state(&arg.component_address, arg.state)
            }),
            EngineOp::ResourceInvoke => Self::handle(env, arg, |env, arg: ResourceInvokeArg| {
                env.state()
                    .interface()
                    .resource_invoke(arg.resource_ref, arg.action, arg.args)
            }),
            EngineOp::VaultInvoke => Self::handle(env, arg, |env, arg: VaultInvokeArg| {
                env.state()
                    .interface()
                    .vault_invoke(arg.vault_ref, arg.action, arg.args)
            }),
            EngineOp::BucketInvoke => Self::handle(env, arg, |env, arg: BucketInvokeArg| {
                env.state()
                    .interface()
                    .bucket_invoke(arg.bucket_ref, arg.action, arg.args)
            }),
        };

        result.unwrap_or_else(|err| {
            eprintln!("{}", err);
            log::error!(target: LOG_TARGET, "{}", err);
            0
        })
    }

    pub fn handle<T, U, E>(
        env: &WasmEnv<Runtime>,
        args: Vec<u8>,
        f: fn(&WasmEnv<Runtime>, T) -> Result<U, E>,
    ) -> Result<i32, WasmExecutionError>
    where
        T: BorshDeserialize,
        U: BorshSerialize,
        WasmExecutionError: From<E>,
    {
        let decoded = decode(&args).map_err(WasmExecutionError::EngineArgDecodeFailed)?;
        let resp = f(env, decoded)?;
        let encoded = encode_with_len(&resp);
        let ptr = env.alloc(encoded.len() as u32)?;
        env.write_to_memory(&ptr, &encoded)?;
        // TODO: It's not clear how/if this memory is freed. When I drop it on the WASM side I get an
        //       out-of-bounds access error.
        Ok(ptr.as_i32())
    }

    fn encoded_abi_context(&self) -> Vec<u8> {
        encode(&AbiContext {
            package: Package {
                id: self.package_address,
            },
            contract: Contract {
                address: self.contract_address,
            },
        })
        .unwrap()
    }
}

impl Invokable for Process {
    type Error = WasmExecutionError;

    fn invoke_by_name(&self, name: &str, args: Vec<Arg>) -> Result<ExecutionResult, Self::Error> {
        let func_def = self
            .module
            .find_func_by_name(name)
            .ok_or_else(|| WasmExecutionError::FunctionNotFound { name: name.into() })?;

        let args = self.env.state().resolve_args(args)?;

        let call_info = CallInfo {
            abi_context: self.encoded_abi_context(),
            func_name: func_def.name.clone(),
            args,
        };

        let main_name = format!("{}_main", self.module.template_name());
        let func = self.instance.exports.get_function(&main_name)?;

        let call_info_ptr = self.alloc_and_write(&call_info)?;
        let res = func.call(&[call_info_ptr.as_i32().into(), Val::I32(call_info_ptr.len() as i32)])?;
        self.env.free(call_info_ptr)?;
        let ptr = res
            .get(0)
            .and_then(|v| v.i32())
            .ok_or(WasmExecutionError::ExpectedPointerReturn { function: main_name })?;

        // Read response from memory
        let raw = self.env.read_memory_with_embedded_len(ptr as u32)?;

        if raw.is_empty() {
            self.env.state().interface().set_last_instruction_output(None)?;
        } else {
            self.env
                .state()
                .interface()
                .set_last_instruction_output(Some(raw.clone()))?;
        }

        // TODO: decode raw as per function def
        Ok(ExecutionResult {
            raw,
            return_type: func_def.output.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub raw: Vec<u8>,
    pub return_type: Type,
}

impl ExecutionResult {
    pub fn decode<T: BorshDeserialize>(&self) -> io::Result<T> {
        tari_template_abi::decode(&self.raw)
    }

    pub fn empty() -> Self {
        ExecutionResult {
            raw: Vec::new(),
            return_type: Type::Unit,
        }
    }
}
