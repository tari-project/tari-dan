//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{optional::Optional, Epoch};
use tari_engine_types::{
    bucket::Bucket,
    component::ComponentHeader,
    confidential::UnclaimedConfidentialOutput,
    events::Event,
    fee_claim::{FeeClaim, FeeClaimAddress},
    logs::LogEntry,
    non_fungible::NonFungibleContainer,
    non_fungible_index::NonFungibleIndex,
    resource::Resource,
    substate::{Substate, SubstateAddress},
    vault::Vault,
    virtual_substate::{VirtualSubstate, VirtualSubstateAddress},
};
use tari_template_lib::models::{
    BucketId,
    ComponentAddress,
    NonFungibleAddress,
    NonFungibleIndexAddress,
    ResourceAddress,
    UnclaimedConfidentialOutputAddress,
    VaultId,
};

use super::workspace::Workspace;
use crate::{
    runtime::{RuntimeError, RuntimeState, TransactionCommitError, VirtualSubstates},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader},
};

#[derive(Debug, Clone)]
pub(super) struct WorkingState {
    pub events: Vec<Event>,
    pub logs: Vec<LogEntry>,
    pub buckets: HashMap<BucketId, Bucket>,
    // These could be "new_substates"
    pub new_resources: HashMap<ResourceAddress, Resource>,
    pub new_components: HashMap<ComponentAddress, ComponentHeader>,
    pub new_vaults: HashMap<VaultId, Vault>,
    pub new_non_fungibles: HashMap<NonFungibleAddress, NonFungibleContainer>,
    pub new_non_fungible_indexes: HashMap<NonFungibleIndexAddress, NonFungibleIndex>,
    pub claimed_confidential_outputs: Vec<UnclaimedConfidentialOutputAddress>,
    pub new_fee_claims: HashMap<FeeClaimAddress, FeeClaim>,
    pub virtual_substates: VirtualSubstates,

    pub runtime_state: Option<RuntimeState>,
    pub last_instruction_output: Option<Vec<u8>>,
    pub workspace: Workspace,
    pub state_store: MemoryStateStore,
}

impl WorkingState {
    pub fn new(state_store: MemoryStateStore, virtual_substates: VirtualSubstates) -> Self {
        Self {
            events: Vec::new(),
            logs: Vec::new(),
            buckets: HashMap::new(),
            new_resources: HashMap::new(),
            new_components: HashMap::new(),
            new_vaults: HashMap::new(),
            new_non_fungibles: HashMap::new(),
            claimed_confidential_outputs: Vec::new(),
            new_non_fungible_indexes: HashMap::new(),
            runtime_state: None,
            last_instruction_output: None,
            workspace: Workspace::default(),
            state_store,
            virtual_substates,
            new_fee_claims: HashMap::default(),
        }
    }

    pub fn get_resource(&self, address: &ResourceAddress) -> Result<Resource, RuntimeError> {
        match self.new_resources.get(address).cloned() {
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
                    .into_substate_value()
                    .into_resource()
                    .expect("Substate was not a resource type at resource address"))
            },
        }
    }

    pub fn get_non_fungible(&self, address: &NonFungibleAddress) -> Result<NonFungibleContainer, RuntimeError> {
        match self.new_non_fungibles.get(address).cloned() {
            Some(nft) => Ok(nft),
            None => {
                let tx = self.state_store.read_access()?;
                let nft = tx
                    .get_state::<_, Substate>(&SubstateAddress::NonFungible(address.clone()))
                    .optional()?
                    .ok_or_else(|| RuntimeError::NonFungibleNotFound {
                        resource_address: *address.resource_address(),
                        nft_id: address.id().clone(),
                    })?;
                Ok(nft
                    .into_substate_value()
                    .into_non_fungible()
                    .expect("Substate was not a non-fungible type at non-fungible address"))
            },
        }
    }

    pub fn get_component(&self, addr: &ComponentAddress) -> Result<ComponentHeader, RuntimeError> {
        let component = self.new_components.get(addr).cloned();
        match component {
            Some(component) => Ok(component),
            None => {
                let tx = self.state_store.read_access()?;
                let value = tx
                    .get_state::<_, Substate>(&SubstateAddress::Component(*addr))
                    .optional()?
                    .ok_or(RuntimeError::ComponentNotFound { address: *addr })?;
                Ok(value
                    .into_substate_value()
                    .into_component()
                    .expect("Substate was not a component type at component address"))
            },
        }
    }

    pub fn get_unclaimed_confidential_commitment(
        &self,
        addr: &UnclaimedConfidentialOutputAddress,
    ) -> Result<UnclaimedConfidentialOutput, RuntimeError> {
        let tx = self.state_store.read_access()?;
        let value = tx
            .get_state::<_, Substate>(&SubstateAddress::UnclaimedConfidentialOutput(*addr))
            .optional()?
            .ok_or(RuntimeError::LayerOneCommitmentNotFound { address: *addr })?;
        Ok(value
            .into_substate_value()
            .into_unclaimed_confidential_output()
            .expect("Substate was not an unclaimed commitment at unclaimed commitment address"))
    }

    pub fn claim_confidential_output(&mut self, addr: &UnclaimedConfidentialOutputAddress) -> Result<(), RuntimeError> {
        if self.claimed_confidential_outputs.contains(addr) {
            return Err(RuntimeError::ConfidentialOutputAlreadyClaimed { address: *addr });
        }
        self.claimed_confidential_outputs.push(*addr);
        Ok(())
    }

    pub fn with_non_fungible_mut<R, F: FnOnce(&mut NonFungibleContainer) -> Result<R, RuntimeError>>(
        &mut self,
        address: &NonFungibleAddress,
        callback: F,
    ) -> Result<R, RuntimeError> {
        let nft_mut = self.new_non_fungibles.get_mut(address);
        match nft_mut {
            Some(nft_mut) => Ok(callback(nft_mut)?),
            None => {
                let substate = self
                    .state_store
                    .read_access()
                    .unwrap()
                    .get_state::<_, Substate>(&SubstateAddress::NonFungible(address.clone()))
                    .optional()?
                    .ok_or_else(|| RuntimeError::NonFungibleNotFound {
                        resource_address: *address.resource_address(),
                        nft_id: address.id().clone(),
                    })?;

                let mut nft = substate.into_substate_value().into_non_fungible().unwrap_or_else(|| {
                    panic!(
                        "Substate was not a NonFungible type at ({}, {})",
                        address.resource_address(),
                        address.id()
                    )
                });
                let ret = callback(&mut nft)?;
                self.new_non_fungibles.insert(address.clone(), nft);
                Ok(ret)
            },
        }
    }

    pub fn borrow_vault<R, F: FnOnce(&Vault) -> R>(&self, vault_id: VaultId, f: F) -> Result<R, RuntimeError> {
        match self.new_vaults.get(&vault_id) {
            Some(vault) => Ok(f(vault)),
            None => {
                let substate = self
                    .state_store
                    .read_access()?
                    .get_state::<_, Substate>(&SubstateAddress::Vault(vault_id))
                    .optional()?
                    .ok_or(RuntimeError::VaultNotFound { vault_id })?;

                let vault = substate
                    .into_substate_value()
                    .into_vault()
                    .expect("Substate was not a vault type at vault address");

                Ok(f(&vault))
            },
        }
    }

    pub fn borrow_vault_mut<R, F: FnOnce(&mut Vault) -> R>(
        &mut self,
        vault_id: VaultId,
        f: F,
    ) -> Result<R, RuntimeError> {
        let vault_mut = self.new_vaults.get_mut(&vault_id);
        match vault_mut {
            Some(vault_mut) => Ok(f(vault_mut)),
            None => {
                let substate = self
                    .state_store
                    .read_access()
                    .unwrap()
                    .get_state::<_, Substate>(&SubstateAddress::Vault(vault_id))
                    .optional()?
                    .ok_or(RuntimeError::VaultNotFound { vault_id })?;

                let mut vault = substate
                    .into_substate_value()
                    .into_vault()
                    .expect("Substate was not a vault type at vault address");
                let ret = f(&mut vault);
                self.new_vaults.insert(vault_id, vault);
                Ok(ret)
            },
        }
    }

    pub fn borrow_resource_mut<R, F: FnOnce(&mut Resource) -> R>(
        &mut self,
        resource_address: &ResourceAddress,
        f: F,
    ) -> Result<R, RuntimeError> {
        let resource_mut = self.new_resources.get_mut(resource_address);
        match resource_mut {
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
                    .into_substate_value()
                    .into_resource()
                    .expect("Substate was not a resource type at resource address");
                let ret = f(&mut resource);
                self.new_resources.insert(*resource_address, resource);
                Ok(ret)
            },
        }
    }

    pub fn take_bucket(&mut self, bucket_id: BucketId) -> Result<Bucket, RuntimeError> {
        self.buckets
            .remove(&bucket_id)
            .ok_or(RuntimeError::BucketNotFound { bucket_id })
    }

    pub fn take_fee_claim(&mut self, epoch: Epoch, validator_public_key: PublicKey) -> Result<FeeClaim, RuntimeError> {
        let substate = self
            .virtual_substates
            .remove(&VirtualSubstateAddress::UnclaimedValidatorFee {
                epoch: epoch.as_u64(),
                address: validator_public_key.clone(),
            })
            .ok_or(RuntimeError::FeeClaimNotPermitted {
                epoch,
                address: validator_public_key.clone(),
            })?;

        let VirtualSubstate::UnclaimedValidatorFee(fee_claim) = substate else {
            return Err(RuntimeError::FeeClaimNotPermitted {
                epoch,
                address: validator_public_key,
            });
        };
        Ok(fee_claim)
    }

    pub fn get_current_epoch(&self) -> Result<Epoch, RuntimeError> {
        let address = VirtualSubstateAddress::CurrentEpoch;
        let current_epoch =
            self.virtual_substates
                .get(&address)
                .ok_or_else(|| RuntimeError::VirtualSubstateNotFound {
                    address: address.clone(),
                })?;
        let VirtualSubstate::CurrentEpoch(epoch) = current_epoch else {
            return Err(RuntimeError::VirtualSubstateNotFound { address });
        };
        Ok(Epoch(*epoch))
    }

    pub(super) fn validate_finalized(&self) -> Result<(), TransactionCommitError> {
        if !self.buckets.is_empty() {
            return Err(TransactionCommitError::DanglingBuckets {
                count: self.buckets.len(),
            });
        }

        Ok(())
    }
}
