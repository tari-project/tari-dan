//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Cow, collections::HashMap, fmt::Display};

use indexmap::IndexMap;
use log::*;
use tari_dan_common_types::{
    optional::Optional,
    LockIntent,
    NumPreshards,
    SubstateAddress,
    SubstateLockType,
    ToSubstateAddress,
    VersionedSubstateId,
};
use tari_dan_storage::{
    consensus_models::{BlockDiff, BlockId, SubstateChange, SubstateLock, SubstateRecord},
    StateStore,
    StateStoreReadTransaction,
};
use tari_engine_types::substate::{Substate, SubstateDiff, SubstateId};
use tari_transaction::TransactionId;

use super::error::SubstateStoreError;
use crate::{
    hotstuff::substate_store::LockFailedError,
    traits::{ReadableSubstateStore, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::dan::hotstuff::substate_store::pending_store";

pub struct PendingSubstateStore<'a, 'tx, TStore: StateStore + 'a + 'tx> {
    store: &'a TStore::ReadTransaction<'tx>,
    /// Map from substate address to the index in the diff list
    pending: HashMap<SubstateAddress, usize>,
    /// Append only list of changes ordered oldest to newest
    diff: Vec<SubstateChange>,
    new_locks: IndexMap<SubstateId, Vec<SubstateLock>>,
    parent_block: BlockId,
    num_preshards: NumPreshards,
}

impl<'a, 'tx, TStore: StateStore + 'a> PendingSubstateStore<'a, 'tx, TStore> {
    pub fn new(store: &'a TStore::ReadTransaction<'tx>, parent_block: BlockId, num_preshards: NumPreshards) -> Self {
        Self {
            store,
            pending: HashMap::new(),
            diff: Vec::new(),
            new_locks: IndexMap::new(),
            parent_block,
            num_preshards,
        }
    }

    pub fn read_transaction(&self) -> &'a TStore::ReadTransaction<'tx> {
        self.store
    }
}

impl<'a, 'tx, TStore: StateStore + 'a + 'tx> ReadableSubstateStore for PendingSubstateStore<'a, 'tx, TStore> {
    type Error = SubstateStoreError;

    fn get(&self, id: &VersionedSubstateId) -> Result<Substate, Self::Error> {
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            return change.up().cloned().ok_or_else(|| SubstateStoreError::SubstateIsDown {
                id: change.versioned_substate_id().clone(),
            });
        }

        if let Some(change) =
            BlockDiff::get_for_substate(self.read_transaction(), &self.parent_block, &id.substate_id).optional()?
        {
            return change
                .into_up()
                .ok_or_else(|| SubstateStoreError::SubstateIsDown { id: id.clone() });
        }

        let Some(substate) = SubstateRecord::get(self.read_transaction(), &id.to_substate_address()).optional()? else {
            return Err(SubstateStoreError::SubstateNotFound { id: id.clone() });
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

    fn put_diff(&mut self, transaction_id: TransactionId, diff: &SubstateDiff) -> Result<(), Self::Error> {
        for (id, version) in diff.down_iter() {
            let id = VersionedSubstateId::new(id.clone(), *version);
            let shard = id.to_substate_address().to_shard(self.num_preshards);
            self.put(SubstateChange::Down {
                id,
                shard,
                transaction_id,
            })?;
        }

        for (id, substate) in diff.up_iter() {
            let id = VersionedSubstateId::new(id.clone(), substate.version());
            let shard = id.to_substate_address().to_shard(self.num_preshards);
            self.put(SubstateChange::Up {
                id,
                shard,
                substate: substate.clone(),
                transaction_id,
            })?;
        }

        Ok(())
    }
}

impl<'a, 'tx, TStore: StateStore + 'a + 'tx> PendingSubstateStore<'a, 'tx, TStore> {
    pub fn get_latest(&self, id: &SubstateId) -> Result<Substate, SubstateStoreError> {
        if let Some(ch) = self
            .diff
            .iter()
            .rev()
            .find(|change| change.versioned_substate_id().substate_id == *id)
        {
            let substate = ch.up().ok_or_else(|| SubstateStoreError::SubstateIsDown {
                id: ch.versioned_substate_id().clone(),
            })?;
            return Ok(substate.clone());
        }

        if let Some(change) = BlockDiff::get_for_substate(self.read_transaction(), &self.parent_block, id).optional()? {
            let id = change.versioned_substate_id().clone();
            return change
                .into_up()
                .ok_or_else(|| SubstateStoreError::SubstateIsDown { id });
        }

        let substate = SubstateRecord::get_latest(self.read_transaction(), id)?;
        Ok(substate.into_substate())
    }

    pub fn try_lock_all<I, L>(
        &mut self,
        transaction_id: TransactionId,
        id_locks: I,
        is_local_only: bool,
    ) -> Result<LockStatus, SubstateStoreError>
    where
        I: IntoIterator<Item = L>,
        L: LockIntent + Display,
    {
        let mut lock_status = LockStatus::new();
        for lock in id_locks {
            match self.try_lock(transaction_id, &lock, is_local_only) {
                Ok(()) => continue,
                Err(err) => {
                    let error = err.ok_lock_failed()?;
                    match error {
                        err @ LockFailedError::SubstateNotFound { .. } => {
                            // If the substate does not exist, the transaction is invalid
                            let index = lock_status.add_failed(err);
                            lock_status.hard_conflict_idx = Some(index);
                        },
                        err @ LockFailedError::LockConflict { .. } => {
                            let index = lock_status.add_failed(err);
                            // If the requested lock is for a specific version, the transaction must be ABORTED
                            if lock.requested_version().is_some() {
                                lock_status.hard_conflict_idx = Some(index);
                            }
                        },
                    }
                },
            }

            if lock_status.is_hard_conflict() {
                // If there are hard conflicts, there is no need to continue as this transaction will be ABORTED
                break;
            }
        }
        Ok(lock_status)
    }

    #[allow(clippy::too_many_lines)]
    pub fn try_lock<L: LockIntent + Display>(
        &mut self,
        transaction_id: TransactionId,
        requested_lock: &L,
        is_local_only: bool,
    ) -> Result<(), SubstateStoreError> {
        let requested_lock_type = requested_lock.lock_type();
        info!(
            target: LOG_TARGET,
            "🔒️ Requested substate lock: {}",
            requested_lock
        );

        let versioned_substate_id = requested_lock.to_versioned_substate_id();

        let Some(existing) = self.get_latest_lock_by_id(versioned_substate_id.substate_id())? else {
            if requested_lock_type.is_output() {
                self.assert_not_exist(&versioned_substate_id)?;
            } else {
                self.assert_is_up(&versioned_substate_id)?;
            }

            let version = versioned_substate_id.version();
            self.add_new_lock(
                versioned_substate_id.substate_id,
                SubstateLock::new(transaction_id, version, requested_lock_type, is_local_only),
            );
            return Ok(());
        };

        // Local-Only-Rules apply if: current lock is local-only AND requested lock is local only
        let has_local_only_rules = existing.is_local_only() && is_local_only;
        let same_transaction = existing.transaction_id() == transaction_id;

        // Duplicate lock requests on the same transaction are idempotent
        if same_transaction {
            return Ok(());
        }

        match existing.substate_lock() {
            // If a substate is already locked as READ:
            // - it MAY be locked as READ
            // - it MUST NOT be locked as WRITE or OUTPUT, unless
            // - if Same-Transaction OR Local-Only-Rules:
            //   - it MAY be locked as requested.
            SubstateLockType::Read => {
                // Cannot write to or create an output for a substate that is already read locked
                if !has_local_only_rules && !requested_lock_type.is_read() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Read lock(local_only={}) is present. Requested lock is {}(local_only={})",
                        versioned_substate_id,
                        existing.is_local_only(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id,
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_type,
                    }
                    .into());
                }

                let version = versioned_substate_id.version();
                self.add_new_lock(
                    versioned_substate_id.substate_id,
                    SubstateLock::new(transaction_id, version, requested_lock_type, is_local_only),
                );
            },

            // If a substate is already locked as WRITE:
            // - it MUST NOT be locked as READ, WRITE or OUTPUT, unless
            // - if Same-Transaction OR Local-Only-Rules:
            //   - it MAY be locked as OUTPUT
            SubstateLockType::Write => {
                // Cannot lock a non-local_only WRITE locked substate
                if !has_local_only_rules {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Write lock(local_only={}) is present. Requested lock is {}(local_only={})",
                        versioned_substate_id,
                        existing.is_local_only(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id,
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_type,
                    }
                    .into());
                }

                if !requested_lock_type.is_output() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Write lock(local_only={}) is present. Requested lock is {}(local_only={})",
                        versioned_substate_id,
                        existing.is_local_only(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id,
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_type,
                    }
                    .into());
                }

                let version = versioned_substate_id.version();
                self.add_new_lock(
                    versioned_substate_id.substate_id,
                    SubstateLock::new(transaction_id, version, SubstateLockType::Output, is_local_only),
                );
            },
            // If a substate is already locked as OUTPUT:
            // - it MUST NOT be locked as READ, WRITE or OUTPUT, unless
            // - if Same-Transaction OR Local-Only-Rules:
            //   - it MAY be locked as WRITE or READ
            //   - it MUST NOT be locked as OUTPUT
            SubstateLockType::Output => {
                if !has_local_only_rules {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}, {}] Output lock(local_only={}) is present. Requested lock is {}(local_only={})",
                        transaction_id,
                        versioned_substate_id,
                        existing.is_local_only(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id,
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_type,
                    }
                    .into());
                }

                if requested_lock_type.is_output() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}, {}] Output lock(local_only={}) is present. Requested lock is Output(local_only={})",
                        transaction_id,
                        versioned_substate_id,
                        existing.is_local_only(),
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id,
                        existing_lock: existing.substate_lock(),
                        requested_lock: requested_lock_type,
                    }
                    .into());
                }

                let version = versioned_substate_id.version();
                self.add_new_lock(
                    versioned_substate_id.substate_id,
                    SubstateLock::new(
                        transaction_id,
                        version,
                        // WRITE or READ
                        requested_lock_type,
                        is_local_only,
                    ),
                );
            },
        }

        Ok(())
    }

    fn get_pending(&self, addr: &SubstateAddress) -> Option<&SubstateChange> {
        self.pending
            .get(addr)
            .map(|&pos| self.diff.get(pos).expect("Index map and diff are out of sync"))
    }

    fn insert(&mut self, change: SubstateChange) {
        self.pending.insert(change.to_substate_address(), self.diff.len());
        self.diff.push(change)
    }

    fn get_latest_lock_by_id(&self, id: &SubstateId) -> Result<Option<Cow<'_, SubstateLock>>, SubstateStoreError> {
        if let Some(lock) = self.new_locks.get(id).and_then(|locks| locks.last()) {
            return Ok(Some(Cow::Borrowed(lock)));
        }

        let maybe_lock = self
            .read_transaction()
            .substate_locks_get_latest_for_substate(id)
            .optional()?;
        Ok(maybe_lock.map(Cow::Owned))
    }

    fn add_new_lock(&mut self, substate_id: SubstateId, lock: SubstateLock) {
        self.new_locks.entry(substate_id).or_default().push(lock);
    }

    fn assert_is_up(&self, id: &VersionedSubstateId) -> Result<(), SubstateStoreError> {
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            if change.is_down() {
                return Err(SubstateStoreError::SubstateIsDown { id: id.clone() });
            }
            return Ok(());
        }

        if let Some(change) =
            BlockDiff::get_for_substate(self.read_transaction(), &self.parent_block, &id.substate_id).optional()?
        {
            if change.is_up() {
                return Ok(());
            }
            return Err(SubstateStoreError::SubstateIsDown { id: id.clone() });
        }

        match SubstateRecord::substate_is_up(self.read_transaction(), &id.to_substate_address()).optional()? {
            Some(true) => Ok(()),
            Some(false) => Err(SubstateStoreError::SubstateIsDown { id: id.clone() }),
            None => Err(SubstateStoreError::SubstateNotFound { id: id.clone() }),
        }
    }

    fn assert_is_down(&self, id: &VersionedSubstateId) -> Result<(), SubstateStoreError> {
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            if change.is_up() {
                return Err(SubstateStoreError::ExpectedSubstateDown { id: id.clone() });
            }
            return Ok(());
        }

        let address = id.to_substate_address();
        let Some(is_up) = SubstateRecord::substate_is_up(self.read_transaction(), &address).optional()? else {
            debug!(target: LOG_TARGET, "Expected substate {} to be DOWN but it does not exist", address);
            return Err(SubstateStoreError::SubstateNotFound { id: id.clone() });
        };
        if is_up {
            return Err(SubstateStoreError::ExpectedSubstateDown { id: id.clone() });
        }

        Ok(())
    }

    fn assert_not_exist(&self, id: &VersionedSubstateId) -> Result<(), SubstateStoreError> {
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            if change.is_up() {
                return Err(SubstateStoreError::ExpectedSubstateNotExist { id: id.clone() });
            }
            return Ok(());
        }

        if SubstateRecord::exists(self.read_transaction(), id)? {
            return Err(SubstateStoreError::ExpectedSubstateNotExist { id: id.clone() });
        }

        Ok(())
    }

    pub fn new_locks(&self) -> &IndexMap<SubstateId, Vec<SubstateLock>> {
        &self.new_locks
    }

    pub fn diff(&self) -> &Vec<SubstateChange> {
        &self.diff
    }

    pub fn into_parts(self) -> (Vec<SubstateChange>, IndexMap<SubstateId, Vec<SubstateLock>>) {
        (self.diff, self.new_locks)
    }
}

#[derive(Debug, Default)]
pub struct LockStatus {
    lock_failures: Vec<LockFailedError>,
    hard_conflict_idx: Option<usize>,
}

impl LockStatus {
    pub fn new() -> Self {
        Default::default()
    }

    pub(self) fn add_failed(&mut self, err: LockFailedError) -> usize {
        let index = self.lock_failures.len();
        self.lock_failures.push(err);
        index
    }

    /// Returns true if any of the lock requests failed. If not a hard conflict (see [LockStatus::hard_conflict]), the
    /// transaction may be proposed later once the lock is released.
    pub fn is_any_failed(&self) -> bool {
        !self.lock_failures.is_empty()
    }

    /// Returns the error message if there is a hard conflict. A hard conflict occurs when a VERSIONED substate lock is
    /// requested and fails leading to the transaction to be ABORTED.
    pub fn hard_conflict(&self) -> Option<&LockFailedError> {
        self.hard_conflict_idx.map(|idx| &self.lock_failures[idx])
    }

    pub fn failures(&self) -> &[LockFailedError] {
        &self.lock_failures
    }

    pub fn is_hard_conflict(&self) -> bool {
        self.hard_conflict_idx.is_some()
    }
}
