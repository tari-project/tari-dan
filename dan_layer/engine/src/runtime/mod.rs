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

mod consensus;
pub use consensus::ConsensusContext;

mod engine_args;
pub use crate::runtime::engine_args::EngineArgs;

mod error;
pub use error::{RuntimeError, TransactionCommitError};

mod functions;
pub use functions::FunctionIdent;

mod module;
pub use module::{RuntimeModule, RuntimeModuleError};

mod fee_state;
mod tracker;
mod working_state;

#[cfg(test)]
mod tests;

use std::{fmt::Debug, sync::Arc};

use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{commit_result::FinalizeResult, confidential::ConfidentialClaim, fees::FeeReceipt};
use tari_template_lib::{
    args::{
        Arg,
        BucketAction,
        BucketRef,
        CallerContextAction,
        ComponentAction,
        ComponentRef,
        ConsensusAction,
        GenerateRandomAction,
        InvokeResult,
        LogLevel,
        NonFungibleAction,
        ResourceAction,
        ResourceRef,
        VaultAction,
        WorkspaceAction,
    },
    invoke_args,
    models::{ComponentAddress, ComponentHeader, NonFungibleAddress, VaultRef},
};
pub use tracker::{RuntimeState, StateTracker};

pub trait RuntimeInterface: Send + Sync {
    fn set_current_runtime_state(&self, state: RuntimeState) -> Result<(), RuntimeError>;

    fn emit_log(&self, level: LogLevel, message: String) -> Result<(), RuntimeError>;

    fn get_component(&self, address: &ComponentAddress) -> Result<ComponentHeader, RuntimeError>;

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

    fn set_last_instruction_output(&self, value: Option<Vec<u8>>) -> Result<(), RuntimeError>;

    fn claim_burn(&self, claim: ConfidentialClaim) -> Result<(), RuntimeError>;

    fn create_free_test_coins(&self, amount: u64, private_key: RistrettoSecretKey) -> Result<(), RuntimeError>;
    fn fee_checkpoint(&self) -> Result<(), RuntimeError>;
    fn reset_to_fee_checkpoint(&self) -> Result<(), RuntimeError>;
    fn finalize(&self) -> Result<(FinalizeResult, FeeReceipt), RuntimeError>;

    fn caller_context_invoke(&self, action: CallerContextAction) -> Result<InvokeResult, RuntimeError>;
}

#[derive(Clone)]
pub struct Runtime {
    interface: Arc<dyn RuntimeInterface>,
}

impl Runtime {
    pub(crate) fn resolve_args(&self, args: Vec<Arg>) -> Result<Vec<Vec<u8>>, RuntimeError> {
        let mut resolved = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Arg::Workspace(key) => {
                    let value = self
                        .interface
                        .workspace_invoke(WorkspaceAction::Get, invoke_args![key].into())?;
                    resolved.push(value.decode()?);
                },
                Arg::Literal(v) => resolved.push(v),
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
