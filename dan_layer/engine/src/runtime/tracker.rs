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
    collections::{BTreeSet, HashMap},
    mem,
    sync::{Arc, RwLock},
};

use log::debug;
use tari_dan_common_types::optional::Optional;
use tari_engine_types::{
    bucket::Bucket,
    logs::LogEntry,
    resource::Resource,
    resource_container::ResourceContainer,
    substate::{Substate, SubstateAddress, SubstateDiff},
    vault::Vault,
};
use tari_template_lib::{
    args::MintArg,
    models::{
        Amount,
        BucketId,
        ComponentAddress,
        ComponentBody,
        ComponentHeader,
        Metadata,
        NftToken,
        NftTokenId,
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
    new_resources: HashMap<ResourceAddress, Resource>,
    new_components: HashMap<ComponentAddress, ComponentHeader>,
    new_vaults: HashMap<VaultId, Vault>,
    new_non_fungibles: HashMap<(ResourceAddress, NftTokenId), NftToken>,

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
                new_non_fungibles: HashMap::new(),
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

    pub fn new_resource(
        &self,
        resource_type: ResourceType,
        metadata: Metadata,
    ) -> Result<ResourceAddress, RuntimeError> {
        let resource_address = self.id_provider.new_resource_address()?;
        let resource = Resource::new(resource_type, resource_address, metadata);
        self.write_with(|state| {
            state.new_resources.insert(resource_address, resource);
        });
        Ok(resource_address)
    }

    pub fn mint_resource(
        &self,
        resource_address: ResourceAddress,
        mint_arg: MintArg,
    ) -> Result<BucketId, RuntimeError> {
        let resource_container = match mint_arg {
            MintArg::Fungible { amount } => {
                self.check_amount(amount)?;
                debug!(
                    target: LOG_TARGET,
                    "Minting {} fungible tokens on resource: {}", amount, resource_address
                );

                ResourceContainer::fungible(resource_address, amount)
            },
            MintArg::NonFungible { tokens } => {
                debug!(
                    target: LOG_TARGET,
                    "Minting {} NFT tokens on resource: {}",
                    tokens.len(),
                    resource_address
                );
                self.write_with(|state| {
                    let mut token_ids = BTreeSet::new();
                    for (id, token) in tokens {
                        if state.new_non_fungibles.insert((resource_address, id), token).is_some() {
                            return Err(RuntimeError::DuplicateNftTokenId { token_id: id });
                        }
                        token_ids.insert(id);
                    }

                    Ok(ResourceContainer::non_fungible(resource_address, token_ids))
                })?
            },
        };

        // Increase the total supply, this also validates that the resource already exists.
        self.borrow_resource_mut(&resource_address, |resource| {
            resource.increase_total_supply(resource_container.amount());
        })?;

        let bucket = self.new_bucket(resource_container)?;
        Ok(bucket)
    }

    pub fn get_resource(&self, address: &ResourceAddress) -> Result<Resource, RuntimeError> {
        self.read_with(|state| match state.new_resources.get(address).cloned() {
            Some(resource) => Ok(resource),
            None => {
                let tx = self.state_store.read_access()?;
                let resource = tx
                    .get_state::<_, Substate>(&SubstateAddress::Resource(*address))
                    .optional()?
                    .ok_or(RuntimeError::ResourceNotFound {
                        resource_address: *address,
                    })?;
                Ok(resource
                    .into_substate()
                    .into_resource()
                    .expect("Substate was not a resource type at resource address"))
            },
        })
    }

    pub fn new_bucket(&self, resource: ResourceContainer) -> Result<BucketId, RuntimeError> {
        self.write_with(|state| {
            let bucket_id = self.id_provider.new_bucket_id();
            debug!(target: LOG_TARGET, "New bucket: {}", bucket_id);
            let bucket = Bucket::new(resource);
            state.buckets.insert(bucket_id, bucket);
            Ok(bucket_id)
        })
    }

    pub fn new_empty_bucket(
        &self,
        resource_address: ResourceAddress,
        resource_type: ResourceType,
    ) -> Result<BucketId, RuntimeError> {
        self.write_with(|state| {
            let bucket_id = self.id_provider.new_bucket_id();
            debug!(
                target: LOG_TARGET,
                "New bucket {} for resource {} {:?}", bucket_id, resource_address, resource_type
            );
            let new_state = match resource_type {
                ResourceType::Fungible => ResourceContainer::fungible(resource_address, Amount::zero()),
                ResourceType::NonFungible => ResourceContainer::non_fungible(resource_address, BTreeSet::new()),
                ResourceType::Confidential => todo!("new_empty_bucket"),
            };
            let bucket = Bucket::new(new_state);
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
        let component = ComponentBody { state };
        let component_address = self.id_provider().new_component_address()?;
        debug!(target: LOG_TARGET, "New component created: {}", component_address);
        let component = ComponentHeader {
            component_address,
            template_address: runtime_state.template_address,
            module_name,
            state: component,
        };

        self.write_with(|state| {
            // New root component
            state.new_components.insert(component_address, component);
        });
        Ok(component_address)
    }

    pub fn get_component(&self, addr: &ComponentAddress) -> Result<ComponentHeader, RuntimeError> {
        let component = self.read_with(|state| state.new_components.get(addr).cloned());
        match component {
            Some(component) => Ok(component),
            None => {
                let tx = self.state_store.read_access()?;
                let value = tx
                    .get_state::<_, Substate>(&SubstateAddress::Component(*addr))
                    .optional()?
                    .ok_or(RuntimeError::ComponentNotFound { address: *addr })?;
                Ok(value
                    .into_substate()
                    .into_component()
                    .expect("Substate was not a component type at component address"))
            },
        }
    }

    /// Set the component. This may be called many times during execution but always results in exactly one UP substate
    /// with an incremented version.
    pub fn set_component(
        &self,
        component_address: ComponentAddress,
        component: ComponentHeader,
    ) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            state.new_components.insert(component_address, component);
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
            ResourceType::Fungible => ResourceContainer::fungible(resource_address, 0.into()),
            ResourceType::NonFungible => ResourceContainer::non_fungible(resource_address, BTreeSet::new()),
            ResourceType::Confidential => todo!("new_vault: thaum resource"),
        };
        let vault = Vault::new(vault_id, resource);

        self.write_with(|state| {
            state.new_vaults.insert(vault_id, vault);
        });

        Ok(vault_id)
    }

    pub fn borrow_resource_mut<R, F: FnOnce(&mut Resource) -> R>(
        &self,
        resource_address: &ResourceAddress,
        f: F,
    ) -> Result<R, RuntimeError> {
        self.write_with(|state| {
            let vault_mut = state.new_resources.get_mut(resource_address);
            match vault_mut {
                Some(resource_mut) => Ok(f(resource_mut)),
                None => {
                    let substate = self
                        .state_store
                        .read_access()
                        .unwrap()
                        .get_state::<_, Substate>(&SubstateAddress::Resource(*resource_address))
                        .optional()?
                        .ok_or(RuntimeError::ResourceNotFound {
                            resource_address: *resource_address,
                        })?;

                    let mut resource = substate
                        .into_substate()
                        .into_resource()
                        .expect("Substate was not a vault type at vault address");
                    let ret = f(&mut resource);
                    state.new_resources.insert(*resource_address, resource);
                    Ok(ret)
                },
            }
        })
    }

    pub fn borrow_vault_mut<R, F: FnOnce(&mut Vault) -> R>(&self, vault_id: &VaultId, f: F) -> Result<R, RuntimeError> {
        self.write_with(|state| {
            let vault_mut = state.new_vaults.get_mut(vault_id);
            match vault_mut {
                Some(vault_mut) => Ok(f(vault_mut)),
                None => {
                    let substate = self
                        .state_store
                        .read_access()
                        .unwrap()
                        .get_state::<_, Substate>(&SubstateAddress::Vault(*vault_id))
                        .optional()?
                        .ok_or(RuntimeError::VaultNotFound { vault_id: *vault_id })?;

                    let mut vault = substate
                        .into_substate()
                        .into_vault()
                        .expect("Substate was not a vault type at vault address");
                    let ret = f(&mut vault);
                    state.new_vaults.insert(*vault_id, vault);
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
                        substate_diff.down(addr, existing_state.version());
                        Substate::new(addr, existing_state.version() + 1, substate)
                    },
                    None => Substate::new(addr, 0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for (vault_id, substate) in state.new_vaults.drain() {
                let addr = SubstateAddress::Vault(vault_id);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr, existing_state.version());
                        Substate::new(addr, existing_state.version() + 1, substate)
                    },
                    None => Substate::new(addr, 0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for (resource_addr, substate) in state.new_resources.drain() {
                let addr = SubstateAddress::Resource(resource_addr);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr, existing_state.version());
                        Substate::new(addr, existing_state.version() + 1, substate)
                    },
                    None => Substate::new(addr, 0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for ((resource_addr, id), substate) in state.new_non_fungibles.drain() {
                let addr = SubstateAddress::NonFungible(resource_addr, id);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr, existing_state.version());
                        Substate::new(addr, existing_state.version() + 1, substate)
                    },
                    None => Substate::new(addr, 0, substate),
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
