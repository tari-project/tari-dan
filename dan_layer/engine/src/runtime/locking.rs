//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::Display,
};

use tari_dan_common_types::optional::IsNotFoundError;
use tari_engine_types::{
    lock::{LockFlag, LockId},
    substate::SubstateId,
};

#[derive(Debug, Default, Clone)]
pub struct LockedSubstates {
    lock_ids: HashMap<LockId, SubstateId>,
    locks: HashMap<SubstateId, LockState>,
    id_counter: LockId,
}

impl LockedSubstates {
    pub fn try_lock(&mut self, addr: &SubstateId, lock_flag: LockFlag) -> Result<LockId, LockError> {
        match self.locks.get(addr) {
            Some(state @ LockState::Read(count)) => {
                if lock_flag.is_write() {
                    return Err(LockError::InvalidLockRequest {
                        address: addr.clone(),
                        requested_lock: lock_flag,
                        lock_state: *state,
                    });
                }

                self.locks.insert(addr.clone(), LockState::Read(*count + 1));
                let id = self.next_id()?;
                self.lock_ids.insert(id, addr.clone());
                Ok(id)
            },
            Some(LockState::Write) => {
                if lock_flag.is_write() {
                    // Just a slightly clearer error for this case
                    return Err(LockError::MultipleWriteLockRequested { address: addr.clone() });
                }

                Err(LockError::InvalidLockRequest {
                    address: addr.clone(),
                    requested_lock: lock_flag,
                    lock_state: LockState::Write,
                })
            },
            None => {
                self.locks.insert(addr.clone(), lock_flag.into());
                let id = self.next_id()?;
                self.lock_ids.insert(id, addr.clone());
                Ok(id)
            },
        }
    }

    pub fn try_unlock(&mut self, lock_id: LockId) -> Result<(), LockError> {
        let addr = self
            .lock_ids
            .remove(&lock_id)
            .ok_or(LockError::LockIdNotFound { lock_id })?;
        let entry = self.locks.entry(addr);
        match entry {
            Entry::Occupied(mut val) => match val.get_mut() {
                LockState::Read(count_mut) => {
                    *count_mut -= 1;
                    if *count_mut == 0 {
                        val.remove_entry();
                    }
                },
                LockState::Write => {
                    val.remove_entry();
                },
            },
            Entry::Vacant(vacant) => {
                return Err(LockError::InvariantError {
                    function: "LockedSubstates::try_unlock",
                    details: format!(
                        "Lock id {} was found but the address {} did not exist in the locks map",
                        lock_id,
                        vacant.key()
                    ),
                });
            },
        }
        Ok(())
    }

    pub fn get(&self, lock_id: LockId, lock_flag: LockFlag) -> Result<LockedSubstate, LockError> {
        let addr = self
            .lock_ids
            .get(&lock_id)
            .ok_or(LockError::LockIdNotFound { lock_id })?;

        let lock_state = self.locks.get(addr).ok_or_else(|| LockError::InvariantError {
            function: "LockedSubstates::get",
            details: format!("Lock id {lock_id} was found but the address {addr} did not exist in the locks map"),
        })?;

        if !lock_state.has_access(lock_flag) {
            return Err(LockError::InvalidLockRequest {
                address: addr.clone(),
                requested_lock: lock_flag,
                lock_state: *lock_state,
            });
        }

        Ok(LockedSubstate::new(addr.clone(), lock_id, lock_flag))
    }

    fn next_id(&mut self) -> Result<LockId, LockError> {
        let id = self.id_counter;
        self.id_counter = self
            .id_counter
            .checked_add(1)
            .ok_or_else(|| LockError::InvariantError {
                function: "LockedSubstates::next_id",
                details: "[LockedSubstates::next_id] ID counter overflowed. Too many locked objects.".to_string(),
            })?;
        Ok(id)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LockState {
    Read(usize),
    Write,
}

impl LockState {
    pub fn is_read(&self) -> bool {
        matches!(self, Self::Read(_))
    }

    pub fn is_write(&self) -> bool {
        matches!(self, Self::Write)
    }

    pub fn has_access(&self, lock_flag: LockFlag) -> bool {
        match lock_flag {
            LockFlag::Read => self.is_read() || self.is_write(),
            LockFlag::Write => self.is_write(),
        }
    }
}

impl From<LockFlag> for LockState {
    fn from(flag: LockFlag) -> Self {
        match flag {
            LockFlag::Read => Self::Read(1),
            LockFlag::Write => Self::Write,
        }
    }
}

impl Display for LockState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockState::Read(count) => write!(f, "Read({count})"),
            LockState::Write => write!(f, "Write"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LockedSubstate {
    address: SubstateId,
    lock_id: u32,
    lock_flag: LockFlag,
}

impl LockedSubstate {
    pub(super) fn new(address: SubstateId, lock_id: u32, lock_flag: LockFlag) -> Self {
        Self {
            address,
            lock_id,
            lock_flag,
        }
    }

    pub fn address(&self) -> &SubstateId {
        &self.address
    }

    pub fn lock_id(&self) -> u32 {
        self.lock_id
    }

    pub fn check_access(&self, lock_flag: LockFlag) -> Result<(), LockError> {
        let has_access = match lock_flag {
            LockFlag::Read => self.lock_flag.is_read() || self.lock_flag.is_write(),
            LockFlag::Write => self.lock_flag.is_write(),
        };
        if !has_access {
            return Err(LockError::InvalidLockAccess {
                address: self.address.clone(),
                requested: lock_flag,
                actual: self.lock_flag,
            });
        }

        Ok(())
    }
}
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("Lock ID not found: {lock_id}")]
    LockIdNotFound { lock_id: LockId },
    #[error("Substate {address} not locked")]
    SubstateNotLocked { address: SubstateId },
    #[error("BUG: Invariant error: {details}")]
    InvariantError { function: &'static str, details: String },
    #[error("Requested {requested_lock} lock on substate {address} but it is already locked with {lock_state}")]
    InvalidLockRequest {
        address: SubstateId,
        requested_lock: LockFlag,
        lock_state: LockState,
    },
    #[error("Multiple write locks requested for substate {address}")]
    MultipleWriteLockRequested { address: SubstateId },
    #[error("Lock for {address} does not have the required access. Requested: {requested}, Actual: {actual}")]
    InvalidLockAccess {
        address: SubstateId,
        requested: LockFlag,
        actual: LockFlag,
    },
}

impl IsNotFoundError for LockError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::LockIdNotFound { .. })
    }
}
