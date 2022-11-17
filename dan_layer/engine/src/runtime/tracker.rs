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
    mem,
    sync::{Arc, RwLock},
};

use log::debug;
use tari_dan_common_types::optional::Optional;
use tari_engine_types::{
    bucket::Bucket,
    logs::LogEntry,
    resource::Resource,
    substate::{Substate, SubstateAddress, SubstateDiff, SubstateValue},
    vault::Vault,
};
use tari_template_lib::{
    args::MintResourceArg,
    models::{
        Amount,
        BucketId,
        Component,
        ComponentAddress,
        ComponentInstance,
        Metadata,
        ResourceAddress,
        TemplateAddress,
        VaultId,
    },
    resource::ResourceType,
    Hash,
};

use crate::{
    runtime::{id_provider::IdProvider, RuntimeError, TransactionCommitError},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader},
};

const LOG_TARGET: &str = "tari::engine::runtime::state_tracker";

#[derive(Debug, Clone)]
pub struct StateTracker {
    state_store: MemoryStateStore,
    working_state: Arc<RwLock<WorkingState>>,
    id_provider: IdProvider,
}

#[derive(Debug, Clone)]
pub struct RuntimeState {
    pub template_address: TemplateAddress,
}

#[derive(Debug, Clone)]
struct WorkingState {
    logs: Vec<LogEntry>,
    buckets: HashMap<BucketId, Bucket>,
    // These could be "new_substates"
    new_resources: HashMap<ResourceAddress, SubstateValue>,
    new_components: HashMap<ComponentAddress, SubstateValue>,
    new_vaults: HashMap<VaultId, SubstateValue>,

    runtime_state: Option<RuntimeState>,
    last_instruction_output: Option<Vec<u8>>,
    workspace: HashMap<Vec<u8>, Vec<u8>>,
}

impl StateTracker {
    pub fn new(state_store: MemoryStateStore, id_provider: IdProvider) -> Self {
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
            id_provider,
        }
    }

    pub fn add_log(&self, log: LogEntry) {
        self.write_with(|state| state.logs.push(log));
    }

    pub fn take_logs(&self) -> Vec<LogEntry> {
        self.write_with(|state| mem::take(&mut state.logs))
    }

    fn check_amount(&self, amount: Amount) -> Result<(), RuntimeError> {
        if amount.is_negative() {
            return Err(RuntimeError::InvalidAmount {
                amount,
                reason: "Amount must be positive".to_string(),
            });
        }
        Ok(())
    }

    pub fn mint_resource(&self, mint_arg: MintResourceArg) -> Result<ResourceAddress, RuntimeError> {
        let resource_address = self.id_provider.new_resource_address()?;
        debug!(target: LOG_TARGET, "New resource minted: {}", resource_address);
        dbg!(resource_address.to_string());
        match mint_arg {
            MintResourceArg::Fungible { amount, metadata } => {
                self.check_amount(amount)?;
                self.write_with(|state| {
                    let resource = Resource::fungible(resource_address, amount, metadata);
                    state.new_resources.insert(resource.address(), resource.into());
                });
            },
            MintResourceArg::NonFungible { token_ids, metadata } => {
                self.write_with(|state| {
                    let resource = Resource::non_fungible(resource_address, token_ids, metadata);
                    state.new_resources.insert(resource.address(), resource.into());
                });
            },
        }

        Ok(resource_address)
    }

    pub fn get_resource(&self, address: &ResourceAddress) -> Result<Resource, RuntimeError> {
        self.read_with(|state| {
            match state.new_resources.get(address).cloned().map(|substate| {
                substate
                    .into_resource()
                    .expect("new_resources contains non-resource substate")
            }) {
                Some(resource) => Ok(resource),
                None => {
                    let tx = self.state_store.read_access()?;
                    let resource = tx.get_state(&SubstateAddress::Resource(*address)).optional()?.ok_or(
                        RuntimeError::ResourceNotFound {
                            resource_address: *address,
                        },
                    )?;
                    Ok(resource)
                },
            }
        })
    }

    pub fn new_bucket(&self, resource: Resource) -> Result<BucketId, RuntimeError> {
        self.write_with(|state| {
            let bucket_id = self.id_provider.new_bucket_id();
            debug!(target: LOG_TARGET, "New bucket: {}", bucket_id);
            let bucket = Bucket::new(resource);
            state.buckets.insert(bucket_id, bucket);
            Ok(bucket_id)
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

    pub fn list_buckets(&self) -> Vec<BucketId> {
        self.read_with(|state| state.buckets.keys().copied().collect())
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

    pub fn new_component(&self, module_name: String, state: Vec<u8>) -> Result<ComponentAddress, RuntimeError> {
        let runtime_state = self.runtime_state()?;
        let component = Component {
            template_address: runtime_state.template_address,
            module_name,
            state,
        };
        let component_address = self.id_provider().new_component_address()?;
        debug!(target: LOG_TARGET, "New component created: {}", component_address);
        let component = ComponentInstance::new(component_address, component);
        self.write_with(|state| {
            // New root component
            state.new_components.insert(component_address, component.into());
        });
        Ok(component_address)
    }

    pub fn get_substate(&self, substate_address: &SubstateAddress) -> Result<SubstateValue, RuntimeError> {
        let substate = self.read_with(|state| match substate_address {
            SubstateAddress::Component(addr) => state.new_components.get(addr).cloned(),
            SubstateAddress::Resource(addr) => state.new_resources.get(addr).cloned(),
            SubstateAddress::Vault(addr) => state.new_vaults.get(addr).cloned(),
        });
        match substate {
            Some(substate) => Ok(substate),
            None => {
                let tx = self.state_store.read_access()?;
                let value = tx
                    .get_state(substate_address)
                    .optional()?
                    .ok_or(RuntimeError::SubstateNotFound {
                        address: *substate_address,
                    })?;
                Ok(value)
            },
        }
    }

    /// Set the substate. This may be called many times during execution but always results in exactly one UP substate
    /// with an incremented version.
    pub fn set_substate(&self, value: SubstateValue) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            match value {
                SubstateValue::Component(component) => {
                    state.new_components.insert(component.address(), component.into());
                },
                SubstateValue::Resource(resource) => {
                    state.new_resources.insert(resource.address(), resource.into());
                },
                SubstateValue::Vault(vault) => {
                    state.new_vaults.insert(vault.id(), vault.into());
                },
            }
            Ok(())
        })
    }

    pub(crate) fn set_current_runtime_state(&self, state: RuntimeState) {
        self.write_with(|s| s.runtime_state = Some(state));
    }

    pub fn new_vault(
        &self,
        resource_address: ResourceAddress,
        resource_type: ResourceType,
    ) -> Result<VaultId, RuntimeError> {
        let vault_id = self.id_provider.new_vault_id()?;
        debug!(target: LOG_TARGET, "New vault id: {}", vault_id);
        let resource = match resource_type {
            ResourceType::Fungible => Resource::fungible(resource_address, 0.into(), Metadata::new()),
            ResourceType::NonFungible => Resource::non_fungible(resource_address, vec![], Metadata::new()),
            ResourceType::Confidential => todo!("thaum resource"),
        };
        let vault = Vault::new(vault_id, resource);

        self.write_with(|state| {
            state.new_vaults.insert(vault_id, vault.into());
        });

        Ok(vault_id)
    }

    pub fn borrow_vault_mut<R, F: FnOnce(&mut Vault) -> R>(&self, vault_id: &VaultId, f: F) -> Result<R, RuntimeError> {
        self.write_with(|state| {
            let vault_mut = state.new_vaults.get_mut(vault_id);
            match vault_mut {
                Some(SubstateValue::Vault(vault_mut)) => Ok(f(vault_mut)),
                Some(_) => unreachable!(),
                None => {
                    // TODO: This is not correct
                    let substate: Substate = self
                        .state_store
                        .read_access()
                        .unwrap()
                        .get_state(&SubstateAddress::Vault(*vault_id))
                        .optional()?
                        .ok_or(RuntimeError::VaultNotFound { vault_id: *vault_id })?;

                    let mut vault = substate
                        .into_substate()
                        .into_vault()
                        .expect("Vault key does not point to vault substate");

                    let ret = f(&mut vault);
                    state.new_vaults.insert(*vault_id, vault.into());
                    Ok(ret)
                },
            }
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

    fn validate_finalized(&self) -> Result<(), TransactionCommitError> {
        self.read_with(|state| {
            if !state.buckets.is_empty() {
                return Err(TransactionCommitError::DanglingBuckets {
                    count: state.buckets.len(),
                });
            }

            if !state.workspace.is_empty() {
                return Err(TransactionCommitError::WorkspaceNotEmpty {
                    count: state.workspace.len(),
                });
            }

            Ok(())
        })
    }

    pub fn finalize(&self) -> Result<SubstateDiff, TransactionCommitError> {
        self.validate_finalized()?;
        let tx = self
            .state_store
            .read_access()
            .map_err(TransactionCommitError::StateStoreTransactionError)?;

        let substates = self.write_with(|state| {
            let mut substate_diff = SubstateDiff::new();

            for (component_addr, substate) in state.new_components.drain() {
                let addr = SubstateAddress::Component(component_addr);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr);
                        Substate::new(existing_state.version() + 1, substate)
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            // Vaults are held within a component and contain a resource, so I dont think they are a substate in and of
            // themselves
            for (vault_id, substate) in state.new_vaults.drain() {
                let addr = SubstateAddress::Vault(vault_id);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr);
                        Substate::new(existing_state.version() + 1, substate)
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for (resource_addr, substate) in state.new_resources.drain() {
                let addr = SubstateAddress::Resource(resource_addr);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr);
                        Substate::new(existing_state.version() + 1, substate)
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            Result::<_, TransactionCommitError>::Ok(substate_diff)
        })?;

        Ok(substates)
    }

    fn read_with<R, F: FnOnce(&WorkingState) -> R>(&self, f: F) -> R {
        f(&self.working_state.read().unwrap())
    }

    fn write_with<R, F: FnOnce(&mut WorkingState) -> R>(&self, f: F) -> R {
        f(&mut self.working_state.write().unwrap())
    }

    pub fn transaction_hash(&self) -> Hash {
        self.id_provider.transaction_hash()
    }

    pub(crate) fn id_provider(&self) -> &IdProvider {
        &self.id_provider
    }
}
