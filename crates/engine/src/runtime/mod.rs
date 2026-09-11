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
pub use crate::runtime::engine_args::EngineArgs;

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

use std::{fmt::Debug, ptr::NonNull};

pub use pay_fee::PayFee;
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason},
    component::Component,
    confidential::{ClaimBurnOutputData, MinotariBurnClaimProof},
    fees::FeeReceipt,
    indexed_value::{IndexedValue, IndexedWellKnownTypes},
    lock::LockFlag,
    published_template::TemplateBlob,
};
use tari_ootle_template_metadata::MetadataHash;
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
        ValidatorFeePoolAddress,
        crypto::RistrettoPublicKeyBytes,
        engine_args::IntrinsicId,
        stealth::StealthTransferStatement,
    },
};
pub use tracker::{ComputeAllowance, ComputeFunding, FinalizedState, StateTracker};
pub use working_state::ChargeableState;

use crate::runtime::{locking::LockedSubstate, scope::PushCallFrame};

pub trait RuntimeInterface {
    fn next_entity_id(&mut self) -> Result<EntityId, RuntimeError>;
    fn emit_event(&mut self, topic: String, payload: Metadata) -> Result<(), RuntimeError>;

    fn emit_log(&mut self, level: LogLevel, message: String) -> Result<(), RuntimeError>;

    fn load_component(&mut self, call: ComponentReference) -> Result<(ComponentAddress, Component), RuntimeError>;

    fn lock_component(
        &mut self,
        address: ComponentAddress,
        lock_flag: LockFlag,
    ) -> Result<LockedSubstate, RuntimeError>;

    fn component_invoke(
        &mut self,
        component_ref: ComponentRef,
        action: ComponentAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn resource_invoke(
        &mut self,
        resource_ref: ResourceRef,
        action: ResourceAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn vault_invoke(
        &mut self,
        vault_ref: VaultRef,
        action: VaultAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn bucket_invoke(
        &mut self,
        bucket_ref: BucketRef,
        action: BucketAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn proof_invoke(
        &mut self,
        proof_ref: ProofRef,
        action: ProofAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;
    fn workspace_invoke(&mut self, action: WorkspaceAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError>;

    fn non_fungible_invoke(
        &mut self,
        nf_addr: NonFungibleAddress,
        action: NonFungibleAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn consensus_invoke(&mut self, action: ConsensusAction) -> Result<InvokeResult, RuntimeError>;

    fn generate_random_invoke(&mut self, action: GenerateRandomAction) -> Result<InvokeResult, RuntimeError>;

    fn generate_uuid(&mut self) -> Result<[u8; 32], RuntimeError>;

    fn set_last_instruction_output(&mut self, value: IndexedValue) -> Result<(), RuntimeError>;

    fn claim_burn(
        &mut self,
        claim: MinotariBurnClaimProof,
        output_data: ClaimBurnOutputData,
    ) -> Result<(), RuntimeError>;

    fn claim_validator_fees(
        &mut self,
        address: ValidatorFeePoolAddress,
        max_amount: Option<Amount>,
    ) -> Result<(), RuntimeError>;

    fn checkpoint_fee_intent(&mut self) -> Result<(), RuntimeError>;
    fn finalize(&mut self) -> Result<FinalizeResult, RuntimeError>;
    fn finalize_failure(&mut self, reason: RejectReason) -> Result<FinalizeResult, RuntimeError>;
    fn validate_finalized(&self) -> Result<(), RuntimeError>;

    fn caller_context_invoke(
        &mut self,
        action: CallerContextAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn allocate_address_invoke(&mut self, action: AddressAllocationInvokeArg) -> Result<InvokeResult, RuntimeError>;

    fn call_invoke(&mut self, action: CallAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError>;

    fn builtin_template_invoke(&mut self, action: BuiltinTemplateAction) -> Result<InvokeResult, RuntimeError>;

    /// Checks whether the current execution context has access to the given component method.
    fn check_component_access_rules(&self, method: &str) -> Result<(), RuntimeError>;
    /// Checks whether the current execution context has owner permission of the given component.
    fn check_component_ownership(&self, action: ActionIdent) -> Result<(), RuntimeError>;

    /// Asserts that the signer badge for `public_key` is in the transaction's base auth scope, i.e. that the
    /// transaction is signed by that key.
    fn check_signer_badge_in_scope(&self, public_key: RistrettoPublicKeyBytes) -> Result<(), RuntimeError>;

    fn update_component_template(&mut self, new_template: TemplateAddress) -> Result<(), RuntimeError>;

    fn validate_return_value(&self, value: &IndexedValue) -> Result<(), RuntimeError>;

    fn push_call_frame(&mut self, frame: PushCallFrame) -> Result<(), RuntimeError>;
    fn pop_call_frame(&mut self, returned: &IndexedWellKnownTypes) -> Result<(), RuntimeError>;
    fn publish_template(
        &mut self,
        template: TemplateBlob,
        metadata_hash: Option<MetadataHash>,
        template_def: TemplateDef,
    ) -> Result<(), RuntimeError>;
    fn put_on_workspace(&mut self, id: WorkspaceId, value: IndexedValue) -> Result<(), RuntimeError>;

    /// Runs a native intrinsic — a pure function of its arguments, priced from those arguments and
    /// charged before it runs. Backs the `tari_template_lib::intrinsics` API.
    fn intrinsic_invoke(&mut self, intrinsic: IntrinsicId, args: EngineArgs) -> Result<InvokeResult, RuntimeError>;

    /// Read-only introspection over the spending `StealthTransferStatement`, available only while a spend-script
    /// predicate is executing. Backs the `SpendContext` template-lib API.
    fn spend_context_invoke(&mut self, action: SpendContextAction) -> Result<InvokeResult, RuntimeError>;

    fn allocate_address(
        &mut self,
        substate_type: AllocatableAddressType,
        entity_id: EntityId,
        workspace_id: WorkspaceId,
    ) -> Result<AllocateAddressResult, RuntimeError>;

    fn stealth_transfer(
        &mut self,
        resource_address: ResourceAddressRef,
        statement: StealthTransferStatement,
        revealed_funds_bucket: Option<BucketId>,
    ) -> Result<Option<BucketId>, RuntimeError>;

    fn pay_fee(&mut self, pay_fee: PayFee) -> Result<(), RuntimeError>;

    fn track_template_loaded(
        &mut self,
        template_address: &TemplateAddress,
        bytes_loaded: usize,
    ) -> Result<(), RuntimeError>;

    /// Records the number of Wasmer metering points consumed by a single WASM template invocation
    /// so that runtime modules (notably the fee module) can convert them into a fee charge, and adds
    /// them to the transaction-wide total. Called once per `WasmProcess::invoke`, including across
    /// nested cross-template calls.
    fn record_wasm_execution(&mut self, points_consumed: u64) -> Result<(), RuntimeError>;

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
    fn set_runtime_pointer(&mut self, pointer: *mut Box<dyn RuntimeInterface>);
}

#[derive(Clone)]
pub struct Runtime {
    interface: NonNull<Box<dyn RuntimeInterface>>,
}

// SAFETY: The Runtime is strictly only used on a single thread. We implement Sync and Send manually to satify wasmer,
// which tries to account for multithreaded usage.
unsafe impl Sync for Runtime {}
// SAFETY: The Runtime is strictly only used on a single thread
unsafe impl Send for Runtime {}

impl Runtime {
    pub const fn from_mut(interface: &mut Box<dyn RuntimeInterface>) -> Self {
        Self {
            interface: NonNull::from_mut(interface),
        }
    }

    /// Creates a Runtime from a raw pointer. Returns None if the pointer is null.
    pub fn from_pointer(interface: *mut Box<dyn RuntimeInterface>) -> Option<Self> {
        Some(Self {
            interface: NonNull::new(interface)?,
        })
    }

    pub fn as_pointer(&self) -> *mut Box<dyn RuntimeInterface> {
        self.interface.as_ptr()
    }

    pub fn interface(&self) -> &dyn RuntimeInterface {
        // SAFETY: Caller promises that the interface is non-null and valid for the lifetime of Runtime.
        unsafe { self.interface.as_ref() }.as_ref()
    }

    pub fn interface_mut(&mut self) -> &mut dyn RuntimeInterface {
        // SAFETY: Caller promises that the interface is non-null and valid for the lifetime of Runtime.
        unsafe { self.interface.as_mut() }.as_mut()
    }
}

impl Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("interface", &"dyn RuntimeInterface")
            .finish()
    }
}
