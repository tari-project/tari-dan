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

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tari_template_lib::{
    args::{CreateComponentArg, MintResourceArg},
    models::{
        Amount,
        BucketId,
        Component,
        ComponentAddress,
        ComponentInstance,
        ContractAddress,
        Metadata,
        PackageAddress,
        ResourceAddress,
        VaultId,
    },
    resource::ResourceType,
    Hash,
};

use crate::{
    models::{Bucket, Resource, Vault},
    runtime::{id_provider::IdProvider, logs::LogEntry, RuntimeError},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader},
};

#[derive(Debug, Clone)]
pub struct StateTracker {
    state_store: MemoryStateStore,
    working_state: Arc<RwLock<WorkingState>>,
    id_provider: IdProvider,
}

#[derive(Debug, Clone)]
pub struct RuntimeState {
    pub package_address: PackageAddress,
    pub contract_address: ContractAddress,
}

#[derive(Debug, Clone)]
struct WorkingState {
    logs: Vec<LogEntry>,
    buckets: HashMap<BucketId, Bucket>,
    new_resources: HashMap<ResourceAddress, Resource>,
    new_components: HashMap<ComponentAddress, ComponentInstance>,
    new_vaults: HashMap<VaultId, Vault>,
    runtime_state: Option<RuntimeState>,
    last_instruction_output: Option<Vec<u8>>,
    workspace: HashMap<Vec<u8>, Vec<u8>>,
}

impl StateTracker {
    pub fn new(state_store: MemoryStateStore, transaction_hash: Hash) -> Self {
        Self {
            state_store,
            working_state: Arc::new(RwLock::new(WorkingState {
                logs: Vec::new(),
                buckets: HashMap::new(),
                new_resources: HashMap::new(),
                new_components: HashMap::new(),
                new_vaults: HashMap::new(),
                runtime_state: None,
                last_instruction_output: None,
                workspace: HashMap::new(),
            })),
            id_provider: IdProvider::new(transaction_hash),
        }
    }

    pub fn add_log(&self, log: LogEntry) {
        self.write_with(|state| state.logs.push(log));
    }

    fn check_amount(&self, amount: &Amount) -> Result<(), RuntimeError> {
        if amount.is_negative() {
            return Err(RuntimeError::InvalidAmount {
                amount: *amount,
                reason: "Amount must be positive".to_string(),
            });
        }
        Ok(())
    }

    pub fn mint_resource(&self, mint_arg: MintResourceArg) -> Result<ResourceAddress, RuntimeError> {
        let resource_address = self.id_provider.new_resource_address();
        match mint_arg {
            MintResourceArg::Fungible { amount, metadata } => {
                self.check_amount(&amount)?;
                self.write_with(|state| {
                    let resource = Resource::fungible(resource_address, amount, metadata);
                    state.new_resources.insert(resource.address(), resource);
                });
            },
            MintResourceArg::NonFungible { token_ids, metadata } => {
                self.write_with(|state| {
                    let resource = Resource::non_fungible(resource_address, token_ids, metadata);
                    state.new_resources.insert(resource.address(), resource);
                });
            },
        }

        Ok(resource_address)
    }

    pub fn get_resource(&self, address: &ResourceAddress) -> Option<Resource> {
        // TODO: read from state?
        self.read_with(|state| state.new_resources.get(address).cloned())
    }

    pub fn new_bucket(&self, resource: Resource) -> BucketId {
        self.write_with(|state| {
            let bucket_id = self.id_provider.new_bucket_id();
            let bucket = Bucket::new(resource);
            state.buckets.insert(bucket_id, bucket);
            bucket_id
        })
    }

    pub fn take_bucket(&self, bucket_id: BucketId) -> Result<Bucket, RuntimeError> {
        self.write_with(|state| {
            state
                .buckets
                .remove(&bucket_id)
                .ok_or(RuntimeError::BucketNotFound { bucket_id })
        })
    }

    pub fn get_bucket(&self, bucket_id: BucketId) -> Result<Bucket, RuntimeError> {
        self.read_with(|state| {
            state
                .buckets
                .get(&bucket_id)
                .cloned()
                .ok_or(RuntimeError::BucketNotFound { bucket_id })
        })
    }

    pub fn with_bucket_mut<R, F: FnMut(&mut Bucket) -> R>(
        &self,
        bucket_id: BucketId,
        mut callback: F,
    ) -> Result<R, RuntimeError> {
        self.write_with(|state| {
            let bucket = state
                .buckets
                .get_mut(&bucket_id)
                .ok_or(RuntimeError::BucketNotFound { bucket_id })?;
            Ok(callback(bucket))
        })
    }

    pub fn new_vault(&self, resource_address: ResourceAddress, resource_type: ResourceType) -> VaultId {
        let vault_id = self.id_provider.new_vault_id();
        let resource = match resource_type {
            ResourceType::Fungible => Resource::fungible(resource_address, 0.into(), Metadata::new()),
            ResourceType::NonFungible => Resource::non_fungible(resource_address, vec![], Metadata::new()),
            ResourceType::Confidential => todo!("thaum resource"),
        };
        let vault = Vault::new(resource);

        self.write_with(|state| {
            state.new_vaults.insert(vault_id, vault);
        });

        vault_id
    }

    pub fn new_component(&self, new_component: CreateComponentArg) -> Result<ComponentAddress, RuntimeError> {
        let runtime_state = self.runtime_state()?;
        let component = Component {
            contract_address: runtime_state.contract_address,
            package_address: runtime_state.package_address,
            module_name: new_component.module_name,
            state: new_component.state,
        };
        let component_address = self.id_provider().new_component_address(&component);
        let component = ComponentInstance::new(component_address, component);
        self.write_with(|state| {
            state.new_components.insert(component_address, component);
        });
        Ok(component_address)
    }

    pub fn get_component(&self, component_address: &ComponentAddress) -> Result<ComponentInstance, RuntimeError> {
        let component = self.read_with(|state| state.new_components.get(component_address).cloned());
        match component {
            Some(component) => Ok(component),
            None => {
                let tx = self.state_store.read_access()?;
                let component = tx
                    .get_state(component_address)?
                    .ok_or(RuntimeError::ComponentNotFound {
                        address: *component_address,
                    })?;
                Ok(component)
            },
        }
    }

    pub fn set_component(&self, updated_component: ComponentInstance) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            // TODO: Load from state store?
            let component = state
                .new_components
                .get_mut(&updated_component.component_address)
                .ok_or(RuntimeError::ComponentNotFound {
                    address: updated_component.component_address,
                })?;
            *component = updated_component;
            Ok(())
        })
    }

    pub(crate) fn set_current_runtime_state(&self, state: RuntimeState) {
        self.write_with(|s| s.runtime_state = Some(state));
    }

    pub fn get_vault(&self, vault_id: &VaultId) -> Result<Vault, RuntimeError> {
        self.read_with(|state| {
            state
                .new_vaults
                .get(vault_id)
                .cloned()
                .ok_or(RuntimeError::VaultNotFound { vault_id: *vault_id })
        })
    }

    pub fn set_vault(&self, vault_id: &VaultId, vault: Vault) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            let vault_mut = state
                .new_vaults
                .get_mut(vault_id)
                .ok_or(RuntimeError::VaultNotFound { vault_id: *vault_id })?;
            *vault_mut = vault;
            Ok(())
        })
    }

    fn runtime_state(&self) -> Result<RuntimeState, RuntimeError> {
        self.read_with(|state| state.runtime_state.clone().ok_or(RuntimeError::IllegalRuntimeState))
    }

    pub fn set_last_instruction_output(&self, output: Option<Vec<u8>>) {
        self.write_with(|state| {
            state.last_instruction_output = output;
        });
    }

    pub fn take_last_instruction_output(&self) -> Option<Vec<u8>> {
        self.write_with(|state| state.last_instruction_output.take())
    }

    pub fn take_from_workspace(&self, key: &[u8]) -> Result<Vec<u8>, RuntimeError> {
        self.write_with(|state| {
            state.workspace.remove(key).ok_or(RuntimeError::ItemNotOnWorkspace {
                key: String::from_utf8_lossy(key).to_string(),
            })
        })
    }

    pub fn put_in_workspace(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            if state.workspace.insert(key.clone(), value).is_some() {
                return Err(RuntimeError::WorkspaceItemKeyExists {
                    key: String::from_utf8_lossy(&key).to_string(),
                });
            }
            Ok(())
        })
    }

    fn read_with<R, F: FnOnce(&WorkingState) -> R>(&self, f: F) -> R {
        f(&*self.working_state.read().unwrap())
    }

    fn write_with<R, F: FnOnce(&mut WorkingState) -> R>(&self, f: F) -> R {
        f(&mut *self.working_state.write().unwrap())
    }

    pub(crate) fn id_provider(&self) -> &IdProvider {
        &self.id_provider
    }
}
