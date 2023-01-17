//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
//

use std::sync::{Arc, RwLock};

use tari_crypto::ristretto::RistrettoComSig;
use tari_dan_engine::{
    runtime::{IdProvider, RuntimeError, RuntimeInterface, RuntimeInterfaceImpl, RuntimeState, StateTracker},
    state_store::memory::MemoryStateStore,
};
use tari_engine_types::{commit_result::FinalizeResult, resource::Resource, substate::SubstateAddress};
use tari_template_lib::{
    args::{
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
    models::{ComponentAddress, ComponentHeader, ResourceAddress, VaultRef},
    Hash,
};

#[derive(Debug, Clone)]
pub struct MockRuntimeInterface {
    state: MemoryStateStore,
    calls: Arc<RwLock<Vec<&'static str>>>,
    invoke_result: Arc<RwLock<Option<InvokeResult>>>,
    inner: RuntimeInterfaceImpl,
}

impl Default for MockRuntimeInterface {
    fn default() -> Self {
        // TODO: We use a zero transaction hash for tests, however this isn't correct and won't always work.
        let tx_hash = Hash::default();
        let state = MemoryStateStore::default();
        let tracker = StateTracker::new(state.clone(), IdProvider::new(tx_hash, 100));
        Self {
            state,
            calls: Arc::new(RwLock::new(vec![])),
            invoke_result: Arc::new(RwLock::new(None)),
            inner: RuntimeInterfaceImpl::new(tracker),
        }
    }
}

impl MockRuntimeInterface {
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

    fn get_component(&self, address: &ComponentAddress) -> Result<ComponentHeader, RuntimeError> {
        self.add_call("get_component");
        self.inner.get_component(address)
    }

    fn get_resource(&self, address: &ResourceAddress) -> Result<Resource, RuntimeError> {
        self.add_call("get_resource()");
        self.inner.get_resource(address)
    }

    fn component_invoke(
        &self,
        component_ref: ComponentRef,
        action: ComponentAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        self.add_call("component_invoke");
        self.inner.component_invoke(component_ref, action, args)
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

    fn claim_burn(
        &self,
        substate_address: SubstateAddress,
        commitment: tari_common_types::types::Commitment,
        range_proof: tari_common_types::types::BulletRangeProof,
        owner_sig: RistrettoComSig,
    ) -> Result<(), RuntimeError> {
        todo!()
    }

    fn finalize(&self) -> Result<FinalizeResult, RuntimeError> {
        self.add_call("finalize");
        self.inner.finalize()
    }
}
