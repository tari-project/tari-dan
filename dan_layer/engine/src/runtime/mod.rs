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
pub use error::{AssertError, RuntimeError, TransactionCommitError};

mod actions;
pub use actions::*;

mod module;
pub use module::{RuntimeModule, RuntimeModuleError};

mod fee_state;
mod tracker;

mod locking;
pub mod scope;
pub use locking::{LockError, LockState};
mod address_allocation;
mod state_store;
mod tracker_auth;
mod utils;
mod working_state;
mod workspace;

use std::{fmt::Debug, sync::Arc};

use tari_bor::decode_exact;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::Epoch;
use tari_engine_types::{
    commit_result::FinalizeResult,
    component::ComponentHeader,
    confidential::ConfidentialClaim,
    indexed_value::IndexedValue,
    lock::LockFlag,
    substate::SubstateValue,
};
use tari_template_lib::{
    args::{
        Arg,
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
        LogLevel,
        NonFungibleAction,
        ProofAction,
        ProofRef,
        ResourceAction,
        ResourceRef,
        VaultAction,
        WorkspaceAction,
    },
    invoke_args,
    models::{ComponentAddress, EntityId, Metadata, NonFungibleAddress, VaultRef},
};
pub use tracker::StateTracker;

use crate::runtime::{locking::LockedSubstate, scope::PushCallFrame};

pub trait RuntimeInterface: Send + Sync {
    fn next_entity_id(&self) -> Result<EntityId, RuntimeError>;
    fn emit_event(&self, topic: String, payload: Metadata) -> Result<(), RuntimeError>;

    fn emit_log(&self, level: LogLevel, message: String) -> Result<(), RuntimeError>;

    fn load_component(&self, address: &ComponentAddress) -> Result<ComponentHeader, RuntimeError>;

    fn lock_component(&self, address: &ComponentAddress, lock_flag: LockFlag) -> Result<LockedSubstate, RuntimeError>;

    fn get_substate(&self, lock: &LockedSubstate) -> Result<SubstateValue, RuntimeError>;
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

    fn set_last_instruction_output(&self, value: IndexedValue) -> Result<(), RuntimeError>;

    fn claim_burn(&self, claim: ConfidentialClaim) -> Result<(), RuntimeError>;

    fn claim_validator_fees(&self, epoch: Epoch, validator_public_key: PublicKey) -> Result<(), RuntimeError>;

    fn set_fee_checkpoint(&self) -> Result<(), RuntimeError>;
    fn reset_to_fee_checkpoint(&self) -> Result<(), RuntimeError>;
    fn finalize(&self) -> Result<FinalizeResult, RuntimeError>;
    fn validate_finalized(&self) -> Result<(), RuntimeError>;

    fn caller_context_invoke(
        &self,
        action: CallerContextAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError>;

    fn call_invoke(&self, action: CallAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError>;

    fn builtin_template_invoke(&self, action: BuiltinTemplateAction) -> Result<InvokeResult, RuntimeError>;

    fn check_component_access_rules(&self, method: &str, locked: &LockedSubstate) -> Result<(), RuntimeError>;

    fn validate_return_value(&self, value: &IndexedValue) -> Result<(), RuntimeError>;

    fn push_call_frame(&self, frame: PushCallFrame) -> Result<(), RuntimeError>;
    fn pop_call_frame(&self) -> Result<(), RuntimeError>;
}

#[derive(Clone)]
pub struct Runtime {
    interface: Arc<dyn RuntimeInterface>,
}

impl Runtime {
    pub(crate) fn resolve_args(&self, args: Vec<Arg>) -> Result<Vec<tari_bor::Value>, RuntimeError> {
        let mut resolved = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Arg::Workspace(key) => {
                    let value = self
                        .interface
                        .workspace_invoke(WorkspaceAction::Get, invoke_args![key].into())?;
                    resolved.push(value.into_value()?);
                },
                Arg::Literal(v) => resolved.push(decode_exact(&v)?),
            }
        }
        Ok(resolved)
    }
}

impl Runtime {
    pub fn new(interface: Arc<dyn RuntimeInterface>) -> Self {
        Self { interface }
    }

    pub fn interface(&self) -> &dyn RuntimeInterface {
        &*self.interface
    }
}

impl Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").field("engine", &"dyn RuntimeEngine").finish()
    }
}
