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
    runtime::{IdProvider, RuntimeError, RuntimeInterface, RuntimeState},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateWriter},
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
    models::{Component, ComponentAddress, ComponentInstance, VaultRef},
    Hash,
};

#[derive(Debug, Clone)]
pub struct MockRuntimeInterface {
    state: MemoryStateStore,
    calls: Arc<RwLock<Vec<&'static str>>>,
    id_provider: IdProvider,
    invoke_result: Arc<RwLock<Option<InvokeResult>>>,
    runtime_state: Arc<RwLock<Option<RuntimeState>>>,
}

impl MockRuntimeInterface {
    pub fn new() -> Self {
        Self {
            state: MemoryStateStore::default(),
            calls: Arc::new(RwLock::new(vec![])),
            id_provider: IdProvider::new(Hash::default()),
            invoke_result: Arc::new(RwLock::new(None)),
            runtime_state: Arc::new(RwLock::new(None)),
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

    // pub fn set_resource_invoke_result(&self, result: InvokeResult) {
    //     *self.invoke_result.write().unwrap() = Some(result);
    // }

    fn add_call(&self, call: &'static str) {
        self.calls.write().unwrap().push(call);
    }
}

impl RuntimeInterface for MockRuntimeInterface {
    fn set_current_runtime_state(&self, state: RuntimeState) {
        self.runtime_state.write().unwrap().replace(state);
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
        let new_component = Component {
            contract_address: Default::default(),
            package_address: Default::default(),
            module_name: arg.module_name,
            state: arg.state,
        };
        let component_address = self.id_provider.new_component_address(&new_component);

        let component = ComponentInstance::new(component_address, new_component);
        let mut tx = self.state.write_access().map_err(RuntimeError::StateDbError)?;
        tx.set_state(&component_address, component)?;
        tx.commit()?;

        Ok(component_address)
    }

    fn get_component(&self, component_address: &ComponentAddress) -> Result<ComponentInstance, RuntimeError> {
        self.add_call("get_component");
        let component = self
            .state
            .read_access()
            .map_err(RuntimeError::StateDbError)?
            .get_state(component_address)?
            .ok_or(RuntimeError::ComponentNotFound {
                address: *component_address,
            })?;
        Ok(component)
    }

    fn set_component_state(&self, component_address: &ComponentAddress, state: Vec<u8>) -> Result<(), RuntimeError> {
        self.add_call("set_component_state");
        let mut tx = self.state.write_access().map_err(RuntimeError::StateDbError)?;
        let mut component: ComponentInstance =
            tx.get_state(component_address)?
                .ok_or(RuntimeError::ComponentNotFound {
                    address: *component_address,
                })?;
        component.state = state;
        tx.set_state(&component_address, component)?;
        tx.commit()?;

        Ok(())
    }

    fn resource_invoke(
        &self,
        _resource_ref: ResourceRef,
        _action: ResourceAction,
        _args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        self.add_call("resource_invoke");
        Ok(self.invoke_result.read().unwrap().as_ref().unwrap().clone())
    }

    fn vault_invoke(
        &self,
        _resource_ref: VaultRef,
        _action: VaultAction,
        _args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        self.add_call("vault_invoke");
        Ok(self.invoke_result.read().unwrap().as_ref().unwrap().clone())
    }

    fn bucket_invoke(
        &self,
        _bucket_ref: BucketRef,
        _action: BucketAction,
        _args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        todo!()
    }

    fn workspace_invoke(&self, _action: WorkspaceAction, _args: Vec<Vec<u8>>) -> Result<InvokeResult, RuntimeError> {
        self.add_call("workspace_invoke");
        Ok(self.invoke_result.read().unwrap().as_ref().unwrap().clone())
    }

    fn set_last_instruction_output(&self, _value: Option<Vec<u8>>) -> Result<(), RuntimeError> {
        self.add_call("set_last_instruction_output");
        Ok(())
    }
}
