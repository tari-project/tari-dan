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

use std::sync::{Arc, RwLock};

use tari_dan_engine::{
    runtime::{FinalizeResult, RuntimeError, RuntimeInterface, RuntimeInterfaceImpl, RuntimeState, StateTracker},
    state_store::memory::MemoryStateStore,
};
use tari_template_lib::{
    args::{
        BucketAction,
        BucketRef,
        CreateComponentArg,
        InvokeResult,
        LogLevel,
        ResourceAction,
        ResourceRef,
        VaultAction,
        WorkspaceAction,
    },
    models::{ComponentAddress, ComponentInstance, VaultRef},
    Hash,
};

#[derive(Debug, Clone)]
pub struct MockRuntimeInterface {
    state: MemoryStateStore,
    calls: Arc<RwLock<Vec<&'static str>>>,
    invoke_result: Arc<RwLock<Option<InvokeResult>>>,
    inner: RuntimeInterfaceImpl,
}

impl MockRuntimeInterface {
    pub fn new() -> Self {
        // TODO: We use a zero transaction hash for tests, however this isn't correct and won't always work.
        let tx_hash = Hash::default();
        let state = MemoryStateStore::default();
        let tracker = StateTracker::new(state.clone(), tx_hash);
        Self {
            state,
            calls: Arc::new(RwLock::new(vec![])),
            invoke_result: Arc::new(RwLock::new(None)),
            inner: RuntimeInterfaceImpl::new(tracker),
        }
    }

    pub fn state_store(&self) -> MemoryStateStore {
        self.state.clone()
    }

    pub fn get_calls(&self) -> Vec<&'static str> {
        self.calls.read().unwrap().clone()
    }

    pub fn clear_calls(&self) {
        self.calls.write().unwrap().clear();
    }

    fn add_call(&self, call: &'static str) {
        self.calls.write().unwrap().push(call);
    }
}

impl RuntimeInterface for MockRuntimeInterface {
    fn set_current_runtime_state(&self, state: RuntimeState) {
        self.add_call("set_current_runtime_state");
        self.inner.set_current_runtime_state(state);
    }

    fn emit_log(&self, level: LogLevel, message: String) {
        self.add_call("emit_log");
        let level = match level {
            LogLevel::Error => log::Level::Error,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Info => log::Level::Info,
            LogLevel::Debug => log::Level::Debug,
        };
        eprintln!("[{:?}] {}", level, message);
        log::log!(target: "tari::dan::engine::runtime", level, "{}", message);
    }

    fn create_component(&self, arg: CreateComponentArg) -> Result<ComponentAddress, RuntimeError> {
        self.add_call("create_component");
        self.inner.create_component(arg)
    }

    fn get_component(&self, component_address: &ComponentAddress) -> Result<ComponentInstance, RuntimeError> {
        self.add_call("get_component");
        self.inner.get_component(component_address)
    }

    fn set_component_state(&self, component_address: &ComponentAddress, state: Vec<u8>) -> Result<(), RuntimeError> {
        self.add_call("set_component_state");
        self.inner.set_component_state(component_address, state)
    }

    fn resource_invoke(
        &self,
        resource_ref: ResourceRef,
        action: ResourceAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        self.add_call("resource_invoke");
        match self.invoke_result.read().unwrap().as_ref() {
            Some(result) => Ok(result.clone()),
            None => self.inner.resource_invoke(resource_ref, action, args),
        }
    }

    fn vault_invoke(
        &self,
        vault_ref: VaultRef,
        action: VaultAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        self.add_call("vault_invoke");
        match self.invoke_result.read().unwrap().as_ref() {
            Some(result) => Ok(result.clone()),
            None => self.inner.vault_invoke(vault_ref, action, args),
        }
    }

    fn bucket_invoke(
        &self,
        bucket_ref: BucketRef,
        action: BucketAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        match self.invoke_result.read().unwrap().as_ref() {
            Some(result) => Ok(result.clone()),
            None => self.inner.bucket_invoke(bucket_ref, action, args),
        }
    }

    fn workspace_invoke(&self, action: WorkspaceAction, args: Vec<Vec<u8>>) -> Result<InvokeResult, RuntimeError> {
        self.add_call("workspace_invoke");
        match self.invoke_result.read().unwrap().as_ref() {
            Some(result) => Ok(result.clone()),
            None => self.inner.workspace_invoke(action, args),
        }
    }

    fn set_last_instruction_output(&self, value: Option<Vec<u8>>) -> Result<(), RuntimeError> {
        self.add_call("set_last_instruction_output");
        self.inner.set_last_instruction_output(value)
    }

    fn finalize(&self) -> Result<FinalizeResult, RuntimeError> {
        self.add_call("finalize");
        self.inner.finalize()
    }
}
