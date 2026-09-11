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

use log::*;
use tari_bor::{ByteCounter, decode_exact, encode_into_writer, encoded_len};
use tari_engine_types::{indexed_value::IndexedValue, instruction_result::InstructionResult, limits};
use tari_template_abi::{
    CallInfo,
    EngineOp,
    FunctionDef,
    TemplateDef,
    diagnostics::expand_panic_message,
    func_hasher::hash_function_name,
    version,
};
use tari_template_lib::{
    args::{
        AddressAllocationInvokeArg,
        BucketInvokeArg,
        BuiltinTemplateInvokeArg,
        CallInvokeArg,
        CallerContextInvokeArg,
        ComponentInvokeArg,
        ConsensusInvokeArg,
        EmitEventArg,
        EmitLogArg,
        GenerateRandomInvokeArg,
        NonFungibleInvokeArg,
        ProofInvokeArg,
        ResourceInvokeArg,
        SpendContextInvokeArg,
        VaultInvokeArg,
    },
    types::engine_args::IntrinsicInvokeArg,
};
use wasmer::{AsStoreMut, AsStoreRef, Function, FunctionEnv, FunctionEnvMut, Instance, Store, WasmPtr, imports};
use wasmer_middlewares::metering::{MeteringPoints, get_remaining_points, set_remaining_points};

use crate::{
    abi_metrics,
    runtime::{ComputeAllowance, ComputeFunding, Runtime, RuntimeError},
    traits::Invokable,
    wasm::{
        LoadedWasmTemplate,
        environment::{AllocPtr, WasmEnv},
        error::WasmExecutionError,
        mem_writer::MemWriter,
        module::MainFunction,
    },
};

const LOG_TARGET: &str = "tari::ootle::engine::wasm::process";
/// Log target for everything a template itself writes: its `tari_debug` output and its panics.
const WASM_DEBUG_LOG_TARGET: &str = "tari::ootle::wasm";

pub struct WasmProcess {
    module: LoadedWasmTemplate,
    fn_env: FunctionEnv<WasmEnv<Runtime>>,
    instance: Instance,
}

impl WasmProcess {
    pub fn init(store: &mut Store, module: LoadedWasmTemplate, state: Runtime) -> Result<Self, WasmExecutionError> {
        let fn_env = FunctionEnv::new(store, WasmEnv::new(state));
        let tari_engine = Function::new_typed_with_env(store, &fn_env, Self::tari_engine_entrypoint);

        let imports = imports! {
            "env" => {
                "tari_engine" => tari_engine,
                "tari_debug" => Function::new_typed_with_env(store, &fn_env, debug_handler),
                "on_panic" => Function::new_typed_with_env(store,&fn_env, on_panic_handler),
            }
        };
        let instance = Instance::new(store, module.wasm_module(), &imports)?;
        let memory = instance.exports.get_memory("memory")?.clone();
        let tari_alloc = instance.exports.get_typed_function(store, "tari_alloc")?;
        let tari_free = instance.exports.get_typed_function(store, "tari_free")?;
        fn_env
            .as_mut(store)
            .set_memory(memory)
            .set_alloc_funcs(tari_alloc, tari_free);

        Ok(Self {
            module,
            fn_env,
            instance,
        })
    }

    fn with_alloc_and_mem_writer<S, F, R>(
        &self,
        store: &mut S,
        alloc_size: usize,
        callback: F,
    ) -> Result<AllocPtr, WasmExecutionError>
    where
        S: AsStoreMut,
        F: for<'m> Fn(&'m mut MemWriter<'_>) -> Result<R, WasmExecutionError>,
    {
        if alloc_size > limits::ENGINE_LIMITS.max_call_size {
            return Err(WasmExecutionError::CallSizeLimitExceeded {
                limit: limits::ENGINE_LIMITS.max_call_size,
            });
        }
        let len = u32::try_from(alloc_size).map_err(|_| WasmExecutionError::MemoryAllocationTooLarge)?;

        let span = abi_metrics::Span::start();
        let ptr = self.alloc_checked(store, len)?;
        abi_metrics::record_guest_alloc(alloc_size, span.finish());
        let mut fn_env = self.env_and_store(store);
        let (env, mut store) = fn_env.data_and_store_mut();
        let mut writer = env.memory_writer(&mut store, ptr)?;
        callback(&mut writer)?;

        Ok(AllocPtr::new(ptr.offset(), len))
    }

    fn env<'a, S: AsStoreRef>(&self, store: &'a S) -> &'a WasmEnv<Runtime> {
        self.fn_env.as_ref(store)
    }

    fn env_mut<'a, S: AsStoreMut>(&self, store: &'a mut S) -> &'a mut WasmEnv<Runtime> {
        self.fn_env.as_mut(store)
    }

    /// Borrows the environment together with a store handle, as host calls receive them. Reading
    /// or writing the instance's memory needs both at once.
    fn env_and_store<'a, S: AsStoreMut>(&self, store: &'a mut S) -> FunctionEnvMut<'a, WasmEnv<Runtime>> {
        self.fn_env.clone().into_mut(store)
    }

    /// Calls the template's `tari_alloc`, failing the call if it called the engine.
    ///
    /// The environment is left unborrowed for the duration of the call. `tari_alloc` is template
    /// code, and a template that calls `tari_engine` from it has the engine take its own `&mut` to
    /// the same environment to record the refusal — so a borrow held across the call would alias.
    fn alloc_checked<S: AsStoreMut>(&self, store: &mut S, len: u32) -> Result<WasmPtr<u8>, WasmExecutionError> {
        let alloc_fn = self.env(store).mem_alloc_func()?;
        let result = alloc_fn.call(store, len);
        take_refused_engine_call(self.env_mut(store))?;
        let ptr = result?;
        if ptr.is_null() {
            return Err(WasmExecutionError::MemoryAllocationFailed);
        }
        Ok(ptr)
    }

    /// Calls the template's `tari_free`, failing the call if it called the engine. Borrows the
    /// environment under the same rule as [`Self::alloc_checked`].
    fn free_checked<S: AsStoreMut>(&self, store: &mut S, ptr: WasmPtr<u8>) -> Result<(), WasmExecutionError> {
        let free_fn = self.env(store).mem_free_func()?;
        let result = free_fn.call(store, ptr);
        take_refused_engine_call(self.env_mut(store))?;
        result?;
        Ok(())
    }

    /// Works out how much compute this invocation may run, and what bounds it.
    ///
    /// The Wasmer meter starts each store at the per-call ceiling (set when the engine compiles the
    /// module, see `wasm::module::create_engine`). Lowering it to what remains of the
    /// transaction-wide budget stops a transaction from exceeding
    /// `MAX_WASM_POINTS_PER_TRANSACTION` by spreading work across many instructions or nested
    /// cross-template calls, each of which would otherwise get a fresh per-call budget. When the
    /// budget is already spent the allowance is zero and the call traps out-of-gas on its first
    /// metered op.
    ///
    /// It is capped again by the compute the transaction is authorized to run: the fee intent's
    /// flat credit, or past the checkpoint what the fees paid can cover. That bounds the compute an
    /// under-paying transaction can extract — it traps out-of-gas once it exhausts the allowance
    /// rather than running up to the per-transaction hard cap. The allowance is shared with native
    /// verification (which pre-charges its point cost), so it is reduced by the combined
    /// consumption; the hard cap bounds WASM work only.
    fn metering_allowance(&self, store: &mut Store) -> MeteringAllowance {
        let per_call_cap = match get_remaining_points(store, &self.instance) {
            MeteringPoints::Remaining(n) => n,
            MeteringPoints::Exhausted => 0,
        };
        let interface = self.env(store).state().interface();
        let consumed = interface.wasm_points_consumed();
        let native_consumed = interface.native_points_consumed();
        let budget_remaining = limits::MAX_WASM_POINTS_PER_TRANSACTION.saturating_sub(consumed);
        let allowance_remaining = interface.compute_allowance().map(|allowance| {
            let remaining = allowance
                .points
                .saturating_sub(consumed.saturating_add(native_consumed));
            (allowance, remaining)
        });

        MeteringAllowance {
            consumed,
            points_before: match allowance_remaining {
                Some((_, remaining)) => per_call_cap.min(budget_remaining).min(remaining),
                None => per_call_cap.min(budget_remaining),
            },
            // Kept when the allowance — rather than the per-transaction hard cap — is what bounds
            // this call, so an out-of-gas trap is reported against whatever authorized it rather
            // than as a hit cap.
            binding_allowance: allowance_remaining
                .filter(|(_, remaining)| *remaining < budget_remaining && *remaining <= per_call_cap)
                .map(|(allowance, _)| allowance),
        }
    }

    /// Runs one invocation on the meter [`Invokable::invoke`] has installed, and reports how it
    /// ended without charging for it — the caller charges every outcome alike.
    ///
    /// The metered span covers all three pieces of template code the engine drives for a call: the
    /// `tari_alloc` that stages the `CallInfo`, the template function, and the `tari_free` of the
    /// pointer the function returned. All of it is guest code running on the transaction's budget,
    /// so all of it is charged to the transaction. The narrower window in which the template may
    /// call the engine stays around the function alone.
    fn run_metered(
        &self,
        store: &mut Store,
        func: &MainFunction,
        func_ident: u32,
        args: &[tari_bor::Value],
        call_info_size: usize,
    ) -> Result<InvocationOutcome, WasmExecutionError> {
        let span = abi_metrics::Span::start();
        let call_info_ptr = self.with_alloc_and_mem_writer(store, call_info_size, |mem_writer| {
            CallInfo::encode_v1_packed(mem_writer, func_ident, args)?;
            Ok(())
        })?;
        abi_metrics::record_call_info_encode(call_info_size, span.finish());

        // Call the contract entrypoint. Engine calls are admitted for exactly this window: the
        // `tari_alloc` above and the `tari_free` below run template code too, but outside any
        // invocation the engine could attribute effects to. Nothing may return early between the
        // two calls below, or the window is left open over the free.
        self.env_mut(store).enter_template_invocation();
        let res = func.call(store, call_info_ptr.as_wasm_ptr(), call_info_ptr.len());
        self.env_mut(store).exit_template_invocation();

        let return_ptr = match res {
            Ok(return_ptr) => return_ptr,
            Err(err) => return Ok(InvocationOutcome::Trapped(err)),
        };

        // Read response from memory
        // SAFETY: WasmProcess is not used concurrently
        let span = abi_metrics::Span::start();
        let mut return_bytes = 0usize;
        let value = unsafe {
            let mut fn_env = self.env_and_store(store);
            let (env, mut store) = fn_env.data_and_store_mut();
            env.with_memory_embedded_len(&mut store, return_ptr.offset(), |raw| {
                return_bytes = raw.len();
                // The returned value is bounded like the arguments passed the other way: it is
                // decoded, validated and carried into the transaction result, all of it work the
                // engine does outside the meter.
                if raw.len() > limits::ENGINE_LIMITS.max_call_size {
                    return Err(WasmExecutionError::CallSizeLimitExceeded {
                        limit: limits::ENGINE_LIMITS.max_call_size,
                    });
                }
                IndexedValue::from_raw(raw).map_err(WasmExecutionError::from)
            })??
        };
        abi_metrics::record_return_decode(return_bytes, span.finish());

        // Free allocated memory containing the result
        self.free_checked(store, return_ptr)?;

        Ok(InvocationOutcome::Returned(value))
    }

    #[allow(clippy::too_many_lines)]
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
                env.data_mut()
                    .set_last_engine_error(WasmExecutionError::InvalidEngineOp { op });
                return WasmPtr::null();
            },
        };

        // Defence in depth: `max_internal_call_size` sits above what a template can build inside
        // `WASM_LIMITS.max_memory_pages`, since the argument and its encoded copy must both fit, so
        // a template reaching this limit runs out of linear memory first.
        if arg_len as usize > limits::ENGINE_LIMITS.max_internal_call_size {
            log::error!(
                target: LOG_TARGET,
                "Engine call size limit of {} bytes exceeded: {} bytes",
                limits::ENGINE_LIMITS.max_internal_call_size,
                arg_len
            );
            env.data_mut()
                .set_last_engine_error(WasmExecutionError::EngineCallArgSizeExceeded {
                    limit: limits::ENGINE_LIMITS.max_internal_call_size,
                    size: arg_len as usize,
                });
            return WasmPtr::null();
        }

        {
            let (env_mut, mut store) = env.data_and_store_mut();

            // Only a template function invocation may call the engine. The engine also enters WASM
            // to run `tari_alloc`/`tari_free` — staging a `CallInfo`, writing an engine call's
            // response, releasing a returned value — and that template code runs outside any
            // invocation. `WasmProcess::alloc_checked`/`free_checked` and `Self::handle` turn the
            // null returned here into the recorded refusal, so a template that ignores the null
            // cannot proceed either.
            if !env_mut.is_in_template_invocation() {
                env_mut.set_refused_engine_call(op);
                return WasmPtr::null();
            }

            // Sync this invocation's in-flight meter consumption onto the transaction total before
            // dispatching, so budget and allowance checks made inside the host call (native
            // verification pre-charges, nested cross-template call budgets) see it. Without this, a
            // call could spend its whole metering allowance and still pass mid-call checks that
            // read the stale end-of-invocation total.
            if let Some(delta) = env_mut.take_unsynced_in_flight_points(&mut store) &&
                let Err(err) = env_mut.state_mut().interface_mut().record_wasm_execution(delta)
            {
                env_mut.set_last_engine_error(err);
                return WasmPtr::null();
            }
        }

        log::debug!(target: LOG_TARGET, "Engine call: {:?}", op);

        let result = match op {
            EngineOp::EmitLog => Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: EmitLogArg| {
                state.interface_mut().emit_log(arg.level, arg.message)
            }),
            EngineOp::ComponentInvoke => {
                Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: ComponentInvokeArg| {
                    state
                        .interface_mut()
                        .component_invoke(arg.component_ref, arg.action, arg.args.into())
                })
            },
            EngineOp::ResourceInvoke => {
                Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: ResourceInvokeArg| {
                    state
                        .interface_mut()
                        .resource_invoke(arg.resource_ref, arg.action, arg.args.into())
                })
            },
            EngineOp::VaultInvoke => Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: VaultInvokeArg| {
                state
                    .interface_mut()
                    .vault_invoke(arg.vault_ref, arg.action, arg.args.into())
            }),
            EngineOp::BucketInvoke => Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: BucketInvokeArg| {
                state
                    .interface_mut()
                    .bucket_invoke(arg.bucket_ref, arg.action, arg.args.into())
            }),
            EngineOp::NonFungibleInvoke => {
                Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: NonFungibleInvokeArg| {
                    state
                        .interface_mut()
                        .non_fungible_invoke(arg.address, arg.action, arg.args.into())
                })
            },
            EngineOp::GenerateUniqueId => Self::handle(&mut env, op, arg_ptr, arg_len, |state, _arg: ()| {
                state.interface_mut().generate_uuid()
            }),
            EngineOp::ConsensusInvoke => {
                Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: ConsensusInvokeArg| {
                    state.interface_mut().consensus_invoke(arg.action)
                })
            },
            EngineOp::CallerContextInvoke => {
                Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: CallerContextInvokeArg| {
                    state.interface_mut().caller_context_invoke(arg.action, arg.args.into())
                })
            },
            EngineOp::AddressAllocationInvoke => Self::handle(
                &mut env,
                op,
                arg_ptr,
                arg_len,
                |state, arg: AddressAllocationInvokeArg| state.interface_mut().allocate_address_invoke(arg),
            ),
            EngineOp::GenerateRandomInvoke => {
                Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: GenerateRandomInvokeArg| {
                    state.interface_mut().generate_random_invoke(arg.action)
                })
            },
            EngineOp::EmitEvent => Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: EmitEventArg| {
                state.interface_mut().emit_event(arg.topic, arg.payload)
            }),
            EngineOp::CallInvoke => Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: CallInvokeArg| {
                state.interface_mut().call_invoke(arg.action, arg.args.into())
            }),
            EngineOp::ProofInvoke => Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: ProofInvokeArg| {
                state
                    .interface_mut()
                    .proof_invoke(arg.proof_ref, arg.action, arg.args.into())
            }),
            EngineOp::BuiltinTemplateInvoke => Self::handle(
                &mut env,
                op,
                arg_ptr,
                arg_len,
                |state, arg: BuiltinTemplateInvokeArg| state.interface_mut().builtin_template_invoke(arg.action),
            ),
            EngineOp::IntrinsicInvoke => {
                Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: IntrinsicInvokeArg| {
                    state.interface_mut().intrinsic_invoke(arg.intrinsic, arg.args.into())
                })
            },
            EngineOp::SpendContextInvoke => {
                Self::handle(&mut env, op, arg_ptr, arg_len, |state, arg: SpendContextInvokeArg| {
                    state.interface_mut().spend_context_invoke(arg.action)
                })
            },
        };

        result.unwrap_or_else(|err| {
            // The recorded error is what reaches the transaction, as its reject reason. This line is
            // the validator's own record of it.
            log::error!(target: LOG_TARGET, "{}", err);
            env.data_mut().set_last_engine_error(err);
            WasmPtr::null()
        })
    }

    fn handle<T, U, E>(
        env: &mut FunctionEnvMut<WasmEnv<Runtime>>,
        op: EngineOp,
        arg_ptr: WasmPtr<u8>,
        arg_len: u32,
        f: fn(&mut Runtime, T) -> Result<U, E>,
    ) -> Result<WasmPtr<u8>, WasmExecutionError>
    where
        T: for<'b> tari_bor::Decode<'b, ()>,
        U: tari_bor::Encode<()> + tari_bor::CborLen<()>,
        WasmExecutionError: From<E>,
    {
        let mut sample = abi_metrics::OpSample {
            arg_bytes: arg_len as usize,
            ..Default::default()
        };

        let span = abi_metrics::Span::start();
        let decoded = {
            let (env_mut, mut store) = env.data_and_store_mut();
            // SAFETY: WasmProcess is not used concurrently and templates are not able to spawn threads
            unsafe {
                env_mut.with_memory_slice(&mut store, arg_ptr, arg_len, |arg| {
                    decode_exact(arg).map_err(|e| {
                        log::error!(target: LOG_TARGET, "Failed to decode args for engine call: {}", e);
                        WasmExecutionError::EngineArgDecodeFailed(e)
                    })
                })
            }??
        };
        sample.decode_ns = span.finish();

        let span = abi_metrics::Span::start();
        let resp = f(env.data_mut().state_mut(), decoded)?;
        sample.handler_ns = span.finish();

        let span = abi_metrics::Span::start();
        let len = encoded_len(&resp);
        sample.encode_ns = span.finish();
        sample.resp_bytes = len;

        let span = abi_metrics::Span::start();
        let ptr = Self::alloc_response(env, len)?;
        sample.alloc_ns = span.finish();

        // Encode response directly into the WASM memory. The WASM code is responsible for freeing it.
        let span = abi_metrics::Span::start();
        let (env_mut, mut store) = env.data_and_store_mut();
        let mut writer = env_mut.memory_writer(&mut store, ptr)?;
        encode_into_writer(&resp, &mut writer)?;
        sample.encode_ns += span.finish();

        abi_metrics::record_engine_op(op, sample);
        Ok(ptr)
    }

    /// Allocates room for an engine call's response through the template's own `tari_alloc`.
    ///
    /// Servicing an engine call therefore runs template code, which is closed out of the invocation
    /// window for the duration: a `tari_alloc` that calls the engine would otherwise cycle
    /// host -> WASM -> host once per response and exhaust the native stack. Nothing bounds that
    /// cycle — it is one call frame, so `max_call_depth` does not see it, and the per-call metering
    /// ceiling permits far more rounds than the stack survives.
    ///
    /// The environment is left unborrowed across the call, since a refusal is recorded through the
    /// engine's own `&mut` to it.
    fn alloc_response(
        env: &mut FunctionEnvMut<WasmEnv<Runtime>>,
        len: usize,
    ) -> Result<WasmPtr<u8>, WasmExecutionError> {
        let len = u32::try_from(len).map_err(|_| WasmExecutionError::MemoryAllocationTooLarge)?;
        let alloc_fn = env.data().mem_alloc_func()?;

        let was_open = env.data_mut().suspend_template_invocation();
        let result = alloc_fn.call(&mut *env, len);
        env.data_mut().restore_template_invocation(was_open);

        take_refused_engine_call(env.data_mut())?;
        let ptr = result?;
        if ptr.is_null() {
            return Err(WasmExecutionError::MemoryAllocationFailed);
        }
        Ok(ptr)
    }

    /// Determine if the version of the template_lib crate in the WASM is valid.
    pub fn validate_template_abi_version(template_def: &TemplateDef) -> Result<(), WasmExecutionError> {
        let template_abi_ver = template_def.abi_version();

        // Remove once minimum supported version is > 0
        #[expect(clippy::absurd_extreme_comparisons)]
        if template_abi_ver >= version::MINIMUM_SUPPORTED_WASM_ABI_VERSION {
            log::debug!(target: LOG_TARGET, "The WASM ABI version (\"{}\") is compatible with the one used in the engine", template_abi_ver);
        } else {
            log::error!(target: LOG_TARGET, "The WASM ABI version (\"{}\") is incompatible with the one used in the engine (\"{}\")", template_abi_ver, version::MINIMUM_SUPPORTED_WASM_ABI_VERSION);
            return Err(WasmExecutionError::TemplateVersionMismatch {
                engine_version: version::MINIMUM_SUPPORTED_WASM_ABI_VERSION,
                template_version: template_abi_ver,
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
        args: &[tari_bor::Value],
    ) -> Result<InstructionResult, Self::Error> {
        let main_name = format!("{}_main", self.module.template_name());
        let func: MainFunction = self.instance.exports.get_typed_function(store, &main_name)?;
        if func_def.arguments.len() != args.len() {
            return Err(WasmExecutionError::InvalidArgumentCount {
                name: func_def.name.clone(),
                expected: func_def.arguments.len(),
                actual: args.len(),
            });
        }

        let func_ident = hash_function_name(&func_def.name);
        let span = abi_metrics::Span::start();
        let mut counter = ByteCounter::new();
        CallInfo::encode_v1_packed(&mut counter, func_ident, args)?;
        let call_info_size = counter.get();
        abi_metrics::record_call_info_size_pass(call_info_size, span.finish());

        let MeteringAllowance {
            consumed,
            points_before,
            binding_allowance,
        } = self.metering_allowance(store);
        set_remaining_points(store, &self.instance, points_before);
        // Expose the in-flight meter to host calls: consumption inside this invocation must be
        // visible to budget/allowance checks made mid-call (native verification pre-charges,
        // nested cross-template call budgets), not only after the call returns.
        self.env_mut(store)
            .begin_metered_invocation(self.instance.clone(), points_before);

        let outcome = self.run_metered(store, &func, func_ident, args, call_info_size);

        let remaining_after_call = get_remaining_points(store, &self.instance);
        let exhausted = matches!(remaining_after_call, MeteringPoints::Exhausted);
        let points_consumed = match remaining_after_call {
            MeteringPoints::Remaining(n) => points_before.saturating_sub(n),
            // Out-of-gas trap: the meter says zero remaining. Charge for the entire pre-call
            // budget — the host will report a runtime error and the partial work was already done.
            MeteringPoints::Exhausted => points_before,
        };
        // Record only the tail not already synced to the transaction total by mid-call host calls.
        let already_synced = self.env_mut(store).end_metered_invocation();
        // Charging happens before we return the result so fees are recorded even on failure paths.
        self.env_mut(store)
            .state_mut()
            .interface_mut()
            .record_wasm_execution(points_consumed.saturating_sub(already_synced))?;

        // An engine error recorded during the invocation fails the call on both paths.
        // `tari_engine_entrypoint` can only answer a failed call with a null pointer, and a
        // template is free to ignore that and return normally, so the trap path alone is not enough
        // to catch it.
        if let Some(err) = self.env_mut(store).take_last_engine_error() {
            return Err(err);
        }
        // Every site that closes the window drains its own refusal before returning, so this
        // catches only a site that is later added without one.
        take_refused_engine_call(self.env_mut(store))?;

        let outcome = match outcome {
            Ok(outcome) => outcome,
            // The metered span reaches past the template function, so it can also run out of gas in
            // the `tari_alloc` that stages the `CallInfo` or the `tari_free` that releases the
            // return value. Those end the call as an engine-side error rather than a trap, and are
            // reported against whatever authorized the compute all the same.
            Err(err) => {
                return Err(exhausted
                    .then(|| compute_exceeded_error(binding_allowance, consumed, points_consumed))
                    .flatten()
                    .unwrap_or(err));
            },
        };

        match outcome {
            InvocationOutcome::Returned(value) => {
                self.env(store).state().interface().validate_return_value(&value)?;
                self.env_mut(store)
                    .state_mut()
                    .interface_mut()
                    .set_last_instruction_output(value.clone())?;

                Ok(InstructionResult {
                    indexed: value,
                    return_type: func_def.output.clone(),
                })
            },
            InvocationOutcome::Trapped(err) => {
                if let Some(message) = self.env_mut(store).take_last_panic_message() {
                    return Err(WasmExecutionError::Panic {
                        message: expand_panic_message(func_def, message),
                        runtime_error: err,
                    });
                }
                if exhausted && let Some(err) = compute_exceeded_error(binding_allowance, consumed, points_consumed) {
                    return Err(err);
                }
                error!(target: LOG_TARGET, "Error calling function: {}", err);
                Err(err.into())
            },
        }
    }
}

/// Reports an out-of-gas invocation against the compute that authorized it, when the authorized
/// compute — rather than the per-transaction hard cap — is what bound the call. `None` where the
/// hard cap bound it, which is a limit rather than an underpayment.
fn compute_exceeded_error(
    binding_allowance: Option<ComputeAllowance>,
    consumed: u64,
    points_consumed: u64,
) -> Option<WasmExecutionError> {
    let allowance = binding_allowance?;
    let consumed_points = consumed.saturating_add(points_consumed);
    match allowance.funding {
        ComputeFunding::FeeIntentCredit => Some(WasmExecutionError::FeeIntentComputeExceeded {
            consumed_points,
            credit_points: allowance.points,
        }),
        ComputeFunding::Payment => Some(WasmExecutionError::InsufficientFeesForCompute { consumed_points }),
    }
}

/// How a metered invocation ended. Both arms are charged before either is turned into a result.
///
/// One of these exists per invocation and is consumed where it is returned, so the returned value
/// travels in it directly rather than through a box.
#[allow(clippy::large_enum_variant)]
enum InvocationOutcome {
    Returned(IndexedValue),
    /// The template function trapped. The wasmer error is kept so a panic the template recorded can
    /// be reported with it.
    Trapped(wasmer::RuntimeError),
}

/// What one invocation may spend on the Wasmer meter, and what bounds it.
struct MeteringAllowance {
    /// WASM points the transaction has consumed before this invocation.
    consumed: u64,
    /// Points to set on the meter for this invocation.
    points_before: u64,
    /// Set when the authorized compute, not the per-transaction hard cap, is the binding limit.
    binding_allowance: Option<ComputeAllowance>,
}

/// Reports an engine call `tari_engine_entrypoint` refused. It can only signal a refusal by
/// returning a null pointer, which a template is free to ignore, so the recorded refusal is what
/// actually fails the call. It takes precedence over any error the alloc or free itself returned,
/// being the cause of it.
fn take_refused_engine_call<T>(env: &mut WasmEnv<T>) -> Result<(), WasmExecutionError> {
    match env.take_refused_engine_call() {
        Some(op) => Err(WasmExecutionError::RuntimeError(
            RuntimeError::EngineCallOutsideInvocation { op },
        )),
        None => Ok(()),
    }
}

/// `tari_debug` is a template's only way to write to the validator's log. What it writes goes
/// through `log` rather than straight to stderr, is bounded in size and in count, and — like an
/// engine call — is answered only while a template function invocation is in flight.
fn debug_handler<T: Send + 'static>(mut env: FunctionEnvMut<WasmEnv<T>>, arg_ptr: WasmPtr<u8>, arg_len: u32) {
    let (state, mut store) = env.data_and_store_mut();
    if !state.is_in_template_invocation() ||
        !log::log_enabled!(target: WASM_DEBUG_LOG_TARGET, log::Level::Debug) ||
        !state.allow_debug_message()
    {
        return;
    }

    let len = arg_len.min(limits::ENGINE_LIMITS.max_log_size_bytes as u32);

    // SAFETY: WasmProcess is not used concurrently
    unsafe {
        if let Err(err) = state.with_memory_slice(&mut store, arg_ptr, len, |msg| {
            log::debug!(target: WASM_DEBUG_LOG_TARGET, "{}", String::from_utf8_lossy(msg));
        }) {
            log::error!(target: WASM_DEBUG_LOG_TARGET, "Failed to read from memory: {}", err);
        }
    }
}

/// Records the panic a template reports through `on_panic`, which `WasmProcess::invoke` uses to
/// report the trap that follows it. Only a template function invocation may record one: the
/// `tari_alloc`/`tari_free` the engine drives around a call are template code too, and a panic
/// planted from there would be attributed to the next call that traps.
fn on_panic_handler<T: Send + 'static>(
    mut env: FunctionEnvMut<WasmEnv<T>>,
    msg_ptr: WasmPtr<u8>,
    msg_len: i32,
    line: i32,
    col: i32,
) {
    let (state, mut store) = env.data_and_store_mut();
    if !state.is_in_template_invocation() {
        return;
    }

    let Ok(msg_len) = u32::try_from(msg_len) else {
        log::error!(
            target: WASM_DEBUG_LOG_TARGET,
            "📣 PANIC: ({}:{}) WASM template reported a negative panic message length ({})",
            line, col, msg_len
        );
        return;
    };

    // SAFETY: There is no way to call this function concurrently
    let panic_message = unsafe {
        state.with_memory_slice(&mut store, msg_ptr, msg_len, |msg_bytes| {
            if msg_bytes.len() > limits::ENGINE_LIMITS.max_panic_message_size {
                let Ok(msg) = str::from_utf8(msg_bytes) else {
                    error!(target: WASM_DEBUG_LOG_TARGET, "📣 PANIC: ({}:{}) <invalid utf8 message>", line, col);
                    return None;
                };
                log::error!(target: WASM_DEBUG_LOG_TARGET, "📣 PANIC: ({}:{}) {}", line, col, msg);
                let limit = limits::ENGINE_LIMITS.max_panic_message_size;
                let mut end = limit;
                // Ensure we truncate at a char boundary (to avoid a panic when calling truncate)
                while end > 0 && !msg.is_char_boundary(end) {
                    end -= 1;
                }
                error!(target: LOG_TARGET, "Panic message size limit exceeded: for panic {}", msg);
                Some(msg[..end].to_string())
            } else {
                let msg = String::from_utf8_lossy(msg_bytes);
                log::error!(target: WASM_DEBUG_LOG_TARGET, "📣 PANIC: ({}:{}) {}", line, col, msg);
                Some(msg.into_owned())
            }
        })
    }
    .unwrap_or_else(|err| {
        log::error!(
            target: WASM_DEBUG_LOG_TARGET,
            "📣 PANIC: WASM template panicked but did not provide a valid memory pointer to on_panic callback: {}",
            err
        );
        Some(format!(
            "WASM panicked but did not provide a valid message pointer to on_panic callback: {}",
            err
        ))
    });

    if let Some(message) = panic_message {
        state.set_last_panic(message);
    }
}
