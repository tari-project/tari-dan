//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{ptr::NonNull, rc::Rc, sync::Arc};

use log::{warn, *};
use tari_bor::{MaybeTagged, decode_exact};
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_engine_types::{
    Utxo,
    UtxoOutput,
    bucket::Bucket,
    commit_result::{FinalizeResult, RejectReason},
    component::Component,
    confidential::{ClaimBurnOutputData, ClaimedOutputTombstone, MinotariBurnClaimProof},
    crypto,
    crypto::OutputBody,
    entity_id_provider::EntityIdProvider,
    events::Event,
    fees::FeeReceipt,
    hashing::hash_template_code,
    indexed_value::{IndexedValue, IndexedWellKnownTypes},
    instruction_result::InstructionResult,
    limits,
    lock::LockFlag,
    logs::LogEntry,
    proof::{ContainerRef, LockedResource},
    published_template::{PublishedTemplate, PublishedTemplateAddress, TemplateBlob},
    resource::Resource,
    resource_container::{ResourceContainer, ResourceError},
    stealth,
    substate::{SubstateId, SubstateValue},
    vault::Vault,
};
use tari_ootle_common_types::services::template_provider::TemplateProvider;
use tari_ootle_template_metadata::MetadataHash;
use tari_ootle_transaction::{
    AllocatableAddressType,
    Assertion,
    ComponentReference,
    ResourceAddressRef,
    args::{InstructionArg, WorkspaceId, WorkspaceOffsetId},
};
use tari_template_abi::{FunctionDef, TemplateDef, Type};
use tari_template_builtin::{ACCOUNT_TEMPLATE_ADDRESS, NFT_FAUCET_TEMPLATE_ADDRESS, is_builtin_template_address};
use tari_template_lib::{
    SpendContext,
    args::{
        AddressAllocationInvokeArg,
        AllocateAddressResult,
        BucketAction,
        BucketGetAmountArg,
        BucketRef,
        BuiltinTemplateAction,
        BurnBucketArg,
        BurnStealthUtxoArg,
        CallAction,
        CallFunctionArg,
        CallMethodArg,
        CallerContextAction,
        ComponentAction,
        ComponentRef,
        ConsensusAction,
        CreateComponentArg,
        CreateResourceArg,
        FreezeResourceArg,
        GenerateRandomAction,
        InvokeResult,
        MintArg,
        MintResourceArg,
        NonFungibleAction,
        PayFeeArg,
        ProofAction,
        ProofRef,
        RecallResourceArg,
        ResourceAction,
        ResourceGetNonFungibleArg,
        ResourceRef,
        ResourceUpdateNonFungibleDataArg,
        SetFreezeConfidentialOutputsArg,
        SetFreezeStealthUtxosArg,
        SpendContextAction,
        StealthTransferResourceArg,
        UpdateAccessRuleArg,
        UpdateAuthHookArg,
        VaultAction,
        VaultCreateProofByFungibleAmountArg,
        VaultCreateProofByNonFungiblesArg,
        VaultFreezeFlag,
        VaultWithdrawArg,
        WorkspaceAction,
    },
    invoke_args,
    models::{BucketId, ComponentAddressAllocation, NonFungible, NotAuthorized, ResourceAddressAllocation, VaultRef},
    template::BuiltinTemplate,
    types::{
        Amount,
        AuthHook,
        AuthHookCaller,
        ClaimedOutputTombstoneAddress,
        ComponentAddress,
        ConfidentialOutputAddress,
        EntityId,
        Hash32,
        LogLevel,
        Metadata,
        NonFungibleAddress,
        OwnerRule,
        ResourceInfo,
        ResourceType,
        SubstateOwnerRule,
        TemplateAddress,
        UtxoAddress,
        ValidatorFeePoolAddress,
        access_rules::{ComponentAccessRules, ResourceAuthAction, UpdateRule},
        bytes::Bytes,
        constants::{IMAGE_URL, TARI_TOKEN, TOKEN_SYMBOL},
        crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
        engine_args::IntrinsicId,
        metadata,
        stealth::{
            AtomicCondition,
            BuiltinPredicate,
            Covenant,
            CurrentInputView,
            MerkleProof,
            SpendAuthorization,
            SpendCondition,
            SpendWitness,
            StealthInput,
            StealthTransferStatement,
            TemplateFunction,
        },
    },
};

use super::{
    ActionIdent,
    NativeAction,
    Runtime,
    RuntimeEvent,
    spend_script_execution::SpendScriptExecution,
    working_state::WorkingState,
};
use crate::{
    intrinsics,
    runtime::{
        RuntimeError,
        RuntimeInterface,
        engine_args::EngineArgs,
        error::{ArgumentValidationError, LimitError},
        locking::{LockError, LockedSubstate},
        pay_fee::PayFee,
        scope::{FrameWriteMode, PushCallFrame},
        tracker::{ComputeAllowance, FinalizedState, StateTracker},
    },
    state_store::StateReader,
    template::LoadedTemplate,
    traits::ClaimProofVerifier,
    transaction::{ModulesCollection, TransactionProcessor},
};

const LOG_TARGET: &str = "tari::ootle::engine::runtime::impl";

pub struct RuntimeInterfaceImpl<TStore, TTemplateProvider> {
    tracker: StateTracker<TStore>,
    template_provider: Arc<TTemplateProvider>,
    entity_id_provider: EntityIdProvider,
    seal_signer_public_key: RistrettoPublicKeyBytes,
    modules: ModulesCollection<TStore>,
    claim_burn_proof_verifier: Arc<dyn ClaimProofVerifier + Send + Sync + 'static>,
    /// Transaction blob payloads, immutable for the duration of execution. Used to resolve
    /// `InstructionArg::Blob(idx)` references against the surrounding transaction's blobs.
    blobs: Rc<tari_ootle_transaction::Blobs>,
    /// A pointer to the runtime that is set after initialization to allow for cross-template calls.
    runtime_pointer: Option<NonNull<Box<dyn RuntimeInterface>>>,
    /// The introspection context made available to a spend-script predicate for the duration of its evaluation. It is
    /// set immediately before invoking the predicate and cleared immediately after, so `spend_context_invoke` (which
    /// re-enters this same interface through the runtime pointer) can serve the `SpendContext` accessors.
    spend_exec_context: Option<SpendScriptExecution>,
    /// One-shot: when set, the next pushed call frame is restricted to this write mode and denied cross-template
    /// calls. Used for the spend-script predicate frame (`ReadOnly`) and the resource auth hook frame
    /// (`OwnComponent`). Consumed by `push_call_frame`.
    restricted_frame_pending: Option<FrameWriteMode>,
}

impl<TStore: StateReader + Clone + 'static, TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>
    RuntimeInterfaceImpl<TStore, TTemplateProvider>
{
    pub fn initialize(
        tracker: StateTracker<TStore>,
        template_provider: Arc<TTemplateProvider>,
        signer_public_key: RistrettoPublicKeyBytes,
        entity_id_provider: EntityIdProvider,
        modules: ModulesCollection<TStore>,
        claim_burn_proof_verifier: Arc<dyn ClaimProofVerifier + Send + Sync + 'static>,
        blobs: Rc<tari_ootle_transaction::Blobs>,
    ) -> Result<Self, RuntimeError> {
        let mut runtime = Self {
            tracker,
            template_provider,
            entity_id_provider,
            seal_signer_public_key: signer_public_key,
            modules,
            claim_burn_proof_verifier,
            blobs,
            runtime_pointer: None,
            spend_exec_context: None,
            restricted_frame_pending: None,
        };
        runtime.invoke_modules_on_initialize()?;
        Ok(runtime)
    }

    fn invoke_modules_on_initialize(&mut self) -> Result<(), RuntimeError> {
        for module in self.modules.iter() {
            module.on_initialize(&mut self.tracker)?;
        }
        Ok(())
    }

    fn invoke_modules_on_runtime_call(&mut self, function: &'static str) -> Result<(), RuntimeError> {
        // Core sandbox enforcement runs first and unconditionally. It deliberately does NOT live in a
        // RuntimeModule: modules are optional, observer-style functionality (fees, call tracking), so making a
        // security invariant depend on one would mean dropping that module silently re-opens the sandbox. This is the
        // single per-host-op entry point, so enforcing here covers every op that routes through it regardless of
        // which modules are registered.
        self.enforce_frame_restrictions(function)?;
        for module in self.modules.iter() {
            module.on_runtime_call(&mut self.tracker, function)?;
        }
        Ok(())
    }

    /// Layer (b) of the frame sandbox: deny the effectful or non-deterministic host ops that are NOT mediated by the
    /// write-lock chokepoint (layer (a) in `WorkingState::try_lock` / `new_substate`, which neutralises every state
    /// write). Together they make a spend-script predicate provably side-effect-free and deterministic,
    /// and confine a resource auth hook to its own component state.
    ///
    /// Events are permitted in both modes: an event is an output of execution that no later code can observe, and it
    /// is discarded with the transaction if the frame fails, so it is neither a side effect on state nor a source of
    /// non-determinism.
    ///
    /// The lists contain only WASM host ops (each backed by an `EngineOp`), because a restricted frame only exists
    /// while template WASM is executing — instruction-level operations such as `pay_fee` and `publish_template` have
    /// no `EngineOp`, run only at the top level, and so can never execute in a restricted context. `call_invoke` is
    /// also blocked at the frame level (`allow_cross_template_calls == false`) and listed here for defence in depth.
    fn enforce_frame_restrictions(&self, function: &'static str) -> Result<(), RuntimeError> {
        const FORBIDDEN_IN_READ_ONLY: &[&str] = &[
            "call_invoke",
            "generate_random_invoke",
            "generate_uuid",
            "proof_invoke",
            "bucket_invoke",
        ];
        const FORBIDDEN_IN_OWN_COMPONENT: &[&str] = &["call_invoke", "proof_invoke", "bucket_invoke"];

        match self.tracker.current_frame_write_mode() {
            FrameWriteMode::Full => Ok(()),
            FrameWriteMode::OwnComponent if FORBIDDEN_IN_OWN_COMPONENT.contains(&function) => {
                Err(RuntimeError::ForbiddenInAuthHookContext { operation: function })
            },
            FrameWriteMode::ReadOnly if FORBIDDEN_IN_READ_ONLY.contains(&function) => {
                Err(RuntimeError::ForbiddenInReadOnlyContext { operation: function })
            },
            FrameWriteMode::OwnComponent | FrameWriteMode::ReadOnly => Ok(()),
        }
    }

    fn invoke_modules_on_before_finalize(&mut self) -> Result<(), RuntimeError> {
        for module in self.modules.iter() {
            module.on_before_finalize(&mut self.tracker)?;
        }
        Ok(())
    }

    /// Settles the transaction into a result.
    ///
    /// The fee module charges twice here, and the order is the point of the split. It first charges
    /// against the working state: those charges are what [`StateTracker::select_finalized_state`] tests
    /// against the payments, so they decide whether the transaction commits or falls back to a
    /// fee-intent commit. Once that is decided, it charges again against the state actually chosen,
    /// which on a fee-intent commit holds only what the fee intent touched. A transaction is
    /// therefore gated on the cost of the state it asked to commit, but pays for the state that is
    /// really persisted.
    fn finalize_with(&mut self, failure: Option<RejectReason>) -> Result<FinalizeResult, RuntimeError> {
        self.invoke_modules_on_before_finalize()?;
        let mut finalized = self.tracker.select_finalized_state(failure)?;
        // A commit persists the very state the first pass charged against, so charging it again
        // would recompute the same numbers from the same inputs. Only a fee-intent commit swaps the
        // state out from under those charges, and only it needs them redone.
        if !finalized.outcome().is_commit() {
            self.invoke_modules_on_before_persist(&mut finalized)?;
        }
        self.tracker.finalize(finalized)
    }

    fn invoke_modules_on_fee_checkpoint(&mut self) -> Result<(), RuntimeError> {
        for module in self.modules.iter() {
            module.on_fee_checkpoint(&mut self.tracker.chargeable_state())?;
        }
        Ok(())
    }

    fn invoke_modules_on_before_persist(&mut self, finalized: &mut FinalizedState<TStore>) -> Result<(), RuntimeError> {
        for module in self.modules.iter() {
            module.on_before_persist(&mut finalized.chargeable_state())?;
        }
        Ok(())
    }

    fn invoke_modules_on_runtime_event(&mut self, event: RuntimeEvent) -> Result<(), RuntimeError> {
        for module in self.modules.iter() {
            module.on_runtime_event(&mut self.tracker, &event)?;
        }
        Ok(())
    }

    pub fn get_template_def(&self, template_address: &TemplateAddress) -> Result<TemplateDef, RuntimeError> {
        let loaded = self
            .template_provider
            .get_template(template_address)
            .map_err(|e| RuntimeError::FailedToLoadTemplate {
                address: *template_address,
                details: e.to_string(),
            })?
            .ok_or(RuntimeError::TemplateNotFound {
                template_address: *template_address,
            })?;

        Ok(loaded.into_template_def())
    }

    fn validate_return_value(&self, value: &IndexedValue) -> Result<(), RuntimeError> {
        self.tracker.read_with(|state| {
            for bucket_id in value.bucket_ids() {
                let _ignore = state.get_bucket(*bucket_id)?;
            }

            // `get_bucket` scope-checks; `get_proof` does not, so the frame's own scope is checked here to keep a
            // returned proof to the same rule as a returned bucket.
            for proof_id in value.proof_ids() {
                if !state.current_call_scope()?.is_proof_in_scope(proof_id) {
                    return Err(RuntimeError::ProofNotInScope { proof_id: *proof_id });
                }
                let _ignore = state.get_proof(*proof_id)?;
            }

            for id in value.referenced_substates() {
                if !state.substate_exists(&id)? {
                    debug!(
                        target: LOG_TARGET,
                        "Returned substate {id} does not exist",
                    );

                    return Err(RuntimeError::NonExistentSubstateReturned { id });
                }
            }

            Ok(())
        })
    }

    fn check_token_symbol_length(metadata: &Metadata) -> Result<(), RuntimeError> {
        if let Some(symbol) = metadata.get(TOKEN_SYMBOL) &&
            symbol.len() > limits::MAX_TOKEN_SYMBOL_LEN
        {
            return Err(RuntimeError::InvalidArgument {
                argument: "metadata",
                reason: format!(
                    "token symbol exceeds {} bytes (got {})",
                    limits::MAX_TOKEN_SYMBOL_LEN,
                    symbol.len()
                ),
            });
        }
        Ok(())
    }

    fn emit_std_event<T: Into<SubstateId>>(
        object_name: &str,
        action: &str,
        substate_id: T,
        payload: Metadata,
        state_mut: &mut WorkingState<TStore>,
    ) -> Result<(), RuntimeError> {
        let template_address = *state_mut.current_template()?;
        let event = Event::std(Some(substate_id.into()), template_address, object_name, action, payload);
        debug!(target: LOG_TARGET, "Emitted event {}", event);
        state_mut.push_event(event)?;
        Ok(())
    }

    fn invoke_resource_access_hook(
        &mut self,
        auth_hook: AuthHook,
        mut auth_caller: AuthHookCaller,
        action: ResourceAuthAction,
    ) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("invoke_resource_access_hook")?;
        // Check if the component exist
        let skip_hook = self.tracker.read_with(|state| {
            let current_component = state.current_component()?;
            // Only execute hooks if the resource is being acted upon by an external component
            if current_component == Some(auth_hook.component_address) {
                return Ok::<_, RuntimeError>(true);
            }
            // We know that the auth hook has been validated before this is called. However, the component may not yet
            // exist if it is being created in the same call as the resource action is taking place. For
            // example, commonly a user creates a resource with initial supply and deposits it into a bucket
            // before creating the component. In this case, we "skip" the hook.
            let exists = state.store().exists(&auth_hook.component_address.into())?;
            Ok::<_, RuntimeError>(!exists)
        })?;

        if skip_hook {
            return Ok(());
        }

        let caller = auth_caller
            .component()
            .map(|component| self.load_component((*component).into()))
            .transpose()?;

        if let Some((_, caller)) = caller {
            auth_caller.with_component_state(caller.into_component().state);
        }

        // The hook frame only accepts the `AuthHookCaller` argument if the resource it names is in the acting
        // frame's scope. The reference is scoped to the hook call: the acting frame's scope must be the same after
        // the hook as before it, whether or not the resource has a hook.
        let resource_id: SubstateId = (*auth_caller.resource()).into();
        let resource_was_in_scope = self.tracker.write_with(|state_mut| {
            let scope = state_mut.current_call_scope_mut()?;
            let was_in_scope = scope.is_substate_in_scope(&resource_id);
            if !was_in_scope {
                scope.add_substate_to_referenced(resource_id.clone());
            }
            Ok::<_, RuntimeError>(was_in_scope)
        })?;

        // The signature of a call back is (action: ResourceAuthAction, auth_caller: AuthHookCaller).
        // The hook frame carries the acting component's caller badges, and the acting component never chose the hook
        // code, so the frame is confined to its own component state: it cannot use those badges to act on any vault
        // or resource, nor call out to a frame that could.
        self.restricted_frame_pending = Some(FrameWriteMode::OwnComponent);
        let ret = self.invoke_component_method(auth_hook.component_address, &auth_hook.method, invoke_args![
            action,
            auth_caller
        ]);
        self.restricted_frame_pending = None;
        let ret = ret.map_err(|e| match e {
            RuntimeError::CrossTemplateCallMethodError { details, .. } => RuntimeError::AccessDeniedAuthHook {
                action_ident: action.into(),
                details: details.to_string(),
            },
            _ => e,
        })?;
        // Enforce that the return type is actually empty. We cannot rely on InstructionResult::return_type field
        // because that comes from the template definition which is defined by the template author and may not reflect
        // actual behaviour. `is_unit` accepts either `Value::Null` (ciborium/serde encoding of `()`) or
        // `Value::Array([])` (minicbor encoding of `()`).
        if !ret.indexed.value().is_unit() {
            return Err(RuntimeError::UnexpectedNonNullInAuthHookReturn);
        }

        if !resource_was_in_scope {
            self.tracker.write_with(|state_mut| {
                state_mut
                    .current_call_scope_mut()?
                    .remove_substate_from_referenced(&resource_id);
                Ok::<_, RuntimeError>(())
            })?;
        }
        Ok(())
    }

    fn get_call_runtime(&self) -> Runtime {
        // Load the runtime pointer that must be set by whoever initialized this interface
        let ptr = self.runtime_pointer.expect("BUG: Runtime pointer not set");
        Runtime::from_pointer(ptr.as_ptr()).expect("Runtime pointer is null")
    }

    fn invoke_component_method(
        &mut self,
        component_address: ComponentAddress,
        method: &str,
        args: Vec<Bytes>,
    ) -> Result<InstructionResult, RuntimeError> {
        let mut call_runtime = self.get_call_runtime();

        TransactionProcessor::<TStore, _>::call_method(
            &*self.template_provider,
            &mut call_runtime,
            component_address.into(),
            method,
            args.into_iter().map(InstructionArg::Literal).collect(),
        )
        .map_err(|e| RuntimeError::CrossTemplateCallMethodError {
            component_address,
            method: method.to_string(),
            details: e.to_string(),
        })
    }

    fn invoke_template_function(
        &mut self,
        template_address: &TemplateAddress,
        function: &str,
        args: Vec<InstructionArg>,
    ) -> Result<InstructionResult, RuntimeError> {
        let mut call_runtime = self.get_call_runtime();

        TransactionProcessor::<TStore, _>::call_function(
            &*self.template_provider,
            &mut call_runtime,
            template_address,
            function,
            args,
        )
        .map_err(|e| RuntimeError::CrossTemplateCallFunctionError {
            template_address: *template_address,
            function: function.to_string(),
            details: e.to_string(),
        })
    }

    /// It is invalid to burn a bucket that has locked funds (e.g. by a proof). Burning downs only the unlocked
    /// commitments, so a locked one would be left live with nothing referencing it.
    fn check_bucket_is_burnable(bucket_id: BucketId, bucket: &Bucket) -> Result<(), RuntimeError> {
        if bucket.has_locked_funds() {
            return Err(RuntimeError::InvalidOpDepositLockedBucket {
                bucket_id,
                locked_amount: bucket.locked_amount(),
            });
        }
        Ok(())
    }

    /// Charges the native verification cost of a confidential mint against the payment-funded allowance before any
    /// of its proof crypto runs. The charged work must match what `WorkingState::mint_resource` goes on to verify:
    /// the value proof is only checked when the resource tracks supply and the statement mints a commitment.
    fn charge_confidential_mint(
        &mut self,
        mint_arg: &MintArg,
        has_view_key: bool,
        is_total_supply_tracking_enabled: bool,
    ) -> Result<(), RuntimeError> {
        let MintArg::Confidential {
            statement,
            value_proofs,
        } = mint_arg
        else {
            return Ok(());
        };

        self.tracker
            .charge_native_execution(tari_engine_types::confidential::statement_native_points(
                statement,
                has_view_key,
            ))?;

        if is_total_supply_tracking_enabled {
            // One proof is verified per minted commitment; the price depends on each proof's variant.
            let points = statement
                .output
                .iter()
                .filter_map(|output| value_proofs.get(&output.commitment))
                .map(crypto::value_proof_native_points)
                .fold(0u64, u64::saturating_add);
            if points > 0 {
                self.tracker.charge_native_execution(points)?;
            }
        }

        Ok(())
    }

    /// Validates that `hook` names a method with an authorization hook's signature. `argument` names the engine
    /// argument the hook arrived in, so that a rejection points at the call the caller actually made.
    fn check_resource_auth_hook(&mut self, argument: &'static str, hook: &AuthHook) -> Result<(), RuntimeError> {
        let template_address = self
            .tracker
            .write_with(|state| state.get_template_for_component(hook.component_address))?;
        let template = self.get_template_def(&template_address)?;
        let func = template
            .get_function(&hook.method)
            .ok_or(RuntimeError::InvalidArgument {
                argument,
                reason: format!("Authorize hook '{}' not found", hook),
            })?;

        if !matches!(func.output, Type::Unit) {
            return Err(RuntimeError::InvalidArgument {
                argument,
                reason: format!("Authorize hook '{}' must return unit", hook),
            });
        }

        if func.arguments.len() != 3 {
            return Err(RuntimeError::InvalidArgument {
                argument,
                reason: format!(
                    "Authorize hook '{}' must take 3 arguments (incl &self), but found {}",
                    hook,
                    func.arguments.len()
                ),
            });
        }

        if !matches!(
            func.arguments.get(1).expect("length checked").arg_type.other(),
            Some("ResourceAuthAction")
        ) {
            return Err(RuntimeError::InvalidArgument {
                argument,
                reason: format!("Authorize hook '{}' must take a ResourceAuthAction as argument 1", hook),
            });
        }

        if !matches!(
            func.arguments.get(2).expect("length checked").arg_type.other(),
            Some("AuthHookCaller")
        ) {
            return Err(RuntimeError::InvalidArgument {
                argument,
                reason: format!("Authorize hook '{}' must take an AuthHookCaller as argument 2", hook),
            });
        }

        Ok(())
    }

    /// Validates the shape of a spend-script predicate `FunctionDef`, mirroring `check_resource_auth_hook`. A spend
    /// script must be a non-mutable, unit-returning function whose last argument is a `SpendContext`. The mutability
    /// rule is load-bearing: a mutable predicate could take a write lock and cause side effects, defeating the
    /// read-only guarantee.
    fn validate_spend_script_signature(func: &FunctionDef) -> Result<(), RuntimeError> {
        if func.is_mut {
            return Err(RuntimeError::InvalidArgument {
                argument: "TemplateFunction",
                reason: format!("spend script function '{}' must not be mutable", func.name),
            });
        }
        if !matches!(func.output, Type::Unit) {
            return Err(RuntimeError::InvalidArgument {
                argument: "TemplateFunction",
                reason: format!(
                    "spend script function '{}' must return unit (it rejects by panicking, not by returning a value)",
                    func.name
                ),
            });
        }
        match func.arguments.last() {
            Some(arg) if arg.arg_type.other() == Some("SpendContext") => Ok(()),
            _ => Err(RuntimeError::InvalidArgument {
                argument: "TemplateFunction",
                reason: format!(
                    "spend script function '{}' must take a SpendContext as its last argument",
                    func.name
                ),
            }),
        }
    }

    /// Resolves a `TemplateFunction` spend-condition leaf against its template and validates it end-to-end: the
    /// referenced function exists, has the required signature, and `args` carries exactly one well-formed-CBOR element
    /// per leading (non-`SpendContext`) parameter. Because a condition tree commits only an opaque root, leaves are
    /// hidden at creation; this runs at spend time (T2) when the leaf is revealed. Templates are immutable substates,
    /// so a referenced template must already resolve — there is no "not yet published" skip case.
    fn validate_template_function(&self, tf: &TemplateFunction) -> Result<FunctionDef, RuntimeError> {
        let template_def = self.get_template_def(&tf.template)?;
        let func = template_def
            .get_function(&tf.function)
            .ok_or_else(|| RuntimeError::InvalidArgument {
                argument: "TemplateFunction",
                reason: format!(
                    "spend script function '{}' not found on template {}",
                    tf.function, tf.template
                ),
            })?;
        Self::validate_spend_script_signature(func)?;

        // `TemplateFunction.args` is positional: one CBOR value per leading parameter. Signature validation above
        // guarantees there is at least the trailing `SpendContext` argument, so this subtraction cannot underflow.
        let expected_bound_args = func.arguments.len() - 1;
        if tf.args.len() != expected_bound_args {
            return Err(RuntimeError::InvalidArgument {
                argument: "TemplateFunction",
                reason: format!(
                    "spend script '{}' expects {} bound argument(s) but {} were provided",
                    tf.function,
                    expected_bound_args,
                    tf.args.len()
                ),
            });
        }
        // The host can verify each element is well-formed CBOR; full type conformance against the declared parameter
        // type is enforced at the WASM deserialization boundary (the dispatcher's `decode_exact::<T>`).
        for (i, arg) in tf.args.iter().enumerate() {
            let _value: tari_bor::Value = decode_exact(arg).map_err(|e| RuntimeError::InvalidArgument {
                argument: "TemplateFunction",
                reason: format!(
                    "spend script '{}' bound argument {} is not well-formed CBOR: {}",
                    tf.function, i, e
                ),
            })?;
        }

        Ok(func.clone())
    }

    /// Authorises every spent input of a transfer before the spend executes, so that a rejection leaves the inputs
    /// unspent. This is the single, mandatory authorization gate for stealth spends; the execution path
    /// ([`WorkingState::validate_and_spend_stealth_utxos`]) performs no auth of its own.
    ///
    /// Each input's committed [`SpendAuthorization`] is read once, then the per-input [`SpendWitness`] selects the
    /// path:
    /// - **key path** — the output's `spend_key` must be present and its signer badge in the transaction's auth scope;
    /// - **script path** — the revealed leaf must be included under the output's committed `condition_root` (the
    ///   inclusion proof is verified exactly once, here), then the leaf is evaluated: an `AccessRule` leaf natively, a
    ///   `TemplateFunction` leaf as a read-only WASM predicate.
    fn verify_input_authorizations(
        &mut self,
        resource_address: ResourceAddressRef,
        statement: &StealthTransferStatement,
    ) -> Result<(), RuntimeError> {
        if statement.inputs_statement.inputs.is_empty() {
            return Ok(());
        }
        let resolved = self
            .tracker
            .read_with(|state| state.resolve_resource_address_ref(resource_address))?;

        // Read each input's committed authorisation once, parallel to the statement's inputs.
        let input_auths = self.tracker.write_with(|state| {
            statement
                .inputs_statement
                .inputs
                .iter()
                .map(|input| state.get_stealth_utxo_spend_auth(resolved, input))
                .collect::<Result<Vec<_>, RuntimeError>>()
        })?;

        // A covenant predicate partitions inputs by `condition_root`, so it needs the roots of all inputs, not just the
        // one it gates. Key-path inputs (which never join a covenant partition) contribute `None`.
        let input_condition_roots = statement
            .inputs_statement
            .inputs
            .iter()
            .zip(&input_auths)
            .map(|(input, auth)| {
                if input.witness.is_key_path() {
                    None
                } else {
                    auth.condition_root().copied()
                }
            })
            .collect::<Vec<_>>();

        for (index, input) in statement.inputs_statement.inputs.iter().enumerate() {
            match &input.witness {
                SpendWitness::KeyPath => {
                    let Some(pk) = input_auths[index].spend_key() else {
                        return Err(RuntimeError::ResourceError(ResourceError::InvalidSpend {
                            details: format!(
                                "Key-path witness provided for stealth UTXO {} which has no spend_key",
                                input.commitment
                            ),
                        }));
                    };
                    let badge = NonFungibleAddress::from_public_key(*pk);
                    let in_scope = self
                        .tracker
                        .read_with(|state| state.base_call_scope().auth_scope().contains_badge(&badge));
                    if !in_scope {
                        return Err(RuntimeError::ResourceError(
                            ResourceError::RequiredSignatureMissingForStealthUtxo {
                                commitment: input.commitment,
                                public_key: *pk,
                            },
                        ));
                    }
                },
                SpendWitness::ScriptPath { leaf, proof, data } => {
                    self.verify_script_path_authorization(
                        leaf,
                        proof,
                        data,
                        index,
                        input,
                        statement,
                        &input_condition_roots,
                    )?;
                },
            }
        }
        Ok(())
    }

    /// Verifies one script-path input authorisation: bounds the spender-supplied witness data and inclusion proof,
    /// validates the revealed leaf's structure, binds it to the committed `condition_root`, then evaluates it.
    #[allow(clippy::too_many_arguments)]
    fn verify_script_path_authorization(
        &mut self,
        leaf: &SpendCondition,
        proof: &MerkleProof,
        data: &Bytes,
        index: usize,
        input: &StealthInput,
        statement: &StealthTransferStatement,
        input_condition_roots: &[Option<Hash32>],
    ) -> Result<(), RuntimeError> {
        // A `condition_root` is required to spend via the script path.
        let root = input_condition_roots[index].ok_or_else(|| {
            RuntimeError::ResourceError(ResourceError::InvalidSpend {
                details: format!(
                    "Script-path witness provided for stealth UTXO {} which has no condition_root",
                    input.commitment
                ),
            })
        })?;
        // Witness data is processed natively by the leaf's predicates, so bound its size.
        let max_witness_data_len = limits::STEALTH_LIMITS.max_witness_data_len;
        if data.len() > max_witness_data_len {
            return Err(RuntimeError::ResourceError(ResourceError::InvalidSpend {
                details: format!(
                    "Spend witness data for stealth UTXO {} is {} bytes, exceeding the limit of {max_witness_data_len}",
                    input.commitment,
                    data.len()
                ),
            }));
        }
        // The inclusion proof is spender-supplied and each sibling costs a native hash, so bound its length before
        // folding it in `verify_inclusion`.
        let max_inclusion_proof_len = limits::STEALTH_LIMITS.max_inclusion_proof_len;
        if proof.siblings.len() > max_inclusion_proof_len {
            return Err(RuntimeError::ResourceError(ResourceError::InvalidSpend {
                details: format!(
                    "Inclusion proof for stealth UTXO {} has {} siblings, exceeding the limit of \
                     {max_inclusion_proof_len}",
                    input.commitment,
                    proof.siblings.len()
                ),
            }));
        }
        // The revealed leaf is untrusted spender data: validate its structure before hashing, so an adversarial
        // nesting cannot exhaust the stack in the hasher or the evaluator.
        Self::validate_condition_structure(leaf, &input.commitment)?;
        // Bind the revealed leaf to the committed root before evaluating it, so the spender cannot substitute a leaf
        // that was never committed.
        if !stealth::verify_inclusion(stealth::condition_leaf_hash(leaf), proof, root) {
            return Err(RuntimeError::ResourceError(ResourceError::InvalidSpend {
                details: format!(
                    "Revealed spend condition leaf is not committed in the condition_root of stealth UTXO {}",
                    input.commitment
                ),
            }));
        }
        self.evaluate_condition_leaf(
            leaf,
            index as u32,
            input.commitment,
            root,
            statement,
            input_condition_roots,
            data.as_slice(),
        )
    }

    /// Validates a revealed condition leaf's structure before it is hashed or evaluated. The rule itself lives in
    /// [`stealth::validate_condition_structure`] so the wallet can apply the same admissibility test before committing
    /// funds to a tree; this only attributes a failure to the UTXO being spent.
    fn validate_condition_structure(
        leaf: &SpendCondition,
        commitment: &PedersenCommitmentBytes,
    ) -> Result<(), RuntimeError> {
        stealth::validate_condition_structure(leaf).map_err(|err| {
            RuntimeError::ResourceError(ResourceError::InvalidSpend {
                details: format!("Spend condition for stealth UTXO {commitment}: {err}"),
            })
        })
    }

    /// Evaluates a revealed condition leaf that has already been proven included in the committed root. The leaf is a
    /// conjunction: every atom must hold (logical AND). The single witness `data` blob is shared by the whole leaf;
    /// each atom interprets it as it expects (a data-consuming builtin owns it entirely, which
    /// `validate_condition_structure` guarantees by rejecting any other consumer).
    #[allow(clippy::too_many_arguments)]
    fn evaluate_condition_leaf(
        &mut self,
        leaf: &SpendCondition,
        input_index: u32,
        input_commitment: PedersenCommitmentBytes,
        root: Hash32,
        statement: &StealthTransferStatement,
        input_condition_roots: &[Option<Hash32>],
        data: &[u8],
    ) -> Result<(), RuntimeError> {
        for condition in leaf.conditions() {
            self.evaluate_atomic_condition(
                condition,
                input_index,
                input_commitment,
                root,
                statement,
                input_condition_roots,
                data,
            )?;
        }
        Ok(())
    }

    /// Evaluates a single [`AtomicCondition`] of a conjunction leaf: an access rule against the auth scope, a WASM
    /// [`TemplateFunction`], a native [`BuiltinPredicate`], or a native [`Covenant`].
    #[allow(clippy::too_many_arguments)]
    fn evaluate_atomic_condition(
        &mut self,
        condition: &AtomicCondition,
        input_index: u32,
        input_commitment: PedersenCommitmentBytes,
        root: Hash32,
        statement: &StealthTransferStatement,
        input_condition_roots: &[Option<Hash32>],
        data: &[u8],
    ) -> Result<(), RuntimeError> {
        match condition {
            AtomicCondition::AccessRule(access_rule) => {
                let allowed = self
                    .tracker
                    .read_with(|state| state.authorization().check_access_rule(access_rule))?;
                if !allowed {
                    return Err(RuntimeError::AccessDenied {
                        action_ident: ActionIdent::Native(NativeAction::StealthUtxoSpend),
                    });
                }
            },
            AtomicCondition::TemplateFunction(tf) => {
                self.evaluate_spend_script(
                    tf,
                    input_index,
                    input_commitment,
                    root,
                    statement,
                    input_condition_roots,
                    data,
                )?;
            },
            AtomicCondition::Builtin(predicate) => {
                self.evaluate_builtin(predicate, data)?;
            },
            AtomicCondition::Covenant(covenant) => {
                self.evaluate_covenant(
                    covenant,
                    input_index,
                    input_commitment,
                    root,
                    statement,
                    input_condition_roots,
                )?;
            },
        }
        Ok(())
    }

    /// Evaluates a single native [`BuiltinPredicate`] — a local spend predicate (timelock or hashlock) — rejecting the
    /// spend with [`RuntimeError::SpendConditionNotMet`] if it does not hold. A data-consuming predicate (the hashlock)
    /// reads the entire witness `data` blob as raw bytes; `validate_condition_structure` guarantees it is the leaf's
    /// sole consumer, so the whole blob is unambiguously its input.
    fn evaluate_builtin(&mut self, predicate: &BuiltinPredicate, data: &[u8]) -> Result<(), RuntimeError> {
        let satisfied = match predicate {
            BuiltinPredicate::AfterEpoch(unlock_epoch) => self.tracker.get_current_epoch()?.as_u64() >= *unlock_epoch,
            BuiltinPredicate::BeforeEpoch(deadline_epoch) => {
                self.tracker.get_current_epoch()?.as_u64() < *deadline_epoch
            },
            BuiltinPredicate::HashLock { hash, alg } => stealth::hashlock_digest(*alg, data) == *hash,
        };
        if !satisfied {
            return Err(RuntimeError::SpendConditionNotMet {
                details: format!("builtin predicate not satisfied: {predicate:?}"),
            });
        }
        Ok(())
    }

    /// Evaluates a single native [`Covenant`] over the spending transfer, rejecting the spend with
    /// [`RuntimeError::SpendConditionNotMet`] if it does not hold. Every covenant introspects the transfer's outputs,
    /// so it builds a [`SpendScriptExecution`] (which clones the input/output views). `witness_data` is empty: only a
    /// WASM `TemplateFunction` reads it, via the host op.
    #[allow(clippy::too_many_arguments)]
    fn evaluate_covenant(
        &mut self,
        covenant: &Covenant,
        input_index: u32,
        input_commitment: PedersenCommitmentBytes,
        current_input_condition_root: Hash32,
        statement: &StealthTransferStatement,
        input_condition_roots: &[Option<Hash32>],
    ) -> Result<(), RuntimeError> {
        let exec = SpendScriptExecution::new(
            statement,
            input_condition_roots,
            input_index,
            input_commitment,
            current_input_condition_root,
            Vec::new(),
        );
        let satisfied = match covenant {
            Covenant::OutputPreservesCondition => exec.output_preserves_condition(),
            Covenant::OutputTo {
                condition_root,
                min_value,
            } => exec.has_output_to(condition_root, *min_value),
            Covenant::BalancePreserved(max_revealed) => exec.covenant_balanced(*max_revealed),
        };
        if !satisfied {
            return Err(RuntimeError::SpendConditionNotMet {
                details: format!("covenant not satisfied: {covenant:?}"),
            });
        }
        Ok(())
    }

    /// Invokes a single `TemplateFunction` spend-condition predicate inside a read-only restricted frame. Returning
    /// normally authorises the spend; any panic — a deliberate `assert!`, out-of-gas, or a blocked state mutation
    /// (`WriteInReadOnlyContext`) — aborts it as `SpendScriptRejected`.
    #[allow(clippy::too_many_arguments)]
    fn evaluate_spend_script(
        &mut self,
        tf: &TemplateFunction,
        input_index: u32,
        input_commitment: PedersenCommitmentBytes,
        current_input_condition_root: Hash32,
        statement: &StealthTransferStatement,
        input_condition_roots: &[Option<Hash32>],
        witness_data: &[u8],
    ) -> Result<(), RuntimeError> {
        // (T2) Authoritative spend-time validation. A revealed leaf is untrusted spender data, so the function shape
        // and bound-arg encoding are validated immediately before invoking.
        self.validate_template_function(tf)?;

        let exec = SpendScriptExecution::new(
            statement,
            input_condition_roots,
            input_index,
            input_commitment,
            current_input_condition_root,
            witness_data.to_vec(),
        );

        // Assemble the call args as [bound args..., injected SpendContext handle], exactly as the auth hook appends
        // its injected `auth_caller`. The generated dispatcher decodes each slot positionally with `decode_exact::<T>`.
        let mut args = Vec::with_capacity(tf.args.len() + 1);
        args.extend(tf.args.iter().cloned().map(InstructionArg::Literal));
        args.push(InstructionArg::Literal(
            tari_bor::encode(&SpendContext::new(input_index))?.into(),
        ));

        // Make the introspection context reachable for the duration of the call (re-entered via the runtime pointer),
        // and restrict the predicate's frame to a read-only, non-cross-template sandbox.
        self.spend_exec_context = Some(exec);
        self.restricted_frame_pending = Some(FrameWriteMode::ReadOnly);
        let result = self.invoke_template_function(&tf.template, &tf.function, args);
        self.spend_exec_context = None;
        self.restricted_frame_pending = None;

        result
            .map(|_| ())
            .map_err(|e| RuntimeError::SpendScriptRejected { details: Box::new(e) })
    }
}

impl<TStore, TTemplateProvider> RuntimeInterface for RuntimeInterfaceImpl<TStore, TTemplateProvider>
where
    TStore: StateReader + Clone + 'static,
    TTemplateProvider: TemplateProvider<Template = LoadedTemplate>,
{
    fn next_entity_id(&self) -> Result<EntityId, RuntimeError> {
        let id = self.entity_id_provider.next_entity_id()?;
        Ok(id)
    }

    fn emit_event(&mut self, topic: String, payload: Metadata) -> Result<(), RuntimeError> {
        if let Err(reason) = Event::validate_custom_topic(&topic) {
            return Err(RuntimeError::InvalidEventTopic { topic, reason });
        }

        self.invoke_modules_on_runtime_call("emit_event")?;

        let component_address_option = self.tracker.read_with(|state| {
            Ok::<_, RuntimeError>(
                state
                    .current_call_scope()?
                    .get_current_component_lock()
                    .and_then(|l| l.substate_id().as_component_address()),
            )
        })?;
        let substate_id = component_address_option.map(SubstateId::Component);
        let template_address = self.tracker.get_template_address()?;
        let module = self.tracker.get_template_module_name()?;
        let topic = format!("{module}.{topic}");

        self.tracker
            .add_event(Event::custom(substate_id, template_address, topic, payload))?;
        Ok(())
    }

    fn emit_log(&mut self, level: LogLevel, message: String) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("emit_log")?;

        let log_level = match level {
            LogLevel::Error => log::Level::Error,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Info => log::Level::Info,
            LogLevel::Debug => log::Level::Debug,
        };

        // eprintln!("{}: {}", log_level, message);
        log::log!(target: "tari::ootle::engine::runtime", log_level, "{}", message);
        let size_bytes = message.len();
        // Charged after the entry is accepted, so the bytes billed for are the bytes retained: the
        // size and count limits `add_log` enforces are what decides that.
        self.tracker.add_log(LogEntry::new(level, message))?;
        self.invoke_modules_on_runtime_event(RuntimeEvent::LogEmitted { size_bytes })?;
        Ok(())
    }

    fn load_component(&mut self, call: ComponentReference) -> Result<(ComponentAddress, Component), RuntimeError> {
        self.invoke_modules_on_runtime_call("load_component")?;
        match call {
            ComponentReference::Address(address) => self.tracker.write_with(|state_mut| {
                state_mut
                    .load_and_cache_component(address)
                    .cloned()
                    .map(|c| (address, c))
            }),
            ComponentReference::Workspace(id) => self.tracker.write_with(|state_mut| {
                let id = WorkspaceOffsetId::new(id);
                let value = state_mut
                    .workspace()
                    .get(id)?
                    .ok_or_else(|| RuntimeError::ItemNotOnWorkspace {
                        id,
                        existing_ids: state_mut.workspace().all_ids_iter().collect(),
                    })?;

                // If the value is a ComponentAddress, use it directly. If it's an integer, treat it as an allocation
                // ID.
                let address = if value.is_tag_of::<ComponentAddress>() {
                    tari_bor::from_value(value).map_err(|e| RuntimeError::InvalidArgument {
                        argument: "ComponentCall::FromWorkspace",
                        reason: format!("Item on workspace at key '{id}' is not a valid ComponentAddress: {e}",),
                    })?
                } else if value.is_tag_of::<ComponentAddressAllocation>() {
                    let allocation_id: ComponentAddressAllocation =
                        tari_bor::from_value(value).map_err(|e| RuntimeError::InvalidArgument {
                            argument: "ComponentCall::FromWorkspace",
                            reason: format!(
                                "Item on workspace at key '{id}' is not a valid ComponentAddressAllocation: {e}",
                            ),
                        })?;
                    let substate_id = state_mut.get_substate_id_from_used_address_allocation(allocation_id.id())?;
                    match substate_id {
                        SubstateId::Component(addr) => addr,
                        substate_id => {
                            let substate_type =
                                tari_ootle_common_types::substate_type::SubstateType::from(&substate_id);
                            return Err(RuntimeError::InvalidArgument {
                                argument: "ComponentCall::Allocation",
                                reason: format!(
                                    "Invalid attempt to load component with an address allocation ID ({}) with \
                                     substate type {substate_type}",
                                    allocation_id.id()
                                ),
                            });
                        },
                    }
                } else {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "ComponentCall::FromWorkspace",
                        reason: format!(
                            "Item on workspace at key '{id}' is not a valid ComponentAddress or \
                             ComponentAddressAllocation",
                        ),
                    });
                };
                state_mut
                    .load_and_cache_component(address)
                    .cloned()
                    .map(|c| (address, c))
            }),
        }
    }

    fn lock_component(
        &mut self,
        address: ComponentAddress,
        lock_flag: LockFlag,
    ) -> Result<LockedSubstate, RuntimeError> {
        self.tracker.lock_substate(SubstateId::Component(address), lock_flag)
    }

    #[allow(clippy::too_many_lines)]
    fn component_invoke(
        &mut self,
        component_ref: ComponentRef,
        action: ComponentAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("component_invoke")?;

        debug!(
            target: LOG_TARGET,
            "Component invoke: {} {:?}",
            component_ref,
            action,
        );

        match action {
            ComponentAction::Create => {
                let CreateComponentArg {
                    encoded_state,
                    owner_rule,
                    access_rules,
                    address_allocation,
                } = args.assert_one_arg()?;

                let template_addr = self.tracker.get_template_address()?;
                let template_def = self.get_template_def(&template_addr)?;
                validate_component_access_rule_methods(&access_rules, &template_def)?;

                if access_rules.contains_scoped_to_component_or_template() {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "access_rules",
                        reason: "component(..)/template(..) cannot be used on a component method access rule"
                            .to_string(),
                    });
                }
                // A component owner rule is only ever evaluated with the component's own frame on top, so
                // `component(..)`/`template(..)` would be constant (true for the component's own address).
                if let OwnerRule::ByAccessRule(rule) = &owner_rule &&
                    rule.contains_scoped_to_component_or_template()
                {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "owner_rule",
                        reason: "component(..)/template(..) cannot be used in a component owner rule".to_string(),
                    });
                }

                let owner_rule = match owner_rule {
                    OwnerRule::OwnedBySigner => SubstateOwnerRule::ByPublicKey(self.seal_signer_public_key),
                    OwnerRule::None => SubstateOwnerRule::None,
                    OwnerRule::ByAccessRule(rule) => SubstateOwnerRule::ByAccessRule(rule),
                    OwnerRule::ByPublicKey(key) => SubstateOwnerRule::ByPublicKey(key),
                };

                let component_address =
                    self.tracker
                        .new_component(encoded_state, owner_rule, access_rules, address_allocation)?;
                Ok(InvokeResult::encode(&component_address)?)
            },
            ComponentAction::GetState => {
                let component_address =
                    component_ref
                        .as_component_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "component_ref",
                            reason: "GetState component action requires a component address".to_string(),
                        })?;
                args.assert_no_args("ComponentAction::GetState")?;
                self.tracker.write_with(|state| {
                    let is_already_locked = state
                        .current_call_scope()?
                        .get_current_component_lock()
                        .map(|l| *l.substate_id() == component_address)
                        .unwrap_or(false);

                    let component_lock = if is_already_locked {
                        state
                            .current_call_scope()?
                            .get_current_component_lock()
                            .cloned()
                            .ok_or(RuntimeError::NotInComponentContext {
                                action: ComponentAction::GetState.into(),
                            })?
                    } else {
                        state.read_lock_substate(SubstateId::Component(component_address))?
                    };

                    // We only allow mutating of the current component.
                    if *component_lock.substate_id() != component_address {
                        return Err(RuntimeError::LockError(LockError::SubstateNotLocked {
                            address: SubstateId::Component(component_address),
                        }));
                    }

                    let component = state.get_component(&component_lock)?;
                    let result = InvokeResult::encode(component.state())?;
                    if !is_already_locked {
                        state.unlock_substate(component_lock)?;
                    }

                    Ok(result)
                })
            },
            ComponentAction::SetState => {
                let component_address =
                    component_ref
                        .as_component_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "component_ref",
                            reason: "SetState component action should not define a specific component address"
                                .to_string(),
                        })?;
                let component_state = args.assert_one_arg()?;
                self.tracker.write_with(|state| {
                    let component_lock = state
                        .current_call_scope()?
                        .get_current_component_lock()
                        .cloned()
                        .ok_or(RuntimeError::NotInComponentContext {
                            action: ComponentAction::SetState.into(),
                        })?;

                    // We only allow mutating of the current component. Note this check doesnt actually provide any
                    // security itself, it's just checking the engine call is made correctly. The security comes from
                    // the fact that the engine creates the lock on the currently executing component and that is the
                    // lock we use to gain access.
                    if *component_lock.substate_id() != component_address {
                        return Err(RuntimeError::AccessDeniedSetComponentState {
                            attempted_on: component_address.into(),
                            attempted_by: Box::new(component_lock.substate_id().clone()),
                        });
                    }

                    state.modify_component_with(&component_lock, |component| {
                        if component_state == *component.state() {
                            return false;
                        }
                        component.body.set(component_state);
                        true
                    })?;

                    Ok(InvokeResult::unit())
                })
            },
            ComponentAction::SetAccessRules => {
                let component_address =
                    component_ref
                        .as_component_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "component_ref",
                            reason: "SetAccessRules component action requires a component address".to_string(),
                        })?;

                let access_rules: ComponentAccessRules = args.assert_one_arg()?;

                if access_rules.contains_scoped_to_component_or_template() {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "access_rules",
                        reason: "component(..)/template(..) cannot be used on a component method access rule"
                            .to_string(),
                    });
                }

                self.tracker.write_with(|state| {
                    let component_lock = state
                        .current_call_scope()?
                        .get_current_component_lock()
                        .cloned()
                        .ok_or(RuntimeError::NotInComponentContext {
                            action: ComponentAction::SetAccessRules.into(),
                        })?;
                    // We only allow mutating of the current component. Note this check doesnt actually provide any
                    // security itself, it's just checking the engine call is made correctly. The security comes from
                    // the fact that the engine creates the lock on the currently executing component and that is the
                    // lock we use to gain access.
                    if *component_lock.substate_id() != component_address {
                        return Err(RuntimeError::LockError(LockError::SubstateNotLocked {
                            address: SubstateId::Component(component_address),
                        }));
                    }
                    let component = state.get_component(&component_lock)?;
                    state
                        .authorization()
                        .require_ownership(ComponentAction::SetAccessRules, component.as_ownership())?;

                    state.modify_component_with(&component_lock, |component| {
                        if access_rules == *component.access_rules() {
                            return false;
                        }
                        component.set_access_rules(access_rules);
                        true
                    })?;

                    Ok::<_, RuntimeError>(())
                })?;

                Ok(InvokeResult::unit())
            },
            ComponentAction::GetTemplateAddress => {
                let component_address =
                    component_ref
                        .as_component_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "component_ref",
                            reason: "GetTemplateAddress component action requires a component address".to_string(),
                        })?;

                args.assert_no_args("Component::GetTemplateAddress")?;

                self.tracker.write_with(|state| {
                    let locked = state.read_lock_substate(SubstateId::Component(component_address))?;
                    let component = state.get_component(&locked)?;
                    let template_address = *component.template_address();
                    state.unlock_substate(locked)?;
                    Ok(InvokeResult::encode(&template_address)?)
                })
            },
            ComponentAction::GetOwnerProof => {
                let component_address =
                    component_ref
                        .as_component_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "component_ref",
                            reason: "GetOwnerRule component action requires a component address".to_string(),
                        })?;

                args.assert_no_args("Component::GetOwnerRule")?;

                // The owner rule can never change so we'll just fetch the component
                self.tracker.write_with(|state_mut| {
                    let substate = state_mut.store().get_unmodified_substate(&component_address.into())?;
                    let component = substate
                        .substate_value()
                        .component()
                        .ok_or(RuntimeError::InvariantError {
                            function: "GetOwnerProof",
                            details: format!("Substate at {} is not a component", component_address),
                        })?;

                    let Some(owner) = component.owner_rule().owned_by_public_key().copied() else {
                        return Ok(InvokeResult::encode(&None::<tari_template_lib::models::Proof>)?);
                    };

                    let call_scope = state_mut.current_call_scope()?;
                    let badge = NonFungibleAddress::from_public_key(owner);
                    if !call_scope.auth_scope().contains_badge(&badge) {
                        return Err(RuntimeError::SignerBadgeNotInScope { public_key: owner });
                    }

                    let proof_id = state_mut.id_provider()?.new_proof_id();
                    let container = ResourceContainer::public_key(owner);
                    let locked = LockedResource::new(ContainerRef::Runtime, container);
                    state_mut.new_proof(proof_id, locked)?;

                    Ok(InvokeResult::encode(&Some(tari_template_lib::models::Proof::from_id(
                        proof_id,
                    )))?)
                })
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    fn resource_invoke(
        &mut self,
        resource_ref: ResourceRef,
        action: ResourceAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("resource_invoke")?;

        debug!(
            target: LOG_TARGET,
            "Resource invoke: {} {:?}",
            resource_ref,
            action,
        );

        match action {
            ResourceAction::Create => {
                let arg: CreateResourceArg = args.assert_one_arg()?;

                if arg
                    .mint_arg
                    .as_ref()
                    .map(|mint| mint.as_resource_type() != arg.resource_type)
                    .unwrap_or(false)
                {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "CreateResourceArg",
                        reason: "Mint argument type does not match resource type".to_string(),
                    });
                }

                if arg.view_key.is_some() && !arg.resource_type.is_confidential() && !arg.resource_type.is_stealth() {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "CreateResourceArg",
                        reason: "View key can only be set for stealth or confidential resources".to_string(),
                    });
                }

                if arg.divisibility > limits::MAX_DIVISIBILITY {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "CreateResourceArg",
                        reason: format!("Divisibility must be between 0 and {}", limits::MAX_DIVISIBILITY),
                    });
                }

                if arg.resource_type.is_non_fungible() && arg.divisibility > 0 {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "CreateResourceArg",
                        reason: "Non-fungible resources cannot have divisibility".to_string(),
                    });
                }

                Self::check_token_symbol_length(&arg.metadata)?;

                let owner_rule = match arg.owner_rule {
                    OwnerRule::OwnedBySigner => SubstateOwnerRule::ByPublicKey(self.seal_signer_public_key),
                    OwnerRule::ByPublicKey(key) => SubstateOwnerRule::ByPublicKey(key),
                    OwnerRule::None => SubstateOwnerRule::None,
                    OwnerRule::ByAccessRule(rule) => SubstateOwnerRule::ByAccessRule(rule),
                };

                // Check that auth hook is valid
                if let Some(hook) = arg.authorize_hook.as_ref() {
                    self.check_resource_auth_hook("CreateResourceArg", hook)?;
                }

                // Charge the initial mint's native verification cost against the payment-funded
                // allowance before its proof crypto runs.
                if let Some(mint_arg) = arg.mint_arg.as_ref() {
                    self.charge_confidential_mint(
                        mint_arg,
                        arg.view_key.is_some(),
                        arg.is_total_supply_tracking_enabled,
                    )?;
                }

                self.tracker.write_with(|state_mut| {
                    let resource = Resource::new(
                        arg.resource_type,
                        owner_rule,
                        arg.access_rules,
                        arg.metadata,
                        arg.view_key,
                        arg.authorize_hook,
                        arg.divisibility,
                        arg.is_total_supply_tracking_enabled,
                    );

                    let resource_address = match arg.address_allocation {
                        Some(allocation) => {
                            let alloc = state_mut.use_allocated_address(allocation.id())?;
                            alloc.substate_id().as_resource_address().ok_or_else(|| {
                                RuntimeError::AddressAllocationTypeMismatch {
                                    id: alloc.substate_id().clone(),
                                    expected: "ResourceAddress",
                                }
                            })?
                        },
                        None => state_mut.id_provider()?.new_resource_address()?,
                    };

                    // The system's resource addresses must stay under the system's control: the genesis resources
                    // are created once, and the two caller-badge resources must stay empty for the engine's badges
                    // to be unforgeable.
                    if resource_address.is_system_reserved() {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "resource_address",
                            reason: format!("Resource address {resource_address} is reserved by the system"),
                        });
                    }

                    let mut payload = Metadata::from_iter([("resource_type", resource.resource_type().to_string())]);
                    if let Some(symbol) = resource.metadata().get(TOKEN_SYMBOL) {
                        payload.insert(TOKEN_SYMBOL, symbol);
                    }
                    if let Some(image_url) = resource.metadata().get(IMAGE_URL) {
                        payload.insert(IMAGE_URL, image_url);
                    }

                    Self::emit_std_event("resource", "create", resource_address, payload, state_mut)?;

                    state_mut.new_substate(resource_address, resource)?;
                    let resource_lock = state_mut.write_lock_substate(SubstateId::Resource(resource_address))?;

                    let mut output_bucket = None;
                    if let Some(mint_arg) = arg.mint_arg {
                        let container = state_mut.mint_resource(&resource_lock, mint_arg)?;
                        let bucket_id = state_mut.id_provider()?.new_bucket_id();
                        state_mut.new_bucket(bucket_id, container)?;
                        output_bucket = Some(tari_template_lib::models::Bucket::from_id(bucket_id));
                    }

                    state_mut.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::encode(&(resource_address, output_bucket))?)
                })
            },

            ResourceAction::GetTotalSupply => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "GetResourceType resource action requires a resource address".to_string(),
                        })?;
                args.assert_no_args("ResourceAction::GetTotalSupply")?;
                self.tracker.write_with(|state| {
                    let locked = state.read_lock_substate(SubstateId::Resource(resource_address))?;
                    let resource = state.get_resource(&locked)?;
                    let total_supply = resource.total_supply();
                    state.unlock_substate(locked)?;
                    Ok(InvokeResult::encode(&total_supply)?)
                })
            },
            ResourceAction::GetResourceInfo => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "GetResourceType resource action requires a resource address".to_string(),
                        })?;

                args.assert_no_args("ResourceAction::GetResourceType")?;

                self.tracker.write_with(|state| {
                    let locked = state.read_lock_substate(SubstateId::Resource(resource_address))?;
                    let resource = state.get_resource(&locked)?;
                    let resource_type = resource.resource_type();
                    let divisibility = resource.divisibility();
                    state.unlock_substate(locked)?;
                    Ok(InvokeResult::encode(&ResourceInfo {
                        resource_type,
                        divisibility,
                    })?)
                })
            },
            ResourceAction::Mint => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "Mint resource action requires a resource address".to_string(),
                        })?;
                let mint_resource: MintResourceArg = args.assert_one_arg()?;

                let (resource_lock, maybe_auth_hook, auth_caller, has_view_key, tracks_supply) =
                    self.tracker.write_with(|state_mut| {
                        let resource_lock = state_mut.write_lock_substate(SubstateId::Resource(resource_address))?;

                        let resource = state_mut.get_resource(&resource_lock)?;

                        state_mut.authorization().check_resource_access_rules(
                            ResourceAuthAction::Mint,
                            resource.as_ownership(),
                            resource.access_rules(),
                        )?;

                        let auth_caller = state_mut.get_auth_caller(&resource_lock)?;
                        let has_view_key = resource.view_key().is_some();
                        let tracks_supply = resource.is_supply_tracking_enabled();
                        Ok::<_, RuntimeError>((
                            resource_lock,
                            resource.auth_hook().cloned(),
                            auth_caller,
                            has_view_key,
                            tracks_supply,
                        ))
                    })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Mint)?;
                }

                // Charge the mint's native verification cost against the payment-funded allowance
                // before its proof crypto runs.
                self.charge_confidential_mint(&mint_resource.mint_arg, has_view_key, tracks_supply)?;

                self.tracker.write_with(|state_mut| {
                    let mint_arg = mint_resource.mint_arg;

                    let resource = state_mut.mint_resource(&resource_lock, mint_arg)?;
                    let bucket_id = state_mut.id_provider()?.new_bucket_id();

                    let payload = Metadata::from_iter([
                        ("resource_type", resource.resource_type().to_string()),
                        ("amount", resource.unlocked_amount().to_string()),
                    ]);
                    Self::emit_std_event("resource", "mint", resource_address, payload, state_mut)?;

                    state_mut.new_bucket(bucket_id, resource)?;
                    let bucket = tari_template_lib::models::Bucket::from_id(bucket_id);

                    state_mut.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::encode(&bucket)?)
                })
            },
            ResourceAction::Recall => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "Recall resource action requires a resource address".to_string(),
                        })?;
                let arg: RecallResourceArg = args.assert_one_arg()?;

                let (maybe_auth_hook, auth_caller) = self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;
                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::Recall,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    let auth_hook = resource.auth_hook().cloned();
                    let auth_caller = state_mut.get_auth_caller(&resource_lock)?;

                    state_mut.unlock_substate(resource_lock)?;
                    Ok::<_, RuntimeError>((auth_hook, auth_caller))
                })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Recall)?;
                }

                self.tracker.write_with(|state_mut| {
                    let vault_lock = state_mut.write_lock_substate(arg.vault_id.into())?;

                    // The recall rule that authorized this action belongs to `resource_address`, so it may only
                    // reach vaults holding that resource.
                    let vault_resource = *state_mut.get_vault(&vault_lock)?.resource_address();
                    if vault_resource != resource_address {
                        return Err(RuntimeError::RecallResourceMismatch {
                            vault_id: arg.vault_id,
                            resource_address,
                            vault_resource,
                        });
                    }

                    let resource = state_mut.recall_resource_from_vault(&vault_lock, &arg.resource)?;

                    let payload = Metadata::from_iter([
                        ("resource_type", resource.resource_type().to_string()),
                        ("vault_id", arg.vault_id.to_string()),
                        ("recall_desc", arg.resource.to_string()),
                    ]);
                    Self::emit_std_event("resource", "recall", resource_address, payload, state_mut)?;

                    let bucket_id = state_mut.id_provider()?.new_bucket_id();
                    state_mut.new_bucket(bucket_id, resource)?;

                    state_mut.unlock_substate(vault_lock)?;

                    Ok(InvokeResult::encode(&tari_template_lib::models::Bucket::from_id(
                        bucket_id,
                    ))?)
                })
            },
            ResourceAction::GetNonFungible => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "GetNonFungible resource action requires a resource address".to_string(),
                        })?;
                let arg: ResourceGetNonFungibleArg = args.assert_one_arg()?;

                self.tracker.write_with(|state| {
                    let nft_addr = NonFungibleAddress::new(resource_address, arg.id.clone());
                    let addr = SubstateId::NonFungible(nft_addr.clone());
                    let nft_lock = state.read_lock_substate(addr)?;

                    let nf_container = state.get_non_fungible(&nft_lock)?;

                    if nf_container.is_burnt() {
                        return Err(RuntimeError::InvalidOpNonFungibleBurnt {
                            op: "GetNonFungible",
                            nf_id: arg.id,
                            resource_address,
                        });
                    }

                    state.unlock_substate(nft_lock)?;

                    Ok(InvokeResult::encode(&nft_addr)?)
                })
            },
            ResourceAction::UpdateNonFungibleData => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "UpdateNonFungibleData resource action requires a resource address".to_string(),
                        })?;
                let arg: ResourceUpdateNonFungibleDataArg = args.assert_one_arg()?;

                let (maybe_auth_hook, auth_caller) = self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;

                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::UpdateNonFungibleData,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    let auth_hook = resource.auth_hook().cloned();
                    let auth_caller = state_mut.get_auth_caller(&resource_lock)?;

                    state_mut.unlock_substate(resource_lock)?;
                    Ok::<_, RuntimeError>((auth_hook, auth_caller))
                })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(
                        auth_hook,
                        auth_caller,
                        ResourceAuthAction::UpdateNonFungibleData,
                    )?;
                }

                self.tracker.write_with(|state_mut| {
                    let addr = NonFungibleAddress::new(resource_address, arg.id);
                    let locked = state_mut.write_lock_substate(SubstateId::NonFungible(addr.clone()))?;

                    let nft = state_mut.get_non_fungible_mut(&locked)?;

                    let contents = nft
                        .contents_mut()
                        .ok_or_else(|| RuntimeError::InvalidOpNonFungibleBurnt {
                            op: "UpdateNonFungibleData",
                            resource_address,
                            nf_id: addr.id().clone(),
                        })?;
                    contents.set_mutable_data(arg.data);

                    let payload = Metadata::from_iter([("resource_type", ResourceType::NonFungible.to_string())]);
                    Self::emit_std_event(
                        "resource",
                        "update_nonfungible_data",
                        resource_address,
                        payload,
                        state_mut,
                    )?;

                    state_mut.unlock_substate(locked)?;

                    Ok(InvokeResult::unit())
                })
            },
            ResourceAction::UpdateAccessRule => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "UpdateAccessRule resource action requires a resource address".to_string(),
                        })?;
                let UpdateAccessRuleArg { action, new_rule } = args.assert_one_arg()?;

                let resource_lock = self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.write_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;
                    let updater = resource.access_rules().get_updater(&action);

                    let authorized = match updater {
                        UpdateRule::Locked => false,
                        UpdateRule::Owner => state_mut.authorization().check_ownership(resource.as_ownership())?,
                        UpdateRule::AccessRule(rule) => state_mut.authorization().check_access_rule(rule)?,
                    };

                    if !authorized {
                        return Err(RuntimeError::AccessDenied {
                            action_ident: ActionIdent::Native(NativeAction::UpdateResourceAccessRule(action)),
                        });
                    }

                    Ok::<_, RuntimeError>(resource_lock)
                })?;

                self.tracker.write_with(|state_mut| {
                    let resource_mut = state_mut.get_resource_mut(&resource_lock)?;
                    resource_mut.update_access_rule(action, new_rule);
                    let payload = Metadata::from_iter([
                        ("resource_type", resource_mut.resource_type().to_string()),
                        ("action", format!("{:?}", action)),
                    ]);
                    Self::emit_std_event("resource", "update_access_rule", resource_address, payload, state_mut)?;

                    state_mut.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            ResourceAction::UpdateAuthHook => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "UpdateAuthHook resource action requires a resource address".to_string(),
                        })?;
                let UpdateAuthHookArg { auth_hook } = args.assert_one_arg()?;

                let resource_lock = self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.write_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;
                    let updater = resource.access_rules().auth_hook_updater();

                    let authorized = match updater {
                        UpdateRule::Locked => false,
                        UpdateRule::Owner => state_mut.authorization().check_ownership(resource.as_ownership())?,
                        UpdateRule::AccessRule(rule) => state_mut.authorization().check_access_rule(rule)?,
                    };

                    if !authorized {
                        return Err(RuntimeError::AccessDenied {
                            action_ident: ActionIdent::Native(NativeAction::UpdateResourceAuthHook),
                        });
                    }

                    Ok::<_, RuntimeError>(resource_lock)
                })?;

                // The hook being replaced is not invoked: a hook that denies or panics is the failure this
                // action exists to repair, so asking it to approve its own removal would defeat the point.
                if let Some(hook) = auth_hook.as_ref() {
                    self.check_resource_auth_hook("UpdateAuthHookArg", hook)?;
                }

                self.tracker.write_with(|state_mut| {
                    let resource_mut = state_mut.get_resource_mut(&resource_lock)?;
                    let payload = Metadata::from_iter([
                        ("resource_type", resource_mut.resource_type().to_string()),
                        (
                            "auth_hook",
                            auth_hook.as_ref().map_or_else(|| "none".to_string(), |h| h.to_string()),
                        ),
                    ]);
                    resource_mut.set_auth_hook(auth_hook);
                    Self::emit_std_event("resource", "update_auth_hook", resource_address, payload, state_mut)?;

                    state_mut.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            ResourceAction::UpdateMetadata => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "UpdateMetadata resource action requires a resource address".to_string(),
                        })?;
                let new_metadata: Metadata = args.assert_one_arg()?;

                Self::check_token_symbol_length(&new_metadata)?;

                let (resource_lock, maybe_auth_hook, auth_caller) = self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.write_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;

                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::UpdateMetadata,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    // The token symbol is immutable once set.
                    if let Some(existing_symbol) = resource.token_symbol() &&
                        new_metadata.get(TOKEN_SYMBOL) != Some(existing_symbol)
                    {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "metadata",
                            reason: "cannot update an existing token symbol".to_string(),
                        });
                    }

                    let auth_caller = state_mut.get_auth_caller(&resource_lock)?;
                    Ok::<_, RuntimeError>((resource_lock, resource.auth_hook().cloned(), auth_caller))
                })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::UpdateMetadata)?;
                }

                self.tracker.write_with(|state_mut| {
                    let resource_mut = state_mut.get_resource_mut(&resource_lock)?;
                    resource_mut.set_metadata(new_metadata);
                    let mut payload =
                        Metadata::from_iter([("resource_type", resource_mut.resource_type().to_string())]);
                    if let Some(symbol) = resource_mut.token_symbol() {
                        payload.insert(TOKEN_SYMBOL, symbol);
                    }
                    Self::emit_std_event("resource", "update_metadata", resource_address, payload, state_mut)?;

                    state_mut.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            ResourceAction::SetVaultFreeze => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "Freeze resource action requires a resource address".to_string(),
                        })?;
                let arg: FreezeResourceArg = args.assert_one_arg()?;

                if !arg.flags.validate() {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "FreezeResourceArg",
                        reason: "Invalid freeze flags".to_string(),
                    });
                }

                let (maybe_auth_hook, auth_caller) = self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;

                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::Freeze,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    let auth_hook = resource.auth_hook().cloned();
                    let auth_caller = state_mut.get_auth_caller(&resource_lock)?;

                    state_mut.unlock_substate(resource_lock)?;
                    Ok::<_, RuntimeError>((auth_hook, auth_caller))
                })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Freeze)?;
                }

                self.tracker.write_with(|state_mut| {
                    let vault_lock = state_mut.write_lock_substate(arg.vault_id.into())?;

                    // The freeze rule that authorized this action belongs to `resource_address`, so it may only
                    // reach vaults holding that resource.
                    let vault_resource = *state_mut.get_vault(&vault_lock)?.resource_address();
                    if vault_resource != resource_address {
                        return Err(RuntimeError::FreezeResourceMismatch {
                            vault_id: arg.vault_id,
                            resource_address,
                            vault_resource,
                        });
                    }

                    state_mut.set_vault_freeze(&vault_lock, arg.flags)?;
                    let payload =
                        Metadata::from_iter([("vault_id", arg.vault_id.to_string()), ("flags", arg.flags.to_string())]);
                    let action = if arg.flags.is_empty() { "unfreeze" } else { "freeze" };
                    Self::emit_std_event("resource", action, resource_address, payload, state_mut)?;

                    state_mut.unlock_substate(vault_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            ResourceAction::StealthTransfer => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "StealthTransfer resource action requires a resource address".to_string(),
                        })?;
                let arg: StealthTransferResourceArg = args.assert_one_arg()?;
                let maybe_bucket = self.stealth_transfer(resource_address.into(), arg.transfer, arg.input_bucket)?;
                Ok(InvokeResult::encode(
                    &maybe_bucket.map(tari_template_lib::models::Bucket::from_id),
                )?)
            },
            ResourceAction::SetStealthUtxosFreeze => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "FreezeStealthUtxo resource action requires a resource address".to_string(),
                        })?;
                let arg: SetFreezeStealthUtxosArg = args.assert_one_arg()?;

                if arg.utxos.is_empty() {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "SetFreezeStealthUtxosArg",
                        reason: "Utxos list cannot be empty".to_string(),
                    });
                }

                self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;

                    if !resource.resource_type().is_stealth() {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "FreezeStealthUtxo can only be called on stealth resources".to_string(),
                        });
                    }

                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::Freeze,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    for utxo in arg.utxos {
                        let id = SubstateId::Utxo(UtxoAddress::new(resource_address, utxo));
                        let locked = state_mut.write_lock_substate(id.clone())?;

                        let utxo_mut = state_mut
                            .get_locked_substate_mut(&locked)?
                            .as_utxo_mut()
                            .ok_or_else(|| RuntimeError::LockSubstateMismatch {
                                lock_id: locked.lock_id(),
                                expected_type: "Utxo",
                                id,
                            })?;

                        // Freeze is idempotent.
                        if arg.freeze {
                            utxo_mut.freeze();
                        } else {
                            utxo_mut.unfreeze();
                        }
                        state_mut.unlock_substate(locked)?;
                    }

                    state_mut.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            ResourceAction::SetConfidentialOutputsFreeze => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "SetConfidentialOutputsFreeze resource action requires a resource address"
                                .to_string(),
                        })?;
                let arg: SetFreezeConfidentialOutputsArg = args.assert_one_arg()?;

                if arg.commitments.is_empty() {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "SetFreezeConfidentialOutputsArg",
                        reason: "Commitments list cannot be empty".to_string(),
                    });
                }

                self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;

                    if !resource.resource_type().is_confidential() {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "SetConfidentialOutputsFreeze can only be called on confidential resources"
                                .to_string(),
                        });
                    }

                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::Freeze,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    for commitment in arg.commitments {
                        let id = SubstateId::ConfidentialOutput(ConfidentialOutputAddress::new(
                            resource_address,
                            commitment,
                        ));
                        let locked = state_mut.write_lock_substate(id.clone())?;

                        let output_mut = state_mut
                            .get_locked_substate_mut(&locked)?
                            .as_confidential_output_mut()
                            .ok_or_else(|| RuntimeError::LockSubstateMismatch {
                                lock_id: locked.lock_id(),
                                expected_type: "ConfidentialOutput",
                                id,
                            })?;

                        // Freeze is idempotent.
                        if arg.freeze {
                            output_mut.freeze();
                        } else {
                            output_mut.unfreeze();
                        }
                        state_mut.unlock_substate(locked)?;
                    }

                    state_mut.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            ResourceAction::StealthUtxoBurn => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "BurnStealthUtxo resource action requires a resource address".to_string(),
                        })?;
                let arg: BurnStealthUtxoArg = args.assert_one_arg()?;

                // Charge the value proof's native verification (a Schnorr/ElGamal check) against
                // the payment-funded allowance before it runs.
                if arg.value_proof.is_some() {
                    self.tracker
                        .charge_native_execution(tari_engine_types::limits::NativeExecutionPoints::PER_VALUE_PROOF)?;
                }

                self.tracker.write_with(|state_mut| {
                    let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(resource_address))?;

                    let resource = state_mut.get_resource(&resource_lock)?;

                    if !resource.resource_type().is_stealth() {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "FreezeStealthUtxo can only be called on stealth resources".to_string(),
                        });
                    }

                    let is_total_supply_tracking_enabled = resource.is_supply_tracking_enabled();
                    if is_total_supply_tracking_enabled && arg.value_proof.is_none() {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "BurnStealthUtxoArg",
                            reason: "Burning from a total supply tracking resource requires a value proof".to_string(),
                        });
                    }

                    let maybe_view_key =
                        resource
                            .to_view_key_public_key()
                            .map_err(|e| RuntimeError::InvariantError {
                                function: "ResourceAction::StealthUtxoBurn",
                                details: format!(
                                    "Resource {} has a malformed view key: {}",
                                    resource_lock.substate_id(),
                                    e
                                ),
                            })?;

                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::Burn,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    state_mut.unlock_substate(resource_lock)?;

                    let id = SubstateId::Utxo(UtxoAddress::new(resource_address, arg.utxo_id));
                    let utxo_lock = state_mut.write_lock_substate(id.clone())?;

                    let utxo_mut = state_mut
                        .get_locked_substate_mut(&utxo_lock)?
                        .as_utxo_mut()
                        .ok_or_else(|| RuntimeError::LockSubstateMismatch {
                            lock_id: utxo_lock.lock_id(),
                            expected_type: "Utxo",
                            id,
                        })?;

                    if utxo_mut.is_burnt() {
                        return Err(RuntimeError::ResourceError(ResourceError::UtxoBurnFailed {
                            id: arg.utxo_id,
                            details: "already burnt".to_string(),
                        }));
                    }

                    if is_total_supply_tracking_enabled {
                        let value_proof = arg.value_proof.as_ref().expect(
                            "BUG: is_total_supply_tracking_enabled is true and value proof is some has been checked",
                        );
                        let commitment = arg.utxo_id.into_commitment_bytes();
                        // Burning discards the output, so the proof is validated against the viewable balance first
                        let elgamal_proof = utxo_mut
                            .output
                            .as_ref()
                            .and_then(|o| o.output.viewable_balance.as_ref());
                        let value = crypto::validate_value_proof(
                            &commitment,
                            maybe_view_key.as_ref(),
                            elgamal_proof,
                            value_proof,
                        )?;
                        utxo_mut.burn();
                        if value.is_positive() {
                            let resource_lock =
                                state_mut.write_lock_substate(SubstateId::Resource(resource_address))?;
                            state_mut.decrease_total_supply(&resource_lock, value)?;
                            state_mut.unlock_substate(resource_lock)?;
                        }
                    } else {
                        utxo_mut.burn();
                    }

                    state_mut.unlock_substate(utxo_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    fn vault_invoke(
        &mut self,
        vault_ref: VaultRef,
        action: VaultAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("vault_invoke")?;

        debug!(target: LOG_TARGET, "Vault invoke: {} {:?}", vault_ref, action,);

        // Check vault ownership if referencing an ID
        if action.requires_write_access() &&
            let Some(vault_id) = vault_ref.vault_id()
        {
            self.tracker
                .read_with(|state| state.check_component_scope(&vault_id.into(), action))?;
        }

        match action {
            VaultAction::Create => {
                let resource_address = vault_ref
                    .resource_address()
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "vault_ref",
                        reason: "Create vault action requires a resource address".to_string(),
                    })?;
                args.assert_no_args("CreateVault")?;

                self.tracker.write_with(|state| {
                    let resource_substate_id = SubstateId::Resource(*resource_address);
                    let resource_lock = state.read_lock_substate(resource_substate_id.clone())?;
                    let resource = state.get_resource(&resource_lock)?;

                    // Require deposit permissions on the resource to create the vault (even if empty)
                    state.authorization().check_resource_access_rules(
                        ResourceAuthAction::Deposit,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    let resource_type = state.get_resource(&resource_lock)?.resource_type();
                    let vault_id = state.id_provider()?.new_vault_id()?;
                    let resource = match resource_type {
                        ResourceType::Fungible => ResourceContainer::public_fungible(*resource_address, Amount::zero()),
                        ResourceType::NonFungible => {
                            ResourceContainer::non_fungible(*resource_address, Default::default())
                        },
                        ResourceType::Confidential => {
                            ResourceContainer::confidential(*resource_address, None, Amount::zero())
                        },
                        ResourceType::Stealth => ResourceContainer::stealth(*resource_address, Amount::zero()),
                    };

                    let vault = Vault::new(resource);

                    state.new_substate(vault_id, vault)?;
                    debug!(
                        target: LOG_TARGET,
                        "Created vault {} for resource {} ({})",
                        vault_id,
                        resource_address,
                        resource_type
                    );
                    state.unlock_substate(resource_lock)?;

                    // The resource is not orphaned because of the new vault.
                    state
                        .current_call_scope_mut()?
                        .move_node_to_owned(&resource_substate_id)?;

                    Ok(InvokeResult::encode(&vault_id)?)
                })
            },
            VaultAction::Deposit => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "Put vault action requires a vault id".to_string(),
                })?;

                let bucket_id: BucketId = args.assert_one_arg()?;

                let (vault_lock, resource_lock, maybe_auth_hook, auth_caller) =
                    self.tracker.write_with(|state_mut| {
                        let vault_lock = state_mut.write_lock_substate(SubstateId::Vault(vault_id))?;

                        let vault = state_mut.get_vault(&vault_lock)?;
                        if vault.freeze_flags().contains(VaultFreezeFlag::Deposits) {
                            return Err(RuntimeError::VaultFrozen {
                                vault_id,
                                freeze_flag: VaultFreezeFlag::Deposits,
                            });
                        }
                        let resource_address = *vault.resource_address();

                        let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(resource_address))?;

                        let resource = state_mut.get_resource(&resource_lock)?;

                        state_mut.authorization().check_resource_access_rules(
                            ResourceAuthAction::Deposit,
                            resource.as_ownership(),
                            resource.access_rules(),
                        )?;

                        let auth_caller = state_mut.get_auth_caller(&resource_lock)?;
                        Ok::<_, RuntimeError>((vault_lock, resource_lock, resource.auth_hook().cloned(), auth_caller))
                    })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Deposit)?;
                }

                self.tracker.write_with(move |state_mut| {
                    let bucket = state_mut.take_bucket(bucket_id)?;
                    // It is invalid to deposit a bucket that has locked funds
                    if bucket.has_locked_funds() {
                        return Err(RuntimeError::InvalidOpDepositLockedBucket {
                            bucket_id,
                            locked_amount: bucket.locked_amount(),
                        });
                    }

                    // Emit a builtin event for the deposit
                    let payload = Metadata::from_iter([
                        ("resource_address", bucket.resource_address().to_string()),
                        ("resource_type", bucket.resource_type().to_string()),
                        ("amount", bucket.unlocked_amount().to_string()),
                    ]);

                    Self::emit_std_event("vault", "deposit", vault_id, payload, state_mut)?;

                    let vault_mut = state_mut.get_vault_mut(&vault_lock)?;

                    vault_mut.deposit(bucket)?;

                    state_mut.unlock_substate(resource_lock)?;
                    state_mut.unlock_substate(vault_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            VaultAction::Withdraw => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "Withdraw vault action requires a vault id".to_string(),
                })?;
                let arg: VaultWithdrawArg = args.assert_one_arg()?;

                let (vault_lock, resource_lock, maybe_auth_hook, auth_caller, has_view_key) =
                    self.tracker.write_with(|state_mut| {
                        let vault_lock = state_mut.write_lock_substate(SubstateId::Vault(vault_id))?;

                        let vault = state_mut.get_vault(&vault_lock)?;
                        if vault.freeze_flags().contains(VaultFreezeFlag::Withdrawals) {
                            return Err(RuntimeError::VaultFrozen {
                                vault_id,
                                freeze_flag: VaultFreezeFlag::Withdrawals,
                            });
                        }
                        let resource_address = vault.resource_address();

                        let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(*resource_address))?;

                        let resource = state_mut.get_resource(&resource_lock)?;

                        state_mut.authorization().check_resource_access_rules(
                            ResourceAuthAction::Withdraw,
                            resource.as_ownership(),
                            resource.access_rules(),
                        )?;

                        let auth_caller = state_mut.get_auth_caller(&resource_lock)?;
                        let has_view_key = resource.view_key().is_some();
                        Ok::<_, RuntimeError>((
                            vault_lock,
                            resource_lock,
                            resource.auth_hook().cloned(),
                            auth_caller,
                            has_view_key,
                        ))
                    })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Withdraw)?;
                }

                // Charge the withdraw's native verification cost against the payment-funded
                // allowance before any of its proof crypto runs.
                if let VaultWithdrawArg::Confidential { proof } = &arg {
                    self.tracker
                        .charge_native_execution(tari_engine_types::confidential::withdraw_native_points(
                            proof,
                            has_view_key,
                        ))?;
                }

                self.tracker.write_with(|state| {
                    let resource = state.get_resource(&resource_lock)?;
                    let maybe_view_key =
                        resource
                            .to_view_key_public_key()
                            .map_err(|e| RuntimeError::InvariantError {
                                function: "VaultAction::Withdraw",
                                details: format!(
                                    "Resource {} has a malformed view key: {}",
                                    resource_lock.substate_id(),
                                    e
                                ),
                            })?;

                    // Enforce confidential limits before the withdraw's proof crypto runs.
                    if let VaultWithdrawArg::Confidential { proof } = &arg {
                        state.account_confidential_withdraw(proof)?;
                    }

                    let vault_mut = state.get_vault_mut(&vault_lock)?;
                    let (resource_container, public_amount, confidential_effects) = match arg {
                        VaultWithdrawArg::Fungible { amount } | VaultWithdrawArg::Stealth { amount } => {
                            let container = vault_mut.withdraw(amount)?;
                            (container, amount, None)
                        },
                        VaultWithdrawArg::NonFungible { ids } => {
                            let container = vault_mut.withdraw_non_fungibles(&ids)?;
                            let amount = ids.len() as u128;
                            (container, amount.into(), None)
                        },
                        VaultWithdrawArg::Confidential { proof } => {
                            let amount = proof.revealed_input_amount();
                            let (container, effects) =
                                vault_mut.withdraw_confidential(*proof, maybe_view_key.as_ref())?;
                            (container, amount, Some(effects))
                        },
                    };

                    // Down the spent input substates and materialise the new change/output commitments.
                    if let Some(effects) = confidential_effects {
                        let resource_address = *resource_container.resource_address();
                        state.materialize_confidential_outputs(resource_address, effects)?;
                    }

                    // Emit a builtin event for the withdraw
                    let payload = Metadata::from_iter([
                        ("resource_address", resource_container.resource_address().to_string()),
                        ("resource_type", resource_container.resource_type().to_string()),
                        ("amount", public_amount.to_string()),
                    ]);

                    Self::emit_std_event("vault", "withdraw", vault_id, payload, state)?;

                    let bucket_id = state.id_provider()?.new_bucket_id();
                    state.new_bucket(bucket_id, resource_container)?;

                    state.unlock_substate(vault_lock)?;
                    state.unlock_substate(resource_lock)?;

                    let bucket = tari_template_lib::models::Bucket::from_id(bucket_id);
                    Ok(InvokeResult::encode(&bucket)?)
                })
            },
            VaultAction::GetBalance => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "GetBalance vault action requires a vault id".to_string(),
                })?;
                args.assert_no_args("Vault::GetBalance")?;

                self.tracker.write_with(|state| {
                    let vault_lock = state.read_lock_substate(SubstateId::Vault(vault_id))?;
                    let balance = state.get_vault(&vault_lock)?.balance();
                    state.unlock_substate(vault_lock)?;
                    Ok(InvokeResult::encode(&balance)?)
                })
            },
            VaultAction::GetLockedBalance => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "GetBalance vault action requires a vault id".to_string(),
                })?;
                args.assert_no_args("Vault::GetBalance")?;

                self.tracker.write_with(|state| {
                    let vault_lock = state.read_lock_substate(SubstateId::Vault(vault_id))?;
                    let balance = state.get_vault(&vault_lock)?.locked_balance();
                    state.unlock_substate(vault_lock)?;
                    Ok(InvokeResult::encode(&balance)?)
                })
            },
            VaultAction::GetResourceAddress => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;
                args.assert_no_args("Vault::GetResourceAddress")?;

                self.tracker.write_with(|state| {
                    let vault_lock = state.read_lock_substate(SubstateId::Vault(vault_id))?;
                    let resource_address = *state.get_vault(&vault_lock)?.resource_address();
                    state.unlock_substate(vault_lock)?;
                    Ok(InvokeResult::encode(&resource_address)?)
                })
            },
            VaultAction::GetNonFungibleIds => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;
                args.assert_no_args("Vault::GetNonFungibleIds")?;

                self.tracker.write_with(|state| {
                    let vault_lock = state.read_lock_substate(SubstateId::Vault(vault_id))?;
                    let non_fungible_ids = state.get_vault(&vault_lock)?.get_non_fungible_ids();
                    let result = InvokeResult::encode(&non_fungible_ids)?;
                    state.unlock_substate(vault_lock)?;
                    Ok(result)
                })
            },
            VaultAction::GetCommitmentCount => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;

                args.assert_no_args("Vault::GetCommitmentCount")?;

                self.tracker.write_with(|state| {
                    let vault_lock = state.read_lock_substate(SubstateId::Vault(vault_id))?;
                    let commitment_count = state.get_vault(&vault_lock)?.get_commitment_count();
                    state.unlock_substate(vault_lock)?;
                    Ok(InvokeResult::encode(&commitment_count)?)
                })
            },
            VaultAction::PayFee => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "PayFee vault action requires a vault id".to_string(),
                })?;

                let arg: PayFeeArg = args.assert_one_arg()?;

                if self.tracker.is_fee_intent_checkpointed() {
                    return Err(RuntimeError::FeePaymentInMainIntent);
                }

                // Charge for the statement's verification cost, exactly like the
                // `stealth_transfer` method does.
                if let Some(ref statement) = arg.statement {
                    self.tracker.account_fee_intent_stealth_transfer()?;
                    let has_view_key = self.tracker.write_with(|state| {
                        let resource_lock = state.read_lock_substate(SubstateId::Resource(TARI_TOKEN))?;
                        let has_view_key = state.get_resource(&resource_lock)?.view_key().is_some();
                        state.unlock_substate(resource_lock)?;
                        Ok::<_, RuntimeError>(has_view_key)
                    })?;
                    self.tracker
                        .charge_native_execution(stealth::transfer_native_points(statement, has_view_key))?;
                }

                // Authorise the spent inputs before the fee transfer executes — the same mandatory pre-execute gate as
                // `stealth_transfer`, so a rejection leaves the inputs unspent. Fees are always paid in TARI.
                if let Some(ref statement) = arg.statement {
                    self.verify_input_authorizations(TARI_TOKEN.into(), statement)?;
                }

                self.tracker.write_with(|state_mut| {
                    let vault_lock = state_mut.write_lock_substate(SubstateId::Vault(vault_id))?;

                    let vault = state_mut.get_vault(&vault_lock)?;
                    if vault.freeze_flags().contains(VaultFreezeFlag::Withdrawals) {
                        return Err(RuntimeError::VaultFrozen {
                            vault_id,
                            freeze_flag: VaultFreezeFlag::Withdrawals,
                        });
                    }
                    let resource_address = vault.resource_address();
                    if *resource_address != TARI_TOKEN {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "vault_ref",
                            reason: format!(
                                "Fees can only be paid using XTR, however the vault contained resource {}",
                                resource_address
                            ),
                        });
                    }
                    let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(TARI_TOKEN))?;
                    let resource = state_mut.get_resource(&resource_lock)?;

                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::Withdraw,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    let vault_mut = state_mut.get_vault_mut(&vault_lock)?;

                    let mut container = ResourceContainer::stealth(TARI_TOKEN, Amount::zero());
                    if arg.amount.is_positive() {
                        let withdrawn = vault_mut.withdraw(arg.amount)?;
                        container.deposit(withdrawn)?;
                    }
                    if let Some(statement) = arg.statement &&
                        let Some(revealed) =
                            state_mut.execute_stealth_transfer(TARI_TOKEN.into(), statement, None)?
                    {
                        container.deposit(revealed)?;
                    }
                    if container.unlocked_amount().is_zero() {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "TakeFeesArg",
                            reason: "Fee payment has zero value".to_string(),
                        });
                    }

                    Self::emit_std_event(
                        "vault",
                        "pay_fee",
                        vault_id,
                        Metadata::from_iter([("amount", container.unlocked_amount().to_string())]),
                        state_mut,
                    )?;

                    state_mut.pay_fee(container, Some(vault_id))?;

                    state_mut.unlock_substate(resource_lock)?;
                    state_mut.unlock_substate(vault_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            VaultAction::CreateProofByResource => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "CreateProofByResource vault action requires a vault id".to_string(),
                })?;
                args.assert_no_args("CreateProofByResource")?;

                let (vault_lock, resource_lock, maybe_auth_hook, auth_caller) =
                    self.tracker.write_with(|state_mut| {
                        let vault_lock = state_mut.write_lock_substate(SubstateId::Vault(vault_id))?;

                        let vault = state_mut.get_vault(&vault_lock)?;
                        if vault.freeze_flags().contains(VaultFreezeFlag::Withdrawals) {
                            return Err(RuntimeError::VaultFrozen {
                                vault_id,
                                freeze_flag: VaultFreezeFlag::Withdrawals,
                            });
                        }
                        let resource_address = vault.resource_address();

                        let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(*resource_address))?;

                        let resource = state_mut.get_resource(&resource_lock)?;

                        state_mut.authorization().check_resource_access_rules(
                            ResourceAuthAction::Withdraw,
                            resource.as_ownership(),
                            resource.access_rules(),
                        )?;

                        let auth_caller = state_mut.get_auth_caller(&resource_lock)?;
                        Ok::<_, RuntimeError>((vault_lock, resource_lock, resource.auth_hook().cloned(), auth_caller))
                    })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Withdraw)?;
                }

                self.tracker.write_with(|state| {
                    let proof_id = state.id_provider()?.new_proof_id();
                    let vault_mut = state.get_vault_mut(&vault_lock)?;
                    let locked_funds = vault_mut.lock_all(vault_id)?;
                    state.new_proof(proof_id, locked_funds)?;

                    state.unlock_substate(vault_lock)?;
                    state.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::encode(&proof_id)?)
                })
            },
            VaultAction::CreateProofByFungibleAmount => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "CreateProofByFungibleAmount vault action requires a vault id".to_string(),
                })?;
                let arg: VaultCreateProofByFungibleAmountArg = args.assert_one_arg()?;

                let (vault_lock, resource_lock, maybe_auth_hook, auth_caller) =
                    self.tracker.write_with(|state_mut| {
                        let vault_lock = state_mut.write_lock_substate(SubstateId::Vault(vault_id))?;

                        let vault = state_mut.get_vault(&vault_lock)?;
                        if vault.freeze_flags().contains(VaultFreezeFlag::Withdrawals) {
                            return Err(RuntimeError::VaultFrozen {
                                vault_id,
                                freeze_flag: VaultFreezeFlag::Withdrawals,
                            });
                        }
                        let resource_address = vault.resource_address();

                        let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(*resource_address))?;

                        let resource = state_mut.get_resource(&resource_lock)?;

                        state_mut.authorization().check_resource_access_rules(
                            ResourceAuthAction::Withdraw,
                            resource.as_ownership(),
                            resource.access_rules(),
                        )?;

                        let auth_caller = state_mut.get_auth_caller(&resource_lock)?;
                        Ok::<_, RuntimeError>((vault_lock, resource_lock, resource.auth_hook().cloned(), auth_caller))
                    })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Withdraw)?;
                }

                self.tracker.write_with(|state| {
                    let proof_id = state.id_provider()?.new_proof_id();
                    let vault_mut = state.get_vault_mut(&vault_lock)?;
                    let locked_funds = vault_mut.lock_by_amount(vault_id, arg.amount)?;
                    state.new_proof(proof_id, locked_funds)?;

                    state.unlock_substate(vault_lock)?;
                    state.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::encode(&proof_id)?)
                })
            },
            VaultAction::CreateProofByNonFungibles => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "CreateProofByNonFungibles vault action requires a vault id".to_string(),
                })?;
                let arg: VaultCreateProofByNonFungiblesArg = args.assert_one_arg()?;

                let (vault_lock, resource_lock, maybe_auth_hook, auth_caller) =
                    self.tracker.write_with(|state_mut| {
                        let vault_lock = state_mut.write_lock_substate(SubstateId::Vault(vault_id))?;

                        let vault = state_mut.get_vault(&vault_lock)?;
                        if vault.freeze_flags().contains(VaultFreezeFlag::Withdrawals) {
                            return Err(RuntimeError::VaultFrozen {
                                vault_id,
                                freeze_flag: VaultFreezeFlag::Withdrawals,
                            });
                        }
                        let resource_address = vault.resource_address();

                        let resource_lock = state_mut.read_lock_substate(SubstateId::Resource(*resource_address))?;

                        let resource = state_mut.get_resource(&resource_lock)?;

                        state_mut.authorization().check_resource_access_rules(
                            ResourceAuthAction::Withdraw,
                            resource.as_ownership(),
                            resource.access_rules(),
                        )?;

                        let auth_caller = state_mut.get_auth_caller(&resource_lock)?;
                        Ok::<_, RuntimeError>((vault_lock, resource_lock, resource.auth_hook().cloned(), auth_caller))
                    })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Withdraw)?;
                }

                self.tracker.write_with(|state| {
                    let proof_id = state.id_provider()?.new_proof_id();
                    let vault_mut = state.get_vault_mut(&vault_lock)?;
                    let locked_funds = vault_mut.lock_by_non_fungible_ids(vault_id, arg.ids)?;
                    state.new_proof(proof_id, locked_funds)?;

                    state.unlock_substate(vault_lock)?;
                    state.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::encode(&proof_id)?)
                })
            },
            VaultAction::CreateProofByConfidentialResource => Err(RuntimeError::NotSupported {
                details: "CreateProofByConfidentialResource not implemented".to_string(),
            }),
            VaultAction::GetNonFungibles => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "GetNonFungibles vault action requires a vault id".to_string(),
                })?;
                args.assert_no_args("Vault::GetNonFungibles")?;

                self.tracker.write_with(|state| {
                    let vault_lock = state.read_lock_substate(SubstateId::Vault(vault_id))?;
                    let resource_address = state.get_vault(&vault_lock)?.resource_address();
                    let nft_ids = state.get_vault(&vault_lock)?.get_non_fungible_ids();
                    let nfts: Vec<NonFungible> = nft_ids
                        .iter()
                        .map(|id| NonFungibleAddress::new(*resource_address, id.clone()))
                        .map(NonFungible::new)
                        .collect();

                    let result = InvokeResult::encode(&nfts)?;
                    state.unlock_substate(vault_lock)?;
                    Ok(result)
                })
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    fn bucket_invoke(
        &mut self,
        bucket_ref: BucketRef,
        action: BucketAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("bucket_invoke")?;

        debug!(target: LOG_TARGET, "Bucket invoke: {} {:?}", bucket_ref, action,);

        match action {
            BucketAction::GetResourceAddress => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "GetResourceAddress action requires a bucket id".to_string(),
                })?;
                args.assert_no_args("Bucket::GetResourceAddress")?;

                self.tracker.read_with(|state| {
                    let bucket = state.get_bucket(bucket_id)?;
                    Ok(InvokeResult::encode(bucket.resource_address())?)
                })
            },
            BucketAction::GetResourceType => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "GetResourceType action requires a bucket id".to_string(),
                })?;
                args.assert_no_args("Bucket::GetResourceType")?;

                self.tracker.read_with(|state| {
                    let bucket = state.get_bucket(bucket_id)?;
                    Ok(InvokeResult::encode(&bucket.resource_type())?)
                })
            },
            BucketAction::GetAmount => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "GetAmount bucket action requires a bucket id".to_string(),
                })?;

                let arg: BucketGetAmountArg = args.assert_one_arg()?;
                self.tracker.read_with(|state| {
                    let bucket = state.get_bucket(bucket_id)?;
                    match arg {
                        BucketGetAmountArg::AmountOnly => Ok(InvokeResult::encode(&bucket.unlocked_amount())?),
                        BucketGetAmountArg::LockedOnly => Ok(InvokeResult::encode(&bucket.locked_amount())?),
                        BucketGetAmountArg::AmountAndLocked => {
                            let amount = bucket
                                .unlocked_amount()
                                .checked_add(bucket.locked_amount())
                                .ok_or_else(|| RuntimeError::InvariantError {
                                    function: "BucketAction::GetAmount",
                                    details: "Total amount overflowed".to_string(),
                                })?;
                            Ok(InvokeResult::encode(&amount)?)
                        },
                        BucketGetAmountArg::Everything => {
                            let amount = bucket
                                .unlocked_amount()
                                .checked_add(bucket.locked_amount())
                                .and_then(|a| {
                                    a.checked_add(Amount::from_usize(bucket.number_of_confidential_commitments()))
                                })
                                .ok_or_else(|| RuntimeError::InvariantError {
                                    function: "BucketAction::GetAmount",
                                    details: "Total amount overflowed".to_string(),
                                })?;
                            Ok(InvokeResult::encode(&amount)?)
                        },
                    }
                })
            },
            BucketAction::Take => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Take bucket action requires a bucket id".to_string(),
                })?;
                let amount = args.assert_one_arg()?;

                self.tracker.write_with(|state| {
                    let bucket = state.get_bucket_mut(bucket_id)?;
                    let resource = bucket.take(amount)?;
                    let bucket_id = state.new_bucket_id();
                    state.new_bucket(bucket_id, resource)?;
                    Ok(InvokeResult::encode(&bucket_id)?)
                })
            },
            BucketAction::TakeConfidential => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Take bucket action requires a bucket id".to_string(),
                })?;
                let proof = args.assert_one_arg()?;

                // Charge the withdraw's native verification cost against the payment-funded
                // allowance before any of its proof crypto runs. The resource peek is a cheap
                // substate read that determines whether the viewable-balance surcharge applies.
                let has_view_key = self.tracker.write_with(|state| {
                    let bucket = state.get_bucket(bucket_id)?;
                    let resource_lock = state.read_lock_substate((*bucket.resource_address()).into())?;
                    let has_view_key = state.get_resource(&resource_lock)?.view_key().is_some();
                    state.unlock_substate(resource_lock)?;
                    Ok::<_, RuntimeError>(has_view_key)
                })?;
                self.tracker
                    .charge_native_execution(tari_engine_types::confidential::withdraw_native_points(
                        &proof,
                        has_view_key,
                    ))?;

                self.tracker.write_with(|state| {
                    let bucket = state.get_bucket(bucket_id)?;
                    let resource_lock = state.read_lock_substate((*bucket.resource_address()).into())?;
                    let resource = state.get_resource(&resource_lock)?;
                    let view_key = resource
                        .to_view_key_public_key()
                        .map_err(|e| RuntimeError::InvariantError {
                            function: "BucketAction::TakeConfidential",
                            details: format!(
                                "Resource {} has a malformed view key: {}",
                                resource_lock.substate_id(),
                                e
                            ),
                        })?;
                    state.account_confidential_withdraw(&proof)?;
                    let bucket_mut = state.get_bucket_mut(bucket_id)?;
                    let (resource, effects) = bucket_mut.take_confidential(proof, view_key.as_ref())?;
                    let resource_address = *resource.resource_address();
                    state.materialize_confidential_outputs(resource_address, effects)?;
                    let bucket_id = state.id_provider()?.new_bucket_id();
                    state.new_bucket(bucket_id, resource)?;
                    state.unlock_substate(resource_lock)?;
                    Ok(InvokeResult::encode(&bucket_id)?)
                })
            },
            BucketAction::Join => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Join bucket action requires a bucket id".to_string(),
                })?;
                let other_bucket_id = args.assert_one_arg()?;

                self.tracker.write_with(|state| {
                    let other_bucket = state.take_bucket(other_bucket_id)?;
                    let bucket = state.get_bucket_mut(bucket_id)?;
                    bucket.join(other_bucket)?;
                    Ok(InvokeResult::encode(&bucket_id)?)
                })
            },
            BucketAction::Burn => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Burn bucket action requires a bucket id".to_string(),
                })?;

                let arg: BurnBucketArg = args.assert_one_arg()?;

                let (resource_lock, maybe_auth_hook, auth_caller, tracks_supply) =
                    self.tracker.write_with(|state_mut| {
                        let bucket = state_mut.get_bucket(bucket_id)?;
                        // Reject a burn that cannot succeed before the auth hook runs or anything is charged for it.
                        // This is re-checked after the hook, which may lock funds itself.
                        Self::check_bucket_is_burnable(bucket_id, bucket)?;

                        let resource_lock =
                            state_mut.write_lock_substate(SubstateId::Resource(*bucket.resource_address()))?;

                        let resource = state_mut.get_resource(&resource_lock)?;

                        state_mut.authorization().check_resource_access_rules(
                            ResourceAuthAction::Burn,
                            resource.as_ownership(),
                            resource.access_rules(),
                        )?;

                        let auth_caller = state_mut.get_auth_caller(&resource_lock)?;
                        Ok::<_, RuntimeError>((
                            resource_lock,
                            resource.auth_hook().cloned(),
                            auth_caller,
                            resource.is_supply_tracking_enabled(),
                        ))
                    })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Burn)?;
                }

                // The hook may have altered the bucket, so it is re-inspected after the hook runs and before
                // anything is charged: a hook that locked funds makes the burn fail, and the charge must cover the
                // proofs actually verified below rather than those held when the hook was scheduled.
                let value_proof_points = self.tracker.write_with(|state_mut| {
                    let bucket = state_mut.get_bucket(bucket_id)?;
                    Self::check_bucket_is_burnable(bucket_id, bucket)?;
                    if !tracks_supply {
                        return Ok::<_, RuntimeError>(0);
                    }
                    // One proof is verified per unlocked commitment; the price depends on each proof's variant.
                    let points = bucket
                        .get_confidential_commitments()
                        .into_iter()
                        .flatten()
                        .filter_map(|commitment| arg.value_proofs.get(commitment))
                        .map(crypto::value_proof_native_points)
                        .fold(0u64, u64::saturating_add);
                    Ok(points)
                })?;

                // Charge the value proofs' native verification against the payment-funded allowance before they run.
                if value_proof_points > 0 {
                    self.tracker.charge_native_execution(value_proof_points)?;
                }

                self.tracker.write_with(|state| {
                    let bucket = state.take_bucket(bucket_id)?;

                    state.burn_bucket(bucket_id, bucket, &resource_lock, &arg.value_proofs)?;

                    state.unlock_substate(resource_lock)?;

                    Ok(InvokeResult::unit())
                })
            },
            BucketAction::CreateProof => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "CreateProof bucket action requires a bucket id".to_string(),
                })?;

                args.assert_no_args("Bucket::CreateProof")?;

                let (maybe_auth_hook, auth_caller) = self.tracker.write_with(|state_mut| {
                    let bucket = state_mut.get_bucket(bucket_id)?;

                    let resource_lock =
                        state_mut.read_lock_substate(SubstateId::Resource(*bucket.resource_address()))?;

                    let resource = state_mut.get_resource(&resource_lock)?;

                    state_mut.authorization().check_resource_access_rules(
                        ResourceAuthAction::Withdraw,
                        resource.as_ownership(),
                        resource.access_rules(),
                    )?;

                    let auth_hook = resource.auth_hook().cloned();
                    let auth_caller = state_mut.get_auth_caller(&resource_lock)?;

                    state_mut.unlock_substate(resource_lock)?;
                    Ok::<_, RuntimeError>((auth_hook, auth_caller))
                })?;

                if let Some(auth_hook) = maybe_auth_hook {
                    self.invoke_resource_access_hook(auth_hook, auth_caller, ResourceAuthAction::Withdraw)?;
                }

                self.tracker.write_with(|state| {
                    let locked_funds = state.get_bucket_mut(bucket_id)?.lock_all()?;

                    let proof_id = state.id_provider()?.new_proof_id();
                    state.new_proof(proof_id, locked_funds)?;

                    Ok(InvokeResult::encode(&proof_id)?)
                })
            },
            BucketAction::GetNonFungibleIds => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "GetNonFungibleIds bucket action requires a bucket id".to_string(),
                })?;
                args.assert_no_args("Bucket::GetNonFungibleIds")?;

                self.tracker.write_with(|state| {
                    let bucket = state.get_bucket(bucket_id)?;
                    Ok(InvokeResult::encode(bucket.non_fungible_ids())?)
                })
            },
            BucketAction::GetNonFungibles => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "GetNonFungibles bucket action requires a bucket id".to_string(),
                })?;
                args.assert_no_args("Bucket::GetNonFungibles")?;

                self.tracker.write_with(|state| {
                    let bucket = state.get_bucket(bucket_id)?;
                    let resource_address = bucket.resource_address();
                    let nft_ids = bucket.non_fungible_ids();
                    let nfts: Vec<NonFungible> = nft_ids
                        .iter()
                        .map(|id| NonFungibleAddress::new(*resource_address, id.clone()))
                        .map(NonFungible::new)
                        .collect();

                    Ok(InvokeResult::encode(&nfts)?)
                })
            },
            BucketAction::CountConfidentialCommitments => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "CountConfidentialCommitments bucket action requires a bucket id".to_string(),
                })?;
                args.assert_no_args("Bucket::CountConfidentialCommitments")?;

                self.tracker.write_with(|state| {
                    let bucket = state.get_bucket(bucket_id)?;
                    Ok(InvokeResult::encode(&bucket.number_of_confidential_commitments())?)
                })
            },
            BucketAction::DropEmpty => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "DropEmpty bucket action requires a bucket id".to_string(),
                })?;
                args.assert_no_args("Bucket::DropEmpty")?;

                self.tracker.write_with(|state| {
                    let bucket = state.take_bucket(bucket_id)?;
                    if !bucket.is_empty() {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "bucket_ref",
                            reason: "Cannot drop a non-empty bucket".to_string(),
                        });
                    }
                    // Drop
                    Ok(InvokeResult::unit())
                })
            },
        }
    }

    fn proof_invoke(
        &mut self,
        proof_ref: ProofRef,
        action: ProofAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("proof_invoke")?;

        debug!(
            target: LOG_TARGET,
            "Proof invoke: {} {:?}",
            proof_ref,
            action,
        );

        match action {
            ProofAction::GetAmount => {
                let proof_id = proof_ref.proof_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "proof_ref",
                    reason: "GetAmount proof action requires a proof id".to_string(),
                })?;
                args.assert_no_args("Proof.GetAmount")?;
                self.tracker.write_with(|state| {
                    let proof = state.get_proof_in_scope(proof_id)?;
                    Ok(InvokeResult::encode(&proof.amount())?)
                })
            },
            ProofAction::GetResourceAddress => {
                let proof_id = proof_ref.proof_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "proof_ref",
                    reason: "GetResourceAddress proof action requires a proof id".to_string(),
                })?;
                args.assert_no_args("Proof.GetResourceAddress")?;
                self.tracker.write_with(|state| {
                    let proof = state.get_proof_in_scope(proof_id)?;
                    Ok(InvokeResult::encode(proof.resource_address())?)
                })
            },
            ProofAction::GetResourceType => {
                let proof_id = proof_ref.proof_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "proof_ref",
                    reason: "GetResourceType proof action requires a proof id".to_string(),
                })?;

                args.assert_no_args("Proof.GetResourceType")?;

                self.tracker.write_with(|state| {
                    let proof = state.get_proof_in_scope(proof_id)?;
                    Ok(InvokeResult::encode(&proof.resource_type())?)
                })
            },
            ProofAction::GetNonFungibles => {
                let proof_id = proof_ref.proof_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "proof_ref",
                    reason: "GetNonFungibles proof action requires a proof id".to_string(),
                })?;

                args.assert_no_args("Proof.GetNonFungibles")?;

                self.tracker.write_with(|state| {
                    let proof = state.get_proof_in_scope(proof_id)?;
                    let nfts = proof.non_fungible_token_ids();
                    Ok(InvokeResult::encode(&nfts)?)
                })
            },
            ProofAction::Authorize => {
                let proof_id = proof_ref.proof_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "proof_ref",
                    reason: "Authorize proof action requires a proof id".to_string(),
                })?;
                args.assert_no_args("Proof.CreateAccess")?;

                self.tracker.write_with(|state| {
                    // A proof id is a sequential counter shared by the whole transaction, so authority has to come
                    // from the frame's own scope: a proof it created, was passed as an argument, or a callee handed
                    // back. Every other live proof in the transaction belongs to someone else.
                    if !state.proof_exists(proof_id) || !state.current_call_scope()?.is_proof_in_scope(&proof_id) {
                        return Ok(InvokeResult::encode(&Err::<(), _>(NotAuthorized))?);
                    }
                    state.current_call_scope_mut()?.auth_scope_mut().add_proof(proof_id);
                    Ok(InvokeResult::encode(&Ok::<_, NotAuthorized>(()))?)
                })
            },
            ProofAction::DropAuthorize => {
                let proof_id = proof_ref.proof_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "proof_ref",
                    reason: "DropAuthorize proof action requires a proof id".to_string(),
                })?;
                args.assert_no_args("Proof.DropAuthorize")?;

                self.tracker.write_with(|state| {
                    // Giving up an authorization only shrinks this frame's own auth scope, so it succeeds for any
                    // id: an id the frame never authorized is already in the state being asked for. That makes it
                    // answerless by construction, which is what keeps it from reporting whether a proof is live at
                    // an id the frame does not hold — the ids are a dense counter, so an answer would enumerate
                    // every proof in the transaction. `ProofAccess::drop` is the only route here, and a `Drop` has
                    // nowhere to report a failure, so a rejection would abort the transaction from a drop point
                    // the template author never wrote.
                    state.current_call_scope_mut()?.auth_scope_mut().remove_proof(&proof_id);

                    Ok(InvokeResult::unit())
                })
            },
            ProofAction::Drop => {
                let proof_id = proof_ref.proof_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "proof_ref",
                    reason: "Drop proof action requires a proof id".to_string(),
                })?;
                args.assert_no_args("Proof.Drop")?;

                self.tracker.write_with(|state| state.drop_proof(proof_id))?;

                Ok(InvokeResult::unit())
            },
        }
    }

    fn workspace_invoke(&mut self, action: WorkspaceAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("workspace_invoke")?;

        debug!(target: LOG_TARGET, "Workspace invoke: {:?}", action,);

        match action {
            // Names an output on the workspace so that you can refer to it as an
            // Arg::Variable
            WorkspaceAction::PutLastInstructionOutput => {
                let key = args.assert_one_arg()?;
                let last_output = self
                    .tracker
                    .take_last_instruction_output()
                    .ok_or(RuntimeError::NoLastInstructionOutput)?;

                self.validate_return_value(&last_output)?;

                self.tracker
                    .with_workspace_mut(|workspace| workspace.insert(key, last_output))?;
                Ok(InvokeResult::unit())
            },
            WorkspaceAction::Get => {
                let id: WorkspaceOffsetId = args.assert_one_arg()?;
                self.tracker.read_with(|state| {
                    let value =
                        state
                            .workspace()
                            .get(id)?
                            .cloned()
                            .ok_or_else(|| RuntimeError::ItemNotOnWorkspace {
                                id,
                                existing_ids: state.workspace().all_ids_iter().collect(),
                            })?;
                    Ok(InvokeResult::from_value(value)?)
                })
            },

            WorkspaceAction::DropAllProofs => {
                args.assert_no_args("WorkspaceAction::DropAllProofs")?;
                let proofs = self.tracker.with_workspace_mut(|workspace| workspace.take_all_proofs());

                self.tracker.write_with(|state| {
                    for proof_id in proofs {
                        state.drop_proof(proof_id)?;
                    }
                    Ok(InvokeResult::unit())
                })
            },
            WorkspaceAction::Assert => {
                args.assert_n_args(2)?;
                let key: WorkspaceOffsetId = args.get(0)?;
                let assertion: Assertion = args.get(1)?;

                self.tracker.read_with(|state| {
                    state.workspace_assert(key, assertion)?;
                    Ok(InvokeResult::unit())
                })
            },
            WorkspaceAction::DropAll => {
                args.assert_no_args("WorkspaceAction::DropAll")?;
                let proofs = self.tracker.with_workspace_mut(|workspace| {
                    workspace.clear_items();
                    workspace.take_all_proofs()
                });

                self.tracker.write_with(|state| {
                    for proof_id in proofs {
                        state.drop_proof(proof_id)?;
                    }
                    Ok(InvokeResult::unit())
                })
            },
        }
    }

    fn non_fungible_invoke(
        &mut self,
        nf_addr: NonFungibleAddress,
        action: NonFungibleAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("non_fungible_invoke")?;
        debug!(
            target: LOG_TARGET,
            "NonFungible invoke: {} {:?}",
            nf_addr,
            action,
        );

        match action {
            NonFungibleAction::GetData => {
                args.assert_no_args("NonFungibleAction::GetData")?;
                self.tracker.write_with(|state| {
                    let nft_lock = state.read_lock_substate(SubstateId::NonFungible(nf_addr.clone()))?;
                    let nft = state.get_non_fungible(&nft_lock)?;
                    let contents = nft
                        .contents()
                        .ok_or_else(|| RuntimeError::InvalidOpNonFungibleBurnt {
                            op: "GetData",
                            resource_address: *nf_addr.resource_address(),
                            nf_id: nf_addr.id().clone(),
                        })?
                        .data()
                        .clone();
                    state.unlock_substate(nft_lock)?;
                    Ok(InvokeResult::from_value(contents)?)
                })
            },
            NonFungibleAction::GetMutableData => {
                args.assert_no_args("NonFungibleAction::GetMutableData")?;

                self.tracker.write_with(|state| {
                    let nft_lock = state.read_lock_substate(SubstateId::NonFungible(nf_addr.clone()))?;
                    let nft = state.get_non_fungible(&nft_lock)?;
                    let contents = nft
                        .contents()
                        .ok_or_else(|| RuntimeError::InvalidOpNonFungibleBurnt {
                            op: "GetMutableData",
                            resource_address: *nf_addr.resource_address(),
                            nf_id: nf_addr.id().clone(),
                        })?
                        .mutable_data()
                        .clone();
                    state.unlock_substate(nft_lock)?;

                    Ok(InvokeResult::from_value(contents)?)
                })
            },
        }
    }

    fn consensus_invoke(&mut self, action: ConsensusAction) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("consensus_invoke")?;
        match action {
            ConsensusAction::GetCurrentEpoch => {
                let epoch = self.tracker.get_current_epoch()?;
                Ok(InvokeResult::encode(&epoch)?)
            },
            ConsensusAction::GetCurrentEpochHash => {
                let hash = self.tracker.get_current_epoch_hash()?;
                Ok(InvokeResult::encode(&hash)?)
            },
        }
    }

    fn generate_random_invoke(&mut self, action: GenerateRandomAction) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("generate_random_invoke")?;
        match action {
            GenerateRandomAction::GetRandomBytes { len } => {
                let len = len as usize;
                if len > limits::ENGINE_LIMITS.max_random_bytes_len {
                    return Err(LimitError::MaxRandomBytesLenExceeded { len }.into());
                }
                let random = self.tracker.get_pseudorandom_bytes(len)?;
                Ok(InvokeResult::encode(&random)?)
            },
        }
    }

    fn generate_uuid(&mut self) -> Result<[u8; 32], RuntimeError> {
        self.invoke_modules_on_runtime_call("generate_uuid")?;
        self.tracker.read_with(|state| {
            let epoch_hash = state.get_current_epoch_hash()?;
            let id_provider = state.id_provider()?;
            Ok(id_provider.new_uuid(&epoch_hash)?)
        })
    }

    fn set_last_instruction_output(&mut self, value: IndexedValue) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("set_last_instruction_output")?;
        self.tracker.write_with(|state| {
            state.set_last_instruction_output(value);
        });
        Ok(())
    }

    fn claim_burn(
        &mut self,
        claim: MinotariBurnClaimProof,
        output_data: ClaimBurnOutputData,
    ) -> Result<(), RuntimeError> {
        let epoch = self.tracker.get_current_epoch()?;
        self.tracker
            .charge_native_execution(tari_engine_types::limits::NativeExecutionPoints::PER_CLAIM_BURN)?;
        self.claim_burn_proof_verifier
            .verify_claim_proof(epoch, &self.seal_signer_public_key, &claim)
            .map_err(|e| {
                warn!(target: LOG_TARGET, "Claim burn failed - proof verification failed: {}", e);
                RuntimeError::InvalidClaimProof { details: e }
            })?;

        self.tracker.write_with(|state_mut| {
            // 2. Create a tombstone
            let address = ClaimedOutputTombstoneAddress::from_commitment(claim.commitment);
            state_mut.new_substate(address, ClaimedOutputTombstone { value: claim.value })?;

            // 3. Create the stealth UTXO.
            // This mints value with no `increase_total_supply` counterpart, which is sound only because the genesis
            // TARI resource both disables supply tracking and denies `Burn` (see `get_stealth_tari_resource`).
            // Enabling either would need an increase here to balance the decrease that
            // `ResourceAction::StealthUtxoBurn` applies when the UTXO is burnt.
            let address = UtxoAddress::new(TARI_TOKEN, claim.commitment.into());
            let utxo = Utxo::new(UtxoOutput {
                output: OutputBody {
                    public_nonce: claim.burn_public_key,
                    encrypted_data: output_data.encrypted_data,
                    minimum_value_promise: 0,
                    viewable_balance: None,
                },
                auth: SpendAuthorization::Key(self.seal_signer_public_key),
                tag: UtxoTag::new(0),
            });

            state_mut.new_substate(address, utxo)?;

            Ok::<_, RuntimeError>(())
        })?;

        Ok(())
    }

    fn claim_validator_fees(
        &mut self,
        pool_address: ValidatorFeePoolAddress,
        max_amount: Option<Amount>,
    ) -> Result<(), RuntimeError> {
        self.tracker.write_with(|state| {
            let resource = match max_amount {
                Some(max_amount) => state.withdraw_fees_from_pool_up_to(pool_address, max_amount)?,
                None => state.withdraw_all_fees_from_pool(pool_address)?,
            };
            let bucket_id = state.new_bucket_id();
            state.new_bucket(bucket_id, resource)?;
            state.set_last_instruction_output(IndexedValue::from_type(&bucket_id)?);
            Ok(())
        })
    }

    fn metered_fee_receipt(&self) -> FeeReceipt {
        FeeReceipt::builder()
            .with_cost_breakdown(self.tracker.fee_charges())
            .build()
    }

    fn required_fee_payment(&self) -> u64 {
        self.tracker.required_fee_payment()
    }

    fn checkpoint_fee_intent(&mut self) -> Result<(), RuntimeError> {
        // Price the state the fee intent ended on before testing what was paid against it. This is
        // the state a transaction that cannot afford its main intent falls back to committing, so a
        // payment that cannot cover it cannot commit anything at all — better established here,
        // before the main instructions run, than after they have consumed compute nobody pays for.
        self.invoke_modules_on_fee_checkpoint()?;
        if !self.tracker.is_fee_state_dry_run() && self.tracker.total_fee_payments() < self.tracker.total_fee_charges()
        {
            return Err(RuntimeError::InsufficientFeesPaid {
                required_fee: self.tracker.required_fee_payment(),
                fees_paid: self.tracker.total_fee_payments(),
            });
        }
        self.tracker.fee_checkpoint()
    }

    fn finalize(&mut self) -> Result<FinalizeResult, RuntimeError> {
        // Finalization adds no fee charge of its own. The template never invoked it, and the compute
        // allowance is sized against the charges standing when it is computed, so anything charged
        // afterwards puts the total past what that allowance was sized to fit inside.
        self.finalize_with(None)
    }

    fn finalize_failure(&mut self, reason: RejectReason) -> Result<FinalizeResult, RuntimeError> {
        self.finalize_with(Some(reason))
    }

    fn validate_finalized(&self) -> Result<(), RuntimeError> {
        self.tracker.read_with(|state| {
            state.validate_finalized()?;
            Ok(())
        })
    }

    fn caller_context_invoke(
        &mut self,
        action: CallerContextAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("caller_context_invoke")?;

        match action {
            CallerContextAction::GetCallerPublicKey => {
                args.assert_no_args("CallerContextAction::GetCallerPublicKey")?;
                Ok(InvokeResult::encode(&self.seal_signer_public_key)?)
            },
            CallerContextAction::GetComponentAddress => self.tracker.read_with(|state| {
                args.assert_no_args("CallerContextAction::GetComponentAddress")?;
                let call_scope = state.current_call_scope()?;
                let maybe_address = call_scope
                    .get_current_component_lock()
                    .map(|l| l.substate_id().as_component_address().unwrap());
                Ok(InvokeResult::encode(&maybe_address)?)
            }),
            CallerContextAction::GetSignerProof => {
                let public_key = args
                    .get_opt::<RistrettoPublicKeyBytes>(0)?
                    .unwrap_or(self.seal_signer_public_key);

                self.tracker.write_with(|state_mut| {
                    let call_scope = state_mut.current_call_scope()?;
                    let badge = NonFungibleAddress::from_public_key(public_key);
                    if !call_scope.auth_scope().contains_badge(&badge) {
                        return Err(RuntimeError::SignerBadgeNotInScope { public_key });
                    }

                    let proof_id = state_mut.id_provider()?.new_proof_id();
                    let resx = ResourceContainer::public_key(public_key);
                    let locked = LockedResource::new(ContainerRef::Runtime, resx);
                    state_mut.new_proof(proof_id, locked)?;

                    Ok(InvokeResult::encode(&proof_id)?)
                })
            },
        }
    }

    fn allocate_address_invoke(&mut self, action: AddressAllocationInvokeArg) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("allocate_address_invoke")?;

        self.tracker.write_with(|state| {
            match action {
                AddressAllocationInvokeArg::GetAddress(id) => {
                    let allocation = state.get_allocated_address(id)?;
                    match allocation.substate_id() {
                        SubstateId::Component(addr) => Ok(InvokeResult::encode(&addr)?),
                        SubstateId::Resource(addr) => Ok(InvokeResult::encode(&addr)?),
                        // Engine creates the allocations, so never creates other unsupported variants
                        _ => unreachable!("Invalid SubstateId variant. Allocation created for unsupported substate"),
                    }
                },
                AddressAllocationInvokeArg::CreateComponentAllocation { public_key } => {
                    // Validate the public key
                    let _ignore = public_key
                        .map(|pk| {
                            RistrettoPublicKey::from_canonical_bytes(pk.as_bytes()).map_err(|_| {
                                RuntimeError::InvalidArgument {
                                    argument: "public_key",
                                    reason: "Invalid RistrettoPublicKeyBytes".to_string(),
                                }
                            })
                        })
                        .transpose()?;

                    let template = state.current_template()?;
                    let id_provider = state.id_provider()?;
                    let address = public_key
                        .as_ref()
                        .map(|public_key| id_provider.derive_new_component_address(template, public_key))
                        .unwrap_or_else(|| id_provider.new_component_address())?;

                    let id = state.new_address_allocation(address)?;
                    Ok(InvokeResult::encode(&ComponentAddressAllocation::new(id))?)
                },
                AddressAllocationInvokeArg::CreateResourceAllocation => {
                    let address = state.id_provider()?.new_resource_address()?;
                    let id = state.new_address_allocation(address)?;
                    Ok(InvokeResult::encode(&ResourceAddressAllocation::new(id))?)
                },
            }
        })
    }

    fn call_invoke(&mut self, action: CallAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("call_invoke")?;
        self.tracker.read_with(|state| {
            let frame = state.current_call_frame()?;
            if !frame.is_cross_template_calls_allowed() {
                return Err(RuntimeError::CrossTemplateCallNotAllowed { action });
            }
            Ok(())
        })?;

        debug!(
            target: LOG_TARGET,
            "Call invoke: {:?} {:?}",
            action,
            args,
        );

        let exec_result = match action {
            CallAction::CallFunction => {
                let CallFunctionArg {
                    template_address,
                    function,
                    args,
                } = args.assert_one_arg()?;

                self.invoke_template_function(
                    &template_address,
                    &function,
                    args.into_iter().map(InstructionArg::Literal).collect(),
                )?
            },
            CallAction::CallMethod => {
                let CallMethodArg {
                    component_address,
                    method,
                    args,
                } = args.assert_one_arg()?;

                self.invoke_component_method(component_address, &method, args)?
            },
        };

        Ok(InvokeResult::from_value(exec_result.indexed.into_value())?)
    }

    fn builtin_template_invoke(&mut self, action: BuiltinTemplateAction) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("builtin_template_invoke")?;

        let address = match action {
            BuiltinTemplateAction::GetTemplateAddress { builtin: bultin } => match bultin {
                BuiltinTemplate::Account => ACCOUNT_TEMPLATE_ADDRESS,
                BuiltinTemplate::AccountNft => NFT_FAUCET_TEMPLATE_ADDRESS,
            },
        };

        Ok(InvokeResult::encode(&address)?)
    }

    fn check_component_access_rules(&self, method: &str) -> Result<(), RuntimeError> {
        self.tracker
            .read_with(|state| state.authorization().check_current_component_access_rules(method))
    }

    fn check_signer_badge_in_scope(&self, public_key: RistrettoPublicKeyBytes) -> Result<(), RuntimeError> {
        self.tracker.read_with(|state| {
            let badge = NonFungibleAddress::from_public_key(public_key);
            if !state.base_call_scope().auth_scope().contains_badge(&badge) {
                return Err(RuntimeError::SignerBadgeNotInScope { public_key });
            }
            Ok(())
        })
    }

    fn check_component_ownership(&self, action: ActionIdent) -> Result<(), RuntimeError> {
        self.tracker.read_with(|state| {
            let locked = state
                .current_call_scope()?
                .get_current_component_lock()
                .ok_or_else(|| RuntimeError::InvariantError {
                    function: "check_component_ownership",
                    details: "No current component lock in call scope".to_string(),
                })?;

            let component = state.get_component(locked)?;
            state
                .authorization()
                .require_ownership(action, component.as_ownership())
        })
    }

    fn update_component_template(&mut self, new_template: TemplateAddress) -> Result<(), RuntimeError> {
        self.tracker.write_with(|state_mut| {
            let locked = state_mut
                .current_call_scope()?
                .get_current_component_lock()
                .cloned()
                .ok_or_else(|| RuntimeError::InvariantError {
                    function: "update_component_template",
                    details: "No current component lock in call scope".to_string(),
                })?;

            let component_mut = state_mut.get_component_mut(&locked)?;
            let prev_template = *component_mut.template_address();
            component_mut.set_template_address(new_template);

            state_mut.push_event(Event::std(
                Some(locked.substate_id().clone()),
                new_template,
                "component",
                "template_update",
                metadata!["prev_template" => prev_template.to_string()],
            ))?;
            Ok(())
        })
    }

    fn validate_return_value(&self, value: &IndexedValue) -> Result<(), RuntimeError> {
        self.tracker
            .read_with(|state| state.check_all_substates_known(value.well_known_types()))
    }

    fn push_call_frame(&mut self, frame: PushCallFrame) -> Result<(), RuntimeError> {
        self.tracker.push_call_frame(frame)?;
        // Spend-script predicates and auth hooks are invoked via the generic `call_function` / `call_method` paths,
        // so we restrict the frame they just pushed here rather than threading a flag through those paths. The WASM
        // only runs after this returns, so the restriction is in place before any host op can be issued.
        if let Some(mode) = self.restricted_frame_pending.take() {
            self.tracker.write_with(|state| state.restrict_current_frame(mode))?;
        }
        Ok(())
    }

    fn pop_call_frame(&mut self, returned: &IndexedWellKnownTypes) -> Result<(), RuntimeError> {
        self.tracker.pop_call_frame(returned)?;
        Ok(())
    }

    fn publish_template(
        &mut self,
        template: TemplateBlob,
        metadata_hash: Option<MetadataHash>,
        template_def: TemplateDef,
    ) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("publish_template")?;
        self.tracker.write_with(|state_mut| {
            let template_byte_size = template.len();
            let code_hash = hash_template_code(&template);
            let template_address =
                PublishedTemplateAddress::from_author_and_binary_hash(&self.seal_signer_public_key, &code_hash);
            let epoch = state_mut.get_current_epoch()?;
            let template_name = template_def
                .template_name()
                .try_into()
                .map_err(|_| RuntimeError::InvalidArgument {
                    argument: "template_name",
                    reason: format!(
                        "Template name exceeds maximum length of {} bytes",
                        limits::ENGINE_LIMITS.max_template_name_length
                    ),
                })?;
            state_mut.new_substate(
                template_address,
                SubstateValue::Template(PublishedTemplate {
                    template_name,
                    binary: template,
                    author: self.seal_signer_public_key,
                    at_epoch: epoch.as_u64(),
                    metadata_hash: metadata_hash.clone(),
                }),
            )?;
            // Mark template substate as owned by current call stack
            let scope_mut = state_mut.current_call_scope_mut()?;
            scope_mut.move_node_to_owned(&template_address.into())?;
            // Publish template event
            let mut metadata = Metadata::new();
            metadata.insert("template_byte_size".to_string(), template_byte_size.to_string());
            state_mut.push_event(Event::std(
                Some(template_address.into()),
                template_address.as_hash(),
                "template",
                "publish",
                metadata,
            ))?;

            Ok(())
        })
    }

    fn put_on_workspace(&mut self, id: WorkspaceId, value: IndexedValue) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("put_on_workspace")?;

        self.validate_return_value(&value)?;

        self.tracker
            .with_workspace_mut(|workspace| workspace.insert(id, value))?;
        Ok(())
    }

    fn intrinsic_invoke(&mut self, intrinsic: IntrinsicId, args: EngineArgs) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("intrinsic_invoke")?;

        // Priced from the declared arguments and charged before the work runs, so a transaction that
        // cannot afford an intrinsic traps without the validator having performed it.
        let points = intrinsics::price(intrinsic, &args)?;
        self.tracker.charge_native_execution(points)?;

        intrinsics::dispatch(intrinsic, args)
    }

    fn spend_context_invoke(&mut self, action: SpendContextAction) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("spend_context_invoke")?;

        // Only reachable while a spend-script predicate is executing; `spend_exec_context` is set immediately before
        // the predicate is invoked and cleared immediately after.
        let ctx = self
            .spend_exec_context
            .as_ref()
            .ok_or(RuntimeError::SpendContextUnavailable)?;

        match action {
            SpendContextAction::Inputs => Ok(InvokeResult::encode(&ctx.inputs)?),
            SpendContextAction::Outputs => Ok(InvokeResult::encode(&ctx.outputs)?),
            SpendContextAction::CurrentInput => Ok(InvokeResult::encode(&CurrentInputView {
                index: ctx.current_input_index,
                commitment: ctx.current_input_commitment,
                condition_root: Some(ctx.current_input_condition_root),
            })?),
            SpendContextAction::RevealedInputAmount => Ok(InvokeResult::encode(&ctx.revealed_input_amount)?),
            SpendContextAction::RevealedOutputAmount => Ok(InvokeResult::encode(&ctx.revealed_output_amount)?),
            SpendContextAction::AssertCovenantBalanced { max_revealed } => {
                Ok(InvokeResult::encode(&ctx.covenant_balanced(max_revealed))?)
            },
            SpendContextAction::WitnessData => Ok(InvokeResult::encode(&ctx.witness_data)?),
        }
    }

    /// Create a new address allocation for the provided substate type and entity id
    fn allocate_address(
        &mut self,
        substate_type: AllocatableAddressType,
        entity_id: EntityId,
        workspace_id: WorkspaceId,
    ) -> Result<AllocateAddressResult, RuntimeError> {
        self.tracker.write_with(|state| {
            let id_provider = state.id_provider_for_entity(entity_id);

            match substate_type {
                AllocatableAddressType::Component => {
                    let address = id_provider.new_component_address()?;
                    let id = state.new_address_allocation(address)?;
                    let value = IndexedValue::from_type(&ComponentAddressAllocation::new(id))?;
                    state.workspace_mut().insert(workspace_id, value)?;
                    Ok(AllocateAddressResult::ComponentAddress(
                        ComponentAddressAllocation::new(id),
                    ))
                },
                AllocatableAddressType::Resource => {
                    let address = id_provider.new_resource_address()?;
                    let id = state.new_address_allocation(address)?;
                    let value = IndexedValue::from_type(&ResourceAddressAllocation::new(id))?;
                    state.workspace_mut().insert(workspace_id, value)?;
                    Ok(AllocateAddressResult::ResourceAddress(ResourceAddressAllocation::new(
                        id,
                    )))
                },
            }
        })
    }

    fn stealth_transfer(
        &mut self,
        resource_address: ResourceAddressRef,
        statement: StealthTransferStatement,
        revealed_funds_bucket_id: Option<BucketId>,
    ) -> Result<Option<BucketId>, RuntimeError> {
        // (T1) Creation time: a stealth output commits only an opaque `condition_root`, so its leaves are hidden and
        // cannot be validated at creation — that happens at spend time when a leaf is revealed (below / inline). The
        // only creation-time invariant is that an output is spendable by at least one path (`spend_key` or
        // `condition_root` is `Some`), enforced by `validate_stealth_outputs_statement` during execution.

        // The fee intent runs on free-compute credit, so the transfers it may perform are capped. Checked first, so
        // an over-cap fee intent is rejected before any substate read, charge or crypto. Both the `StealthTransfer`
        // instruction and a template's `ResourceManager::stealth_transfer` reach here, so neither route escapes it.
        self.tracker.account_fee_intent_stealth_transfer()?;

        // The whole statement's native verification cost is charged against the payment-funded
        // allowance before any of it runs (the authorisation pass below included), so a transaction
        // that will not pay traps here without extracting the crypto work. The resource peek is a
        // cheap substate read that determines whether the viewable-balance surcharge applies.
        let has_view_key = self.tracker.write_with(|state| {
            let address = state.resolve_resource_address_ref(resource_address.clone())?;
            let resource_lock = state.read_lock_substate(SubstateId::Resource(address))?;
            let has_view_key = state.get_resource(&resource_lock)?.view_key().is_some();
            state.unlock_substate(resource_lock)?;
            Ok::<_, RuntimeError>(has_view_key)
        })?;
        self.tracker
            .charge_native_execution(tari_engine_types::stealth::transfer_native_points(
                &statement,
                has_view_key,
            ))?;

        // (T2) Spend time: authorise every input BEFORE the spend executes, so a rejection leaves the inputs unspent.
        // This is the authoritative, mandatory security gate for all spend paths (key path, AccessRule, and WASM
        // predicate).
        //
        // Covenant soundness invariant: a covenant balance proof binds the partition's output commitments but trusts
        // their values to be in range; `execute_stealth_transfer` below range-proofs those same `statement` outputs.
        // Both run pre-commit in this one atomic call, so the two MUST stay coupled — decoupling them would reopen a
        // wraparound forgery in the covenant check.
        self.verify_input_authorizations(resource_address.clone(), &statement)?;

        self.tracker.write_with(|state_mut| {
            let Some(container) =
                state_mut.execute_stealth_transfer(resource_address, statement, revealed_funds_bucket_id)?
            else {
                return Ok(None);
            };
            let bucket_id = state_mut.new_bucket_id();
            state_mut.new_bucket(bucket_id, container)?;
            Ok(Some(bucket_id))
        })
    }

    fn pay_fee(&mut self, pay_fee: PayFee) -> Result<(), RuntimeError> {
        if self.tracker.is_fee_intent_checkpointed() {
            return Err(RuntimeError::FeePaymentInMainIntent);
        }

        self.tracker.write_with(|state_mut| {
            match pay_fee {
                PayFee::FromBucket { bucket } => {
                    let value = state_mut
                        .workspace()
                        .get(bucket)?
                        .ok_or_else(|| RuntimeError::ItemNotOnWorkspace {
                            id: bucket,
                            existing_ids: state_mut.workspace().all_ids_iter().collect(),
                        })?;
                    let input_bucket =
                        tari_bor::from_value::<BucketId>(value).map_err(|e| RuntimeError::InvalidArgument {
                            argument: "bucket",
                            reason: format!("PayFee::FromBucket: Expected workspace ID to contain a BucketId: {e}"),
                        })?;
                    let bucket = state_mut.take_bucket(input_bucket)?;

                    // No refunds
                    state_mut.pay_fee(bucket.take_all(), None)?;
                    Ok(())
                },
            }
        })
    }

    fn track_template_loaded(
        &mut self,
        template_address: &TemplateAddress,
        bytes_loaded: usize,
    ) -> Result<(), RuntimeError> {
        // Built-in templates are zero-cost
        if is_builtin_template_address(template_address) {
            return Ok(());
        }

        // Note: per-transaction dedup is intentionally pushed into the modules that want it (see
        // `FeeModule::on_template_loaded`), so observer-style modules continue to see every load.
        for module in self.modules.iter() {
            module.on_template_loaded(&mut self.tracker, template_address, bytes_loaded)?;
        }
        Ok(())
    }

    fn record_wasm_execution(&mut self, points_consumed: u64) -> Result<(), RuntimeError> {
        // Accumulate into the transaction-wide total unconditionally (not via a module) so the
        // per-transaction budget is enforced even when fee charging is disabled. The fee module
        // reads this total in `on_before_finalize` to compute the WASM execution charge.
        self.tracker.accumulate_wasm_points(points_consumed);
        for module in self.modules.iter() {
            module.on_wasm_execution(&mut self.tracker, points_consumed)?;
        }
        Ok(())
    }

    fn wasm_points_consumed(&self) -> u64 {
        self.tracker.accumulated_wasm_points()
    }

    fn native_points_consumed(&self) -> u64 {
        self.tracker.accumulated_native_points()
    }

    fn compute_allowance(&self) -> Option<ComputeAllowance> {
        self.tracker.compute_allowance()
    }

    fn resolve_args(
        &self,
        prepend: Option<InstructionArg>,
        args: &[InstructionArg],
    ) -> Result<Vec<tari_bor::Value>, RuntimeError> {
        let prepend_len = usize::from(prepend.is_some());
        let total_len = prepend_len + args.len();
        if total_len > limits::WASM_LIMITS.max_function_arguments {
            return Err(ArgumentValidationError::TooManyArguments {
                got: total_len,
                max: limits::WASM_LIMITS.max_function_arguments,
            }
            .into());
        }

        prepend
            .iter()
            .chain(args.iter())
            .map(|arg| match arg {
                InstructionArg::Workspace(id) => self.resolve_workspace_id(id),
                InstructionArg::Literal(v) => Ok(decode_exact(v)?),
                // A blob arg's value is the raw bytes of the referenced blob, decoded as CBOR
                // by the same path as `Literal`. Lookup is against the surrounding transaction's
                // `Blobs`, set on the runtime at construction time.
                InstructionArg::Blob(idx) => {
                    let blob = self
                        .blobs
                        .get(*idx)
                        .ok_or(ArgumentValidationError::BlobIndexOutOfBounds {
                            index: *idx,
                            count: self.blobs.len(),
                        })?;
                    Ok(decode_exact(blob.as_bytes())?)
                },
            })
            .collect()
    }

    fn resolve_workspace_id(&self, workspace_id: &WorkspaceOffsetId) -> Result<tari_bor::Value, RuntimeError> {
        self.tracker.with_workspace(|workspace| {
            let id = *workspace_id;
            workspace.get(id).map(|opt| opt.cloned()).map(|opt| {
                opt.ok_or_else(|| RuntimeError::ItemNotOnWorkspace {
                    id,
                    existing_ids: workspace.all_ids_iter().collect(),
                })
            })
        })?
    }

    fn set_runtime_pointer(&mut self, pointer: *mut Box<dyn RuntimeInterface>) {
        self.runtime_pointer = NonNull::new(pointer);
    }
}

fn validate_component_access_rule_methods(
    access_rules: &ComponentAccessRules,
    template_def: &TemplateDef,
) -> Result<(), RuntimeError> {
    for (name, _) in access_rules.method_access_rules_iter() {
        if template_def.functions().iter().all(|f| f.name != *name) {
            return Err(RuntimeError::InvalidMethodAccessRule {
                template_name: template_def.template_name().to_string(),
                details: format!("No method '{}' found in template", name),
            });
        }
    }
    Ok(())
}
