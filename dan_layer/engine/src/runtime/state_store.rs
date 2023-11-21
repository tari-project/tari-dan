//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, mem};

use indexmap::IndexMap;
use tari_dan_common_types::optional::Optional;
use tari_engine_types::{
    component::ComponentHeader,
    lock::{LockFlag, LockId},
    substate::{Substate, SubstateAddress, SubstateValue},
    vault::Vault,
};
use tari_template_lib::models::{ComponentAddress, VaultId};

use crate::{
    runtime::{
        locking::{LockError, LockedSubstates},
        RuntimeError,
    },
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader},
};

#[derive(Debug, Clone)]
pub struct WorkingStateStore {
    // This must be ordered deterministically since we use this to create the substate diff
    new_substates: IndexMap<SubstateAddress, SubstateValue>,

    loaded_substates: HashMap<SubstateAddress, SubstateValue>,
    locked_substates: LockedSubstates,

    state_store: MemoryStateStore,
}

impl WorkingStateStore {
    pub fn new(state_store: MemoryStateStore) -> Self {
        Self {
            new_substates: IndexMap::new(),
            loaded_substates: HashMap::new(),
            locked_substates: Default::default(),
            state_store,
        }
    }

    pub fn try_lock(&mut self, address: &SubstateAddress, lock_flag: LockFlag) -> Result<LockId, RuntimeError> {
        if !self.exists(address)? {
            return Err(RuntimeError::SubstateNotFound {
                address: address.clone(),
            });
        }
        let lock_id = self.locked_substates.try_lock(address, lock_flag)?;
        self.load(address)?;
        Ok(lock_id)
    }

    pub fn try_unlock(&mut self, lock_id: LockId) -> Result<(), LockError> {
        self.locked_substates.try_unlock(lock_id)?;
        Ok(())
    }

    pub fn get_locked_substate_mut(
        &mut self,
        lock_id: LockId,
    ) -> Result<(SubstateAddress, &mut SubstateValue), RuntimeError> {
        let lock = self.locked_substates.get(lock_id, LockFlag::Write)?;
        let substate = self.get_for_mut(lock.address())?;
        Ok((lock.address().clone(), substate))
    }

    pub fn get_locked_substate(&self, lock_id: LockId) -> Result<(SubstateAddress, &SubstateValue), RuntimeError> {
        let lock = self.locked_substates.get(lock_id, LockFlag::Read)?;
        let substate = self.get_ref(lock.address())?;
        Ok((lock.address().clone(), substate))
    }

    fn get_ref(&self, address: &SubstateAddress) -> Result<&SubstateValue, LockError> {
        self.new_substates
            .get(address)
            .or_else(|| self.loaded_substates.get(address))
            .ok_or_else(|| LockError::SubstateNotLocked {
                address: address.clone(),
            })
    }

    fn get_for_mut(&mut self, address: &SubstateAddress) -> Result<&mut SubstateValue, LockError> {
        if let Some(substate) = self.loaded_substates.remove(address) {
            self.new_substates.insert(address.clone(), substate);
        }

        if let Some(substate_mut) = self.new_substates.get_mut(address) {
            return Ok(substate_mut);
        }

        Err(LockError::SubstateNotLocked {
            address: address.clone(),
        })
    }

    pub fn exists(&self, address: &SubstateAddress) -> Result<bool, RuntimeError> {
        let exists = self.new_substates.contains_key(address) || self.loaded_substates.contains_key(address) || {
            let tx = self.state_store.read_access()?;
            tx.exists(address)?
        };
        Ok(exists)
    }

    pub fn insert(&mut self, address: SubstateAddress, value: SubstateValue) -> Result<(), RuntimeError> {
        if self.exists(&address)? {
            return Err(RuntimeError::DuplicateSubstate { address });
        }
        self.new_substates.insert(address, value);
        Ok(())
    }

    fn load(&mut self, address: &SubstateAddress) -> Result<(), RuntimeError> {
        if self.new_substates.contains_key(address) {
            return Ok(());
        }
        if self.loaded_substates.contains_key(address) {
            return Ok(());
        }
        let tx = self.state_store.read_access()?;
        let substate =
            tx.get_state::<_, Substate>(address)
                .optional()?
                .ok_or_else(|| RuntimeError::SubstateNotFound {
                    address: address.clone(),
                })?;
        let substate = substate.into_substate_value();
        self.loaded_substates.insert(address.clone(), substate);
        Ok(())
    }

    pub fn take_mutated_substates(&mut self) -> IndexMap<SubstateAddress, SubstateValue> {
        mem::take(&mut self.new_substates)
    }

    pub fn new_vaults(&self) -> impl Iterator<Item = (VaultId, &Vault)> + '_ {
        self.new_substates
            .iter()
            .filter(|(address, _)| address.is_vault())
            .map(|(addr, vault)| (addr.as_vault_id().unwrap(), vault.as_vault().unwrap()))
    }

    pub(super) fn state_store(&self) -> &MemoryStateStore {
        &self.state_store
    }

    /// Load and get the component without a lock
    pub fn load_component(&mut self, address: &ComponentAddress) -> Result<&ComponentHeader, RuntimeError> {
        let addr = SubstateAddress::Component(*address);
        self.load(&addr)?;
        let component = self.get_ref(&addr)?;
        component.component().ok_or_else(|| RuntimeError::InvariantError {
            function: "load_component",
            details: format!("Substate at address {} is not a component", addr),
        })
    }

    pub(super) fn get_unmodified_substate(&self, address: &SubstateAddress) -> Result<Substate, RuntimeError> {
        let tx = self.state_store.read_access()?;
        tx.get_state(address)
            .optional()?
            .ok_or_else(|| RuntimeError::SubstateNotFound {
                address: address.clone(),
            })
    }
}
