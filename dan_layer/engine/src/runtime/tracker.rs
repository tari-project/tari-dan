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
use tari_common_types::types::Commitment;
use tari_dan_common_types::optional::Optional;
use tari_engine_types::{
    address_list::{AddressList, AddressListItem},
    bucket::Bucket,
    logs::LogEntry,
    non_fungible::NonFungibleContainer,
    resource::Resource,
    resource_container::ResourceContainer,
    substate::{Substate, SubstateAddress, SubstateDiff},
    vault::Vault,
    TemplateAddress,
};
use tari_template_abi::TemplateDef;
use tari_template_lib::{
    args::MintArg,
    auth::AccessRules,
    models::{
        Address,
        AddressListId,
        AddressListItemAddress,
        Amount,
        BucketId,
        ComponentAddress,
        ComponentBody,
        ComponentHeader,
        LayerOneCommitmentAddress,
        Metadata,
        NonFungibleAddress,
        ResourceAddress,
        VaultId,
    },
    resource::ResourceType,
    Hash,
};
use tari_transaction::id_provider::IdProvider;

use crate::{
    runtime::{working_state::WorkingState, RuntimeError, TransactionCommitError},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader},
};

const LOG_TARGET: &str = "tari::engine::runtime::state_tracker";

#[derive(Debug, Clone)]
pub struct StateTracker {
    working_state: Arc<RwLock<WorkingState>>,
    id_provider: IdProvider,
    template_defs: HashMap<TemplateAddress, TemplateDef>,
}

impl StateTracker {}

#[derive(Debug, Clone)]
pub struct RuntimeState {
    pub template_address: TemplateAddress,
}

impl StateTracker {
    pub fn new(
        state_store: MemoryStateStore,
        id_provider: IdProvider,
        template_defs: HashMap<TemplateAddress, TemplateDef>,
    ) -> Self {
        Self {
            working_state: Arc::new(RwLock::new(WorkingState::new(state_store))),
            id_provider,
            template_defs,
        }
    }

    pub fn add_log(&self, log: LogEntry) {
        self.write_with(|state| state.logs.push(log));
    }

    pub fn take_logs(&self) -> Vec<LogEntry> {
        self.write_with(|state| mem::take(&mut state.logs))
    }

    pub fn get_template_def(&self) -> Result<&TemplateDef, RuntimeError> {
        let runtime_state = self.runtime_state()?;
        Ok(self
            .template_defs
            .get(&runtime_state.template_address)
            .expect("Template def not found for current template"))
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
        let resource = Resource::new(resource_type, metadata);
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
        let resource_container = self.write_with(|state| {
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
                        "Minting {} NFT token(s) on resource: {}",
                        tokens.len(),
                        resource_address
                    );
                    let mut token_ids = BTreeSet::new();
                    for (id, (data, mut_data)) in tokens {
                        let address = NonFungibleAddress::new(resource_address, id.clone());
                        if state.get_non_fungible(&address).optional()?.is_some() {
                            return Err(RuntimeError::DuplicateNonFungibleId {
                                token_id: address.id().clone(),
                            });
                        }
                        state
                            .new_non_fungibles
                            .insert(address, NonFungibleContainer::new(data, mut_data));
                        if !token_ids.insert(id.clone()) {
                            return Err(RuntimeError::DuplicateNonFungibleId { token_id: id });
                        }
                    }

                    ResourceContainer::non_fungible(resource_address, token_ids)
                },
            };

            // Increase the total supply, this also validates that the resource already exists.
            state.borrow_resource_mut(&resource_address, |resource| {
                resource.increase_total_supply(resource_container.amount())
            })?;

            Ok(resource_container)
        })?;

        let bucket = self.new_bucket(resource_container)?;
        Ok(bucket)
    }

    pub fn get_resource(&self, address: &ResourceAddress) -> Result<Resource, RuntimeError> {
        self.read_with(|state| state.get_resource(address))
    }

    pub fn get_non_fungible(&self, address: &NonFungibleAddress) -> Result<NonFungibleContainer, RuntimeError> {
        self.read_with(|state| state.get_non_fungible(address))
    }

    pub fn set_non_fungible_data(&self, address: &NonFungibleAddress, data: Vec<u8>) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            state.with_non_fungible_mut(address, move |nft| {
                let contents = nft.contents_mut().ok_or(RuntimeError::InvalidOpNonFungibleBurnt {
                    op: "UpdateNonFungibleData",
                    resource_address: *address.resource_address(),
                    nf_id: address.id().clone(),
                })?;
                contents.set_mutable_data(data);
                Ok(())
            })
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
        self.write_with(|state| state.take_bucket(bucket_id))
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

    pub fn with_bucket_mut<R, F: FnOnce(&mut Bucket) -> R>(
        &self,
        bucket_id: BucketId,
        callback: F,
    ) -> Result<R, RuntimeError> {
        self.write_with(|state| {
            let bucket = state
                .buckets
                .get_mut(&bucket_id)
                .ok_or(RuntimeError::BucketNotFound { bucket_id })?;
            Ok(callback(bucket))
        })
    }

    pub fn burn_bucket(&self, bucket_id: BucketId) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            let bucket = state.take_bucket(bucket_id)?;
            if bucket.amount().is_zero() {
                return Ok(());
            }
            let resource_address = *bucket.resource_address();
            let burnt_amount = bucket.amount();
            let mut resource = state.get_resource(&resource_address)?;
            for token_id in bucket.into_non_fungible_ids().into_iter().flatten() {
                let address = NonFungibleAddress::new(resource_address, token_id);
                let mut nft = state.get_non_fungible(&address)?;

                if nft.is_burnt() {
                    return Err(RuntimeError::InvalidOpNonFungibleBurnt {
                        op: "burn_bucket",
                        resource_address,
                        nf_id: address.id().clone(),
                    });
                }
                nft.burn();
                state.new_non_fungibles.insert(address, nft);
            }

            resource.decrease_total_supply(burnt_amount);
            state.new_resources.insert(resource_address, resource);

            Ok(())
        })
    }

    pub fn take_layer_one_commitment(
        &self,
        commitment_address: LayerOneCommitmentAddress,
    ) -> Result<Commitment, RuntimeError> {
        self.write_with(|state| {
            let commitment = state.get_layer_one_commitment(&commitment_address)?;

            state.claim_layer_one_commitment(&commitment_address)?;
            Ok(commitment)
        })
    }

    pub fn new_component(
        &self,
        module_name: String,
        state: Vec<u8>,
        access_rules: AccessRules,
    ) -> Result<ComponentAddress, RuntimeError> {
        let runtime_state = self.runtime_state()?;
        let component = ComponentBody { state };
        let component_address = self.id_provider().new_component_address()?;
        debug!(target: LOG_TARGET, "New component created: {}", component_address);
        let component = ComponentHeader {
            template_address: runtime_state.template_address,
            module_name,
            access_rules,
            state: component,
        };

        self.write_with(|state| {
            // New root component
            state.new_components.insert(component_address, component);
        });
        Ok(component_address)
    }

    pub fn get_component(&self, addr: &ComponentAddress) -> Result<ComponentHeader, RuntimeError> {
        self.read_with(|state| state.get_component(addr))
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
            ResourceType::Confidential => ResourceContainer::confidential(resource_address, vec![]),
        };
        let vault = Vault::new(vault_id, resource);

        self.write_with(|state| {
            state.new_vaults.insert(vault_id, vault);
        });

        Ok(vault_id)
    }

    pub fn borrow_vault<R, F: FnOnce(&Vault) -> R>(&self, vault_id: &VaultId, f: F) -> Result<R, RuntimeError> {
        self.read_with(|state| state.borrow_vault(vault_id, f))
    }

    pub fn borrow_vault_mut<R, F: FnOnce(&mut Vault) -> R>(&self, vault_id: &VaultId, f: F) -> Result<R, RuntimeError> {
        self.write_with(|state| state.borrow_vault_mut(vault_id, f))
    }

    pub fn new_address_list(&self) -> Result<AddressListId, RuntimeError> {
        let address_list_id = self.id_provider.new_address_list_id()?;
        debug!(target: LOG_TARGET, "New address list id: {}", address_list_id);
        let address_list = AddressList::new(address_list_id);

        self.write_with(|state| {
            state.new_address_lists.insert(address_list_id, address_list);
        });

        Ok(address_list_id)
    }

    pub fn address_list_push(
        &self,
        list_id: AddressListId,
        index: u64,
        referenced_address: Address,
    ) -> Result<(), RuntimeError> {
        let item_address = AddressListItemAddress::new(list_id, index);
        let item = AddressListItem::new(referenced_address);

        self.write_with(|state| {
            state.new_address_list_items.insert(item_address, item);
        });

        Ok(())
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

    pub fn finalize(&self) -> Result<SubstateDiff, TransactionCommitError> {
        let substates = self.write_with(|current_state| {
            // Finalise will always reset the state
            let state = mem::replace(current_state, WorkingState::new(current_state.state_store.clone()));
            state.validate_finalized()?;

            let tx = state
                .state_store
                .read_access()
                .map_err(TransactionCommitError::StateStoreTransactionError)?;
            let mut substate_diff = SubstateDiff::new();

            for (component_addr, substate) in state.new_components {
                let addr = SubstateAddress::Component(component_addr);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr.clone(), existing_state.version());
                        Substate::new(existing_state.version() + 1, substate)
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for (vault_id, substate) in state.new_vaults {
                let addr = SubstateAddress::Vault(vault_id);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr.clone(), existing_state.version());
                        Substate::new(existing_state.version() + 1, substate)
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for (resource_addr, substate) in state.new_resources {
                let addr = SubstateAddress::Resource(resource_addr);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr.clone(), existing_state.version());
                        Substate::new(existing_state.version() + 1, substate)
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for (address, substate) in state.new_non_fungibles {
                let addr = SubstateAddress::NonFungible(address);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(existing_state) => {
                        substate_diff.down(addr.clone(), existing_state.version());
                        Substate::new(existing_state.version() + 1, substate)
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for (list_id, substate) in state.new_address_lists {
                let addr = SubstateAddress::AddressList(list_id);
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(_) => {
                        // Addess list roots are immutable
                        return Err(TransactionCommitError::AddressListMutation { list_id });
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for (address, substate) in state.new_address_list_items {
                let addr = SubstateAddress::AddressListItem(address.clone());
                let new_substate = match tx.get_state::<_, Substate>(&addr).optional()? {
                    Some(_) => {
                        // Addess list items are immutable
                        return Err(TransactionCommitError::AddressListItemMutation {
                            list_id: *address.list_id(),
                            index: address.index(),
                        });
                    },
                    None => Substate::new(0, substate),
                };
                substate_diff.up(addr, new_substate);
            }

            for claimed in state.claimed_layer_one_commitments {
                substate_diff.down(SubstateAddress::LayerOneCommitment(claimed), 0);
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
