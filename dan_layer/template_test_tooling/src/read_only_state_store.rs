//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_engine::state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateStoreError};
use tari_engine_types::{
    component::ComponentHeader,
    indexed_value::IndexedValue,
    resource::Resource,
    substate::{Substate, SubstateAddress},
    vault::Vault,
};
use tari_template_lib::models::{ComponentAddress, ResourceAddress, VaultId};

pub struct ReadOnlyStateStore {
    store: MemoryStateStore,
}
impl ReadOnlyStateStore {
    pub fn new(store: MemoryStateStore) -> Self {
        Self { store }
    }

    pub fn get_component(&self, component_address: ComponentAddress) -> Result<ComponentHeader, StateStoreError> {
        let substate = self.get_substate(&SubstateAddress::Component(component_address))?;
        Ok(substate.into_substate_value().into_component().unwrap())
    }

    pub fn get_resource(&self, resource_address: &ResourceAddress) -> Result<Resource, StateStoreError> {
        let substate = self.get_substate(&SubstateAddress::Resource(*resource_address))?;
        Ok(substate.into_substate_value().into_resource().unwrap())
    }

    pub fn get_vault(&self, vault_id: &VaultId) -> Result<Vault, StateStoreError> {
        let substate = self.get_substate(&SubstateAddress::Vault(*vault_id))?;
        Ok(substate.into_substate_value().into_vault().unwrap())
    }

    pub fn inspect_component(&self, component_address: ComponentAddress) -> Result<IndexedValue, StateStoreError> {
        let component = self.get_component(component_address)?;
        Ok(IndexedValue::from_value(component.into_state()).unwrap())
    }

    pub fn get_substate(&self, address: &SubstateAddress) -> Result<Substate, StateStoreError> {
        let tx = self.store.read_access()?;
        let substate = tx.get_state::<_, Substate>(address)?;
        Ok(substate)
    }
}
