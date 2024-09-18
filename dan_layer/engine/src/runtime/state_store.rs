//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, mem};

use indexmap::IndexMap;
use tari_dan_common_types::optional::Optional;
use tari_engine_types::{
    component::ComponentHeader,
    lock::{LockFlag, LockId},
    substate::{Substate, SubstateId, SubstateValue},
    vault::Vault,
};
use tari_template_lib::models::{ComponentAddress, VaultId};

use crate::{
    runtime::{
        locking::{LockError, LockedSubstates},
        RuntimeError,
    },
    state_store::{memory::ReadOnlyMemoryStateStore, StateReader},
};

#[derive(Debug, Clone)]
pub struct WorkingStateStore {
    // This must be ordered deterministically since we use this to create the substate diff
    new_substates: IndexMap<SubstateId, SubstateValue>,

    loaded_substates: HashMap<SubstateId, SubstateValue>,
    locked_substates: LockedSubstates,

    state_store: ReadOnlyMemoryStateStore,
}

impl WorkingStateStore {
    pub fn new(state_store: ReadOnlyMemoryStateStore) -> Self {
        Self {
            new_substates: IndexMap::new(),
            loaded_substates: HashMap::new(),
            locked_substates: Default::default(),
            state_store,
        }
    }

    pub fn try_lock(&mut self, address: &SubstateId, lock_flag: LockFlag) -> Result<LockId, RuntimeError> {
        if !self.exists(address)? {
            return Err(RuntimeError::SubstateNotFound { id: address.clone() });
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
    ) -> Result<(SubstateId, &mut SubstateValue), RuntimeError> {
        let lock = self.locked_substates.get(lock_id, LockFlag::Write)?;
        let substate = self.get_for_mut(lock.address())?;
        Ok((lock.address().clone(), substate))
    }

    pub fn mutate_locked_substate_with<
        R,
        F: FnOnce(&SubstateId, &mut SubstateValue) -> Result<Option<R>, RuntimeError>,
    >(
        &mut self,
        lock_id: LockId,
        callback: F,
    ) -> Result<Option<R>, RuntimeError> {
        let lock = self.locked_substates.get(lock_id, LockFlag::Write)?;
        if let Some(mut substate) = self.loaded_substates.remove(lock.address()) {
            return match callback(lock.address(), &mut substate)? {
                Some(ret) => {
                    self.new_substates.insert(lock.address().clone(), substate);
                    Ok(Some(ret))
                },
                None => {
                    // It is undefined to mutate the state and return None from the callback. We do not assert this
                    // however which is risky.
                    self.loaded_substates.insert(lock.address().clone(), substate);
                    Ok(None)
                },
            };
        }

        let substate_mut = self
            .new_substates
            .get_mut(lock.address())
            .ok_or_else(|| LockError::SubstateNotLocked {
                address: lock.address().clone(),
            })?;

        // Since the substate is already mutated, we dont really care if the callback mutates it again or not
        callback(lock.address(), substate_mut)
    }

    pub fn get_locked_substate(&self, lock_id: LockId) -> Result<(SubstateId, &SubstateValue), RuntimeError> {
        let lock = self.locked_substates.get(lock_id, LockFlag::Read)?;
        let substate = self.get_ref(lock.address())?;
        Ok((lock.address().clone(), substate))
    }

    fn get_ref(&self, address: &SubstateId) -> Result<&SubstateValue, LockError> {
        self.new_substates
            .get(address)
            .or_else(|| self.loaded_substates.get(address))
            .ok_or_else(|| LockError::SubstateNotLocked {
                address: address.clone(),
            })
    }

    fn get_for_mut(&mut self, address: &SubstateId) -> Result<&mut SubstateValue, LockError> {
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

    pub fn exists(&self, id: &SubstateId) -> Result<bool, RuntimeError> {
        let exists = self.new_substates.contains_key(id) ||
            self.loaded_substates.contains_key(id) ||
            self.state_store.exists(id)?;
        Ok(exists)
    }

    pub fn insert(&mut self, id: SubstateId, value: SubstateValue) -> Result<(), RuntimeError> {
        if self.exists(&id)? {
            return Err(RuntimeError::DuplicateSubstate { address: id });
        }
        self.new_substates.insert(id, value);
        Ok(())
    }

    fn load(&mut self, id: &SubstateId) -> Result<(), RuntimeError> {
        if self.new_substates.contains_key(id) {
            return Ok(());
        }
        if self.loaded_substates.contains_key(id) {
            return Ok(());
        }
        let substate = self
            .state_store
            .get_state(id)
            .optional()?
            .ok_or_else(|| RuntimeError::SubstateNotFound { id: id.clone() })?;
        let substate = substate.substate_value().clone();
        self.loaded_substates.insert(id.clone(), substate);
        Ok(())
    }

    pub fn take_mutated_substates(&mut self) -> IndexMap<SubstateId, SubstateValue> {
        mem::take(&mut self.new_substates)
    }

    pub fn mutated_substates(&self) -> &IndexMap<SubstateId, SubstateValue> {
        &self.new_substates
    }

    pub fn new_vaults(&self) -> impl Iterator<Item = (VaultId, &Vault)> + '_ {
        self.new_substates
            .iter()
            .filter(|(address, _)| address.is_vault())
            .map(|(addr, vault)| (addr.as_vault_id().unwrap(), vault.as_vault().unwrap()))
    }

    pub(super) fn state_store(&self) -> &ReadOnlyMemoryStateStore {
        &self.state_store
    }

    /// Load and get the component without a lock
    pub fn load_component(&mut self, address: &ComponentAddress) -> Result<&ComponentHeader, RuntimeError> {
        let addr = SubstateId::Component(*address);
        self.load(&addr)?;
        let component = self.get_ref(&addr)?;
        component.component().ok_or_else(|| RuntimeError::InvariantError {
            function: "load_component",
            details: format!("Substate at address {} is not a component", addr),
        })
    }

    pub(super) fn get_unmodified_substate(&self, address: &SubstateId) -> Result<&Substate, RuntimeError> {
        self.state_store
            .get_state(address)
            .optional()?
            .ok_or_else(|| RuntimeError::SubstateNotFound { id: address.clone() })
    }
}
