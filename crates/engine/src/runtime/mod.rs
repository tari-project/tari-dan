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

mod auth;
pub use auth::{AuthParams, AuthorizationScope};

mod r#impl;
pub use r#impl::RuntimeInterfaceImpl;

mod engine_args;
pub use crate::runtime::engine_args::{EngineArgChargeError, EngineArgs};

mod error;
pub use error::*;

mod actions;
pub use actions::*;

mod module;
pub use module::{RuntimeEvent, RuntimeModule, RuntimeModuleError};

mod fee_state;
mod tracker;

mod locking;
pub mod scope;
pub use locking::{LockError, LockState};
mod address_allocation;
mod pay_fee;
mod spend_script_execution;
mod state_store;
mod tracker_auth;
mod validation;
mod working_state;
mod workspace;

use std::{cell::RefCell, fmt::Debug, rc::Rc};

pub use pay_fee::PayFee;
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason},
    confidential::{ClaimBurnOutputData, MinotariBurnClaimProof},
    fees::FeeReceipt,
    indexed_value::{IndexedValue, IndexedWellKnownTypes},
    limits::{CompileCounts, ModuleShape},
    lock::LockFlag,
    published_template::TemplateBlob,
};
use tari_ootle_transaction::{
    AllocatableAddressType,
    ComponentReference,
    ResourceAddressRef,
    args::{InstructionArg, WorkspaceId, WorkspaceOffsetId},
};
use tari_template_abi::TemplateDef;
use tari_template_lib::{
    args::{
        AddressAllocationInvokeArg,
        AllocateAddressResult,
        BucketAction,
        BucketRef,
        BuiltinTemplateAction,
        CallAction,
        CallerContextAction,
        ComponentAction,
        ComponentRef,
        ConsensusAction,
        GenerateRandomAction,
        InvokeResult,
        NonFungibleAction,
        ProofAction,
        ProofRef,
        ResourceAction,
        ResourceRef,
        SpendContextAction,
        VaultAction,
        WorkspaceAction,
    },
    models::{BucketId, VaultRef},
    types::{
        Amount,
        ComponentAddress,
        EntityId,
        LogLevel,
        Metadata,
        NonFungibleAddress,
        TemplateAddress,
        TemplateMetadataHash,
        ValidatorFeePoolAddress,
        crypto::RistrettoPublicKeyBytes,
        engine_args::IntrinsicId,
        stealth::StealthTransferStatement,
    },
};
pub use tracker::{ComputeAllowance, ComputeFunding, DeclaredInputLoad, FinalizedState, StateTracker};
pub use working_state::ChargeableState;

use crate::{
    runtime::{
        locking::LockedSubstate,
        scope::{FrameWriteMode, PushCallFrame},
    },
    wasm::WasmInstanceCache,
};

/// What a call on a component needs of it: the header fields its frame runs under, and the
/// substates its state holds.
#[derive(Debug, Clone)]
pub struct LoadedComponent {
    pub address: ComponentAddress,
    pub template_address: TemplateAddress,
    pub entity_id: EntityId,
    pub state_scope: IndexedWellKnownTypes,
}

pub trait RuntimeInterface {
    fn next_entity_id(&self) -> Result<EntityId, RuntimeError>;
    fn emit_event(&self, topic: String, payload: Metadata) -> Result<(), RuntimeError>;

    fn emit_log(&self, level: LogLevel, message: String) -> Result<(), RuntimeError>;

    /// Loads a component and indexes its state, charging
    /// [`tari_engine_types::limits::component_load_points`] for what the state holds before
    /// indexing it. The state itself stays in the store.
    fn load_component(&self, call: ComponentReference) -> Result<LoadedComponent, RuntimeError>;

    fn lock_component(&self, address: ComponentAddress, lock_flag: LockFlag) -> Result<LockedSubstate, RuntimeError>;

    fn component_invoke(
        &self,
        component_ref: ComponentRef,
        action: ComponentAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn resource_invoke(
        &self,
        resource_ref: ResourceRef,
        action: ResourceAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn vault_invoke(
        &self,
        vault_ref: VaultRef,
        action: VaultAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn bucket_invoke(
        &self,
        bucket_ref: BucketRef,
        action: BucketAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn proof_invoke(
        &self,
        proof_ref: ProofRef,
        action: ProofAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;
    fn workspace_invoke(&self, action: WorkspaceAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError>;

    fn non_fungible_invoke(
        &self,
        nf_addr: NonFungibleAddress,
        action: NonFungibleAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn consensus_invoke(&self, action: ConsensusAction) -> Result<InvokeResult, RuntimeError>;

    fn generate_random_invoke(&self, action: GenerateRandomAction) -> Result<InvokeResult, RuntimeError>;

    fn generate_uuid(&self) -> Result<[u8; 32], RuntimeError>;

    /// Ends a top-level instruction with what it produced, if anything. `Some` becomes the output a following
    /// `PutLastInstructionOutputOnWorkspace` names; `None` leaves the output of an earlier instruction in place.
    fn complete_instruction(&self, output: Option<IndexedValue>) -> Result<(), RuntimeError>;

    fn claim_burn(&self, claim: MinotariBurnClaimProof, output_data: ClaimBurnOutputData) -> Result<(), RuntimeError>;

    fn claim_validator_fees(
        &self,
        address: ValidatorFeePoolAddress,
        max_amount: Option<Amount>,
    ) -> Result<BucketId, RuntimeError>;

    fn checkpoint_fee_intent(&self) -> Result<(), RuntimeError>;
    fn finalize(&self) -> Result<FinalizeResult, RuntimeError>;
    fn finalize_failure(&self, reason: RejectReason) -> Result<FinalizeResult, RuntimeError>;
    fn validate_finalized(&self) -> Result<(), RuntimeError>;

    fn caller_context_invoke(
        &self,
        action: CallerContextAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn allocate_address_invoke(&self, action: AddressAllocationInvokeArg) -> Result<InvokeResult, RuntimeError>;

    fn call_invoke(&self, action: CallAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError>;

    fn builtin_template_invoke(&self, action: BuiltinTemplateAction) -> Result<InvokeResult, RuntimeError>;

    /// Checks whether the current execution context has access to the given component method.
    fn check_component_access_rules(&self, method: &str) -> Result<(), RuntimeError>;
    /// Checks whether the current execution context has owner permission of the given component.
    fn check_component_ownership(&self, action: ActionIdent) -> Result<(), RuntimeError>;

    /// Ends the current frame's call boundary by revoking the proofs that were in scope only for the boundary
    /// check. Must run after the frame's access rule has been evaluated and before anything acts in the frame.
    fn revoke_boundary_proofs(&self) -> Result<(), RuntimeError>;

    /// Asserts that the signer badge for `public_key` is in the transaction's base auth scope, i.e. that the
    /// transaction is signed by that key.
    fn check_signer_badge_in_scope(&self, public_key: RistrettoPublicKeyBytes) -> Result<(), RuntimeError>;

    fn update_component_template(&self, new_template: TemplateAddress) -> Result<(), RuntimeError>;

    fn validate_return_value(&self, value: &IndexedValue) -> Result<(), RuntimeError>;

    fn push_call_frame(&self, frame: PushCallFrame, restrict_to: Option<FrameWriteMode>) -> Result<(), RuntimeError>;
    fn pop_call_frame(&self, returned: &IndexedWellKnownTypes) -> Result<(), RuntimeError>;
    fn publish_template(
        &self,
        template: TemplateBlob,
        metadata_hash: Option<TemplateMetadataHash>,
        template_def: TemplateDef,
    ) -> Result<(), RuntimeError>;
    fn put_on_workspace(&self, id: WorkspaceId, value: IndexedValue) -> Result<(), RuntimeError>;

    /// Runs a native intrinsic — a pure function of its arguments, priced from those arguments and
    /// charged before it runs. Backs the `tari_template_lib::intrinsics` API.
    fn intrinsic_invoke(&self, intrinsic: IntrinsicId, args: EngineArgs) -> Result<InvokeResult, RuntimeError>;

    /// Read-only introspection over the spending `StealthTransferStatement`, available only while a spend-script
    /// predicate is executing. Backs the `SpendContext` template-lib API.
    fn spend_context_invoke(&self, action: SpendContextAction) -> Result<InvokeResult, RuntimeError>;

    fn allocate_address(
        &self,
        substate_type: AllocatableAddressType,
        entity_id: EntityId,
        workspace_id: WorkspaceId,
    ) -> Result<AllocateAddressResult, RuntimeError>;

    fn stealth_transfer(
        &self,
        resource_address: ResourceAddressRef,
        statement: StealthTransferStatement,
        revealed_funds_bucket: Option<BucketId>,
    ) -> Result<Option<BucketId>, RuntimeError>;

    fn pay_fee(&self, pay_fee: PayFee) -> Result<(), RuntimeError>;

    fn track_template_loaded(
        &self,
        template_address: &TemplateAddress,
        bytes_loaded: usize,
    ) -> Result<(), RuntimeError>;

    /// Records the number of Wasmer metering points consumed by a single WASM template invocation
    /// so that runtime modules (notably the fee module) can convert them into a fee charge, and adds
    /// them to the transaction-wide total. Called once per `WasmProcess::invoke`, including across
    /// nested cross-template calls.
    fn record_wasm_execution(&self, points_consumed: u64) -> Result<(), RuntimeError>;

    /// Charges the cost of building the `Store` and `Instance` a template call runs in, before the
    /// instance exists and before the first metered operator. Priced by
    /// [`tari_engine_types::limits::instantiation_points`] and charged against the same compute
    /// allowance as native verification, so a call that cannot cover it fails having done none of
    /// the work.
    fn charge_template_instantiation(&self, shape: &ModuleShape) -> Result<(), RuntimeError>;

    /// Charges the host-side handling of a value a template call returned, `bytes` long and holding
    /// `items` CBOR data items, before it is decoded. Priced by
    /// [`tari_engine_types::limits::return_value_points`] and charged against the same compute
    /// allowance as native verification.
    fn charge_return_value(&self, bytes: u64, items: u64) -> Result<(), RuntimeError>;

    /// Charges the host-side decode of CBOR a template wrote, `input`, before it is decoded: an
    /// engine call's argument, or a value nested in one. Priced by
    /// [`tari_engine_types::limits::engine_arg_points`] and charged against the same compute
    /// allowance as native verification.
    ///
    /// Counting the items is itself host work, so the walk that counts them reads only as many
    /// items as the transaction can still pay for, and checks their nesting depth on the way.
    /// Input holding more is charged what was read and fails out of compute.
    fn charge_engine_arg(&self, input: &[u8]) -> Result<(), EngineArgChargeError>;

    /// Charges the flat host-side cost of an engine call, before its argument is read.
    /// Priced by [`tari_engine_types::limits::NativeExecutionPoints::PER_ENGINE_CALL`] and charged
    /// against the same compute allowance as native verification.
    fn charge_engine_call(&self) -> Result<(), RuntimeError>;

    /// Charges an engine call's encoded `response` before it is written into the template's memory.
    /// Priced by [`tari_engine_types::limits::engine_response_points`] and charged against the same
    /// compute allowance as native verification. Its items are counted as
    /// [`Self::charge_engine_arg`] counts an argument's.
    fn charge_engine_response(&self, response: &[u8]) -> Result<(), RuntimeError>;

    /// The template instances this transaction has created. A borrow of it must not be held across a
    /// template call, which may reach it again through a nested call.
    fn wasm_instances(&self) -> &RefCell<WasmInstanceCache>;

    /// Charges the Cranelift compile a `PublishTemplate` instruction makes every validator run,
    /// before the compile starts. Priced by [`tari_engine_types::limits::template_compile_points`].
    fn charge_template_compile(&self, binary_bytes: u64, counts: &CompileCounts) -> Result<(), RuntimeError>;

    /// Total Wasmer metering points consumed by the transaction so far, across every template
    /// invocation. Used by `WasmProcess::invoke` to enforce `MAX_WASM_POINTS_PER_TRANSACTION`.
    fn wasm_points_consumed(&self) -> u64;

    /// Total native-verification points (stealth transfers, confidential withdraws, burn claims —
    /// priced in WASM-point equivalents) charged to the transaction so far. Combined with
    /// [`Self::wasm_points_consumed`] when capping a call's payment-funded allowance; excluded
    /// from the WASM-only per-transaction hard cap.
    fn native_points_consumed(&self) -> u64;

    /// The charges metered so far, as a receipt against no payment. A transaction rejected before
    /// finalization never settles a receipt of its own, so this is the only place the payer can read
    /// what it would have cost.
    fn metered_fee_receipt(&self) -> FeeReceipt;

    /// The payment those charges require — the figure a rejected payer has to raise their fee to.
    /// The exhaust burn is a share of what is paid rather than a charge, so it does not raise this.
    fn required_fee_payment(&self) -> u64;

    /// The maximum Wasmer metering points the transaction may consume given the fees paid so far,
    /// used by `WasmProcess::invoke` to cap each call's allowance so unpaid compute cannot exceed
    /// the grace. `None` means no payment-funded bound applies (only the per-transaction hard cap).
    fn compute_allowance(&self) -> Option<ComputeAllowance>;

    fn resolve_args(
        &self,
        prepend: Option<InstructionArg>,
        args: &[InstructionArg],
    ) -> Result<Vec<tari_bor::Value>, RuntimeError>;

    fn resolve_workspace_id(&self, workspace_id: &WorkspaceOffsetId) -> Result<tari_bor::Value, RuntimeError>;
}

/// The handle to the host interface that a WASM invocation carries as its function environment.
///
/// It is a shared handle: every frame of a transaction holds one over that transaction's state, and
/// each hands out `&dyn RuntimeInterface` only. A frame therefore never holds a unique borrow of the
/// interface while a nested call runs against it — the interface's own state is mutated internally,
/// under borrows that close before any nested call is made.
///
/// Frames reach that state two ways. [`Runtime::clone`] shares the interface itself, while
/// `RuntimeInterfaceImpl::for_nested_call` builds a new one over the same tracker, entity-id
/// provider, spend-script context and template instances. What is shared in both cases is the state; the interface is
/// shared only in the first.
#[derive(Clone)]
pub struct Runtime {
    interface: Rc<dyn RuntimeInterface>,
}

// SAFETY: wasmer requires `Send + Sync` of a `FunctionEnv`'s data. A `Runtime` satisfies neither
// structurally — its interface handle is `Rc` and the state behind it is `RefCell` — so what makes
// the impls sound is that a `Runtime` stays on one thread. Its whole lifetime is one execution: it
// is built in `TransactionProcessor::execute`, and a `Store` holds one only while
// `TransactionProcessor::invoke_template` runs a call on the thread executing the transaction, so
// every refcount touch — wasmer's own drop of the `FunctionEnv` data included — happens there. The
// stores themselves live in the transaction's instance cache and are dropped with it, on that
// same thread. Nothing a `Runtime` reaches outlives that execution: the handles
// `TransactionProcessor` is given from outside it, which do cross threads, stay `Arc`.
unsafe impl Sync for Runtime {}
// SAFETY: See the `Sync` impl above.
unsafe impl Send for Runtime {}

impl Runtime {
    /// `pub(crate)` so that the confinement the `Send`/`Sync` impls rest on is the crate boundary's
    /// to keep rather than a caller's to respect: `Runtime` is `Send`, so a constructor reachable
    /// from outside would put a handle on another thread from safe code.
    pub(crate) fn new(interface: Rc<dyn RuntimeInterface>) -> Self {
        Self { interface }
    }

    pub fn interface(&self) -> &dyn RuntimeInterface {
        &*self.interface
    }

    /// A handle to the interface that can outlive this borrow of the runtime.
    pub fn shared_interface(&self) -> Rc<dyn RuntimeInterface> {
        self.interface.clone()
    }
}

impl Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("interface", &"dyn RuntimeInterface")
            .finish()
    }
}
