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

mod id_provider;
pub use id_provider::IdProvider;

mod r#impl;
pub use r#impl::RuntimeInterfaceImpl;

mod error;
pub use error::{RuntimeError, TransactionCommitError};

mod tracker;

#[cfg(test)]
mod tests;

use std::{fmt::Debug, sync::Arc};

use tari_engine_types::{commit_result::FinalizeResult, resource::Resource};
use tari_template_lib::{
    args::{
        Arg,
        BucketAction,
        BucketRef,
        ComponentAction,
        ComponentRef,
        InvokeResult,
        LogLevel,
        ResourceAction,
        ResourceRef,
        VaultAction,
        WorkspaceAction,
    },
    invoke_args,
    models::{ComponentAddress, ComponentHeader, ResourceAddress, VaultRef},
};
pub use tracker::{RuntimeState, StateTracker};

pub trait RuntimeInterface: Send + Sync {
    fn set_current_runtime_state(&self, state: RuntimeState);

    fn emit_log(&self, level: LogLevel, message: String);

    fn get_component(&self, address: &ComponentAddress) -> Result<ComponentHeader, RuntimeError>;
    fn get_resource(&self, address: &ResourceAddress) -> Result<Resource, RuntimeError>;

    fn component_invoke(
        &self,
        component_ref: ComponentRef,
        action: ComponentAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError>;

    fn resource_invoke(
        &self,
        resource_ref: ResourceRef,
        action: ResourceAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError>;

    fn vault_invoke(
        &self,
        vault_ref: VaultRef,
        action: VaultAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError>;

    fn bucket_invoke(
        &self,
        bucket_ref: BucketRef,
        action: BucketAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError>;

    fn workspace_invoke(&self, action: WorkspaceAction, args: Vec<Vec<u8>>) -> Result<InvokeResult, RuntimeError>;

    fn set_last_instruction_output(&self, value: Option<Vec<u8>>) -> Result<(), RuntimeError>;

    fn finalize(&self) -> Result<FinalizeResult, RuntimeError>;
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
                Arg::Variable(key) => {
                    let value = self
                        .interface
                        .workspace_invoke(WorkspaceAction::Take, invoke_args![key])?;
                    resolved.push(value.decode()?);
                },
                Arg::Literal(v) => resolved.push(v),
            }
        }
        Ok(resolved)
    }
}

impl Runtime {
    pub fn new(engine: Arc<dyn RuntimeInterface>) -> Self {
        Self { interface: engine }
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
