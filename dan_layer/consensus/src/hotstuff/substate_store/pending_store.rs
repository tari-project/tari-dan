//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Cow, collections::HashMap};

use indexmap::IndexMap;
use log::*;
use tari_dan_common_types::{optional::Optional, SubstateAddress};
use tari_dan_storage::{
    consensus_models::{
        LockedSubstate,
        SubstateChange,
        SubstateLockFlag,
        SubstateRecord,
        VersionedSubstateIdLockIntent,
    },
    StateStore,
    StateStoreReadTransaction,
};
use tari_engine_types::substate::{Substate, SubstateId};
use tari_transaction::{TransactionId, VersionedSubstateId};

use super::error::SubstateStoreError;
use crate::traits::{ReadableSubstateStore, WriteableSubstateStore};

const LOG_TARGET: &str = "tari::dan::hotstuff::substate_store::pending_store";

pub struct PendingSubstateStore<'a, 'tx, TStore: StateStore + 'a + 'tx> {
    store: &'a TStore::ReadTransaction<'tx>,
    /// Map from substate address to the index in the diff list
    pending: HashMap<SubstateAddress, usize>,
    /// Append only list of changes ordered oldest to newest
    diff: Vec<SubstateChange>,
    new_locks: IndexMap<SubstateId, Vec<LockedSubstate>>,
}

impl<'a, 'tx, TStore: StateStore + 'a> PendingSubstateStore<'a, 'tx, TStore> {
    pub fn new(tx: &'a TStore::ReadTransaction<'tx>) -> Self {
        Self {
            store: tx,
            pending: HashMap::new(),
            diff: Vec::new(),
            new_locks: IndexMap::new(),
        }
    }

    pub fn read_transaction(&self) -> &'a TStore::ReadTransaction<'tx> {
        self.store
    }
}

impl<'a, 'tx, TStore: StateStore + 'a + 'tx> ReadableSubstateStore for PendingSubstateStore<'a, 'tx, TStore> {
    type Error = SubstateStoreError;

    fn get(&self, key: &SubstateAddress) -> Result<Substate, Self::Error> {
        if let Some(change) = self.get_pending(key) {
            return change.up().cloned().ok_or_else(|| SubstateStoreError::SubstateIsDown {
                id: change.versioned_substate_id().clone(),
            });
        }

        let Some(substate) = SubstateRecord::get(self.store, key).optional()? else {
            return Err(SubstateStoreError::SubstateNotFound { address: *key });
        };
        Ok(substate.into_substate())
    }
}

impl<'a, 'tx, TStore: StateStore + 'a + 'tx> WriteableSubstateStore for PendingSubstateStore<'a, 'tx, TStore> {
    fn put(&mut self, change: SubstateChange) -> Result<(), Self::Error> {
        match &change {
            SubstateChange::Up { id, .. } => {
                if let Some(v) = id.to_previous_version() {
                    self.assert_is_down(&v)?;
                }
                // self.assert_has_lock(id, SubstateLockFlag::Output)?;
            },
            SubstateChange::Down { id, .. } => {
                self.assert_is_up(id)?;
                // self.assert_has_lock(id, SubstateLockFlag::Write)?;
            },
        }

        self.insert(change);

        Ok(())
    }
}

impl<'a, 'tx, TStore: StateStore + 'a + 'tx> PendingSubstateStore<'a, 'tx, TStore> {
    pub fn get_latest(&self, id: &SubstateId) -> Result<Substate, SubstateStoreError> {
        // TODO: This returns the pledged inputs (local or foreign)

        // TODO(perf): O(n) lookup. Can be improved by maintaining a map of latest substates
        if let Some(substate) = self
            .diff
            .iter()
            .rev()
            .find(|change| change.versioned_substate_id().substate_id == *id)
            .and_then(|ch| ch.up())
        {
            return Ok(substate.clone());
        }

        let substate = SubstateRecord::get_latest(self.store, id)?;
        Ok(substate.into_substate())
    }

    pub fn try_lock_all<I: IntoIterator<Item = VersionedSubstateIdLockIntent>>(
        &mut self,
        transaction_id: TransactionId,
        id_locks: I,
        is_local_only: bool,
    ) -> Result<(), SubstateStoreError> {
        for id_lock in id_locks {
            self.try_lock(transaction_id, id_lock, is_local_only)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub fn try_lock(
        &mut self,
        transaction_id: TransactionId,
        requested_lock: VersionedSubstateIdLockIntent,
        is_local_only: bool,
    ) -> Result<(), SubstateStoreError> {
        let requested_lock_flag = requested_lock.lock_flag();
        let requested_substate_id = requested_lock.versioned_substate_id().substate_id();
        info!(
            target: LOG_TARGET,
            "🔒️ Requested substate lock: {}",
            requested_lock
        );

        let Some(existing) = self.get_latest_lock_by_id(requested_substate_id)? else {
            if requested_lock_flag.is_output() {
                self.assert_not_exist(requested_lock.versioned_substate_id())?;
            } else {
                self.assert_is_up(requested_lock.versioned_substate_id())?;
            }

            self.add_new_lock(
                requested_lock.versioned_substate_id().substate_id.clone(),
                LockedSubstate::new(
                    transaction_id,
                    requested_lock.versioned_substate_id().version(),
                    requested_lock_flag,
                    is_local_only,
                ),
            );
            return Ok(());
        };

        // Local-Only-Rules apply if: current lock is local-only AND requested lock is local only
        let has_local_only_rules = existing.is_local_only() && is_local_only;
        let same_transaction = existing.transaction_id() == transaction_id;

        match existing.substate_lock() {
            // If a substate is already locked as READ:
            // - it MAY be locked as READ
            // - it MUST NOT be locked as WRITE or OUTPUT, unless
            // - if Same-Transaction OR Local-Only-Rules:
            //   - it MAY be locked as requested.
            SubstateLockFlag::Read => {
                // Cannot write to or create an output for a substate that is already read locked
                if !same_transaction && !has_local_only_rules && !requested_lock_flag.is_read() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Read lock is present. Requested lock is {}",
                        requested_lock.versioned_substate_id(),
                        requested_lock_flag
                    );
                    return Err(SubstateStoreError::LockConflict {
                        substate_id: requested_lock.versioned_substate_id().clone(),
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_flag,
                    });
                }

                self.add_new_lock(
                    requested_lock.versioned_substate_id().substate_id.clone(),
                    LockedSubstate::new(
                        transaction_id,
                        requested_lock.versioned_substate_id().version(),
                        requested_lock_flag,
                        true,
                    ),
                );
            },

            // If a substate is already locked as WRITE:
            // - it MUST NOT be locked as READ, WRITE or OUTPUT, unless
            // - if Same-Transaction OR Local-Only-Rules:
            //   - it MAY be locked as OUTPUT
            SubstateLockFlag::Write => {
                // Cannot lock a non-local WRITE locked substate
                if !has_local_only_rules && !same_transaction {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Write lock is present. Requested lock is {}",
                        requested_lock.versioned_substate_id(),
                        requested_lock_flag
                    );
                    return Err(SubstateStoreError::LockConflict {
                        substate_id: requested_lock.versioned_substate_id().clone(),
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_flag,
                    });
                }

                if !requested_lock_flag.is_output() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Write lock is present. Requested lock is {}",
                        requested_lock.versioned_substate_id(),
                        requested_lock_flag
                    );
                    return Err(SubstateStoreError::LockConflict {
                        substate_id: requested_lock.versioned_substate_id().clone(),
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_flag,
                    });
                }

                self.add_new_lock(
                    requested_lock.versioned_substate_id().substate_id.clone(),
                    LockedSubstate::new(
                        transaction_id,
                        requested_lock.versioned_substate_id().version(),
                        SubstateLockFlag::Output,
                        true,
                    ),
                );
            },
            // If a substate is already locked as OUTPUT:
            // - it MUST NOT be locked as READ, WRITE or OUTPUT, unless
            // - if Same-Transaction OR Local-Only-Rules:
            //   - it MAY be locked as WRITE or READ
            SubstateLockFlag::Output => {
                if !same_transaction && !has_local_only_rules {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Output lock is present. Requested lock is {}",
                        requested_lock.versioned_substate_id(),
                        requested_lock_flag
                    );
                    return Err(SubstateStoreError::LockConflict {
                        substate_id: requested_lock.versioned_substate_id().clone(),
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_flag,
                    });
                }

                if requested_lock_flag.is_output() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Output lock is present. Requested lock is output",
                        requested_lock.versioned_substate_id()
                    );
                    return Err(SubstateStoreError::LockConflict {
                        substate_id: requested_lock.versioned_substate_id().clone(),
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_flag,
                    });
                }

                self.add_new_lock(
                    requested_lock.versioned_substate_id().substate_id.clone(),
                    LockedSubstate::new(
                        transaction_id,
                        requested_lock.versioned_substate_id().version(),
                        // WRITE or READ
                        requested_lock_flag,
                        true,
                    ),
                );
            },
        }

        Ok(())
    }

    fn get_pending(&self, key: &SubstateAddress) -> Option<&SubstateChange> {
        self.pending
            .get(key)
            .map(|&pos| self.diff.get(pos).expect("Index map and diff are out of sync"))
    }

    fn insert(&mut self, change: SubstateChange) {
        self.pending.insert(change.to_substate_address(), self.diff.len());
        self.diff.push(change)
    }

    fn get_latest_lock_by_id(&self, id: &SubstateId) -> Result<Option<Cow<'_, LockedSubstate>>, SubstateStoreError> {
        if let Some(lock) = self.new_locks.get(id).and_then(|locks| locks.last()) {
            return Ok(Some(Cow::Borrowed(lock)));
        }

        let maybe_lock = self.store.substate_locks_get_latest_for_substate(id).optional()?;
        Ok(maybe_lock.map(Cow::Owned))
    }

    fn add_new_lock(&mut self, substate_id: SubstateId, lock: LockedSubstate) {
        self.new_locks.entry(substate_id).or_default().push(lock);
    }

    fn assert_is_up(&self, id: &VersionedSubstateId) -> Result<(), SubstateStoreError> {
        let address = id.to_substate_address();
        if let Some(change) = self.get_pending(&address) {
            if change.is_down() {
                return Err(SubstateStoreError::SubstateIsDown { id: id.clone() });
            }
            return Ok(());
        }

        let is_up = SubstateRecord::substate_is_up(self.store, &address)
            .optional()?
            .unwrap_or(false);
        if !is_up {
            return Err(SubstateStoreError::SubstateIsDown { id: id.clone() });
        }

        Ok(())
    }

    fn assert_is_down(&self, id: &VersionedSubstateId) -> Result<(), SubstateStoreError> {
        let address = id.to_substate_address();
        if let Some(change) = self.get_pending(&address) {
            if change.is_up() {
                return Err(SubstateStoreError::ExpectedSubstateDown { id: id.clone() });
            }
            return Ok(());
        }

        let Some(is_up) = SubstateRecord::substate_is_up(self.store, &address).optional()? else {
            debug!(target: LOG_TARGET, "Expected substate {} to be DOWN but it does not exist", address);
            return Err(SubstateStoreError::SubstateNotFound { address });
        };
        if is_up {
            return Err(SubstateStoreError::ExpectedSubstateDown { id: id.clone() });
        }

        Ok(())
    }

    fn assert_not_exist(&self, id: &VersionedSubstateId) -> Result<(), SubstateStoreError> {
        let address = id.to_substate_address();
        if let Some(change) = self.get_pending(&address) {
            if change.is_up() {
                return Err(SubstateStoreError::ExpectedSubstateNotExist { id: id.clone() });
            }
            return Ok(());
        }

        if SubstateRecord::exists(self.store, &address)? {
            return Err(SubstateStoreError::ExpectedSubstateNotExist { id: id.clone() });
        }

        Ok(())
    }

    pub fn new_locks(&self) -> &IndexMap<SubstateId, Vec<LockedSubstate>> {
        &self.new_locks
    }

    pub fn diff(&self) -> &Vec<SubstateChange> {
        &self.diff
    }

    pub fn into_diff_and_locks(self) -> (Vec<SubstateChange>, IndexMap<SubstateId, Vec<LockedSubstate>>) {
        (self.diff, self.new_locks)
    }
}
