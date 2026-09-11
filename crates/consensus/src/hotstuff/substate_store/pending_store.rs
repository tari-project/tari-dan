//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Cow, collections::HashMap, fmt::Display, iter};

use indexmap::IndexMap;
use log::*;
use tari_consensus_types::LeafBlock;
use tari_engine_types::substate::{Substate, SubstateDiff, SubstateId, SubstateValue};
use tari_ootle_common_types::{
    LockIntent,
    NumPreshards,
    SubstateAddress,
    SubstateLockType,
    SubstateRequirement,
    ToSubstateAddress,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    displayable::Displayable,
    optional::Optional,
    substate_type::SubstateType,
};
use tari_ootle_storage::{
    StateStoreReadTransaction,
    consensus_models::{BlockDiff, LockConflict, SubstateChange, SubstateLock, SubstateRecord},
};
use tari_ootle_transaction::TransactionId;

use super::error::SubstateStoreError;
use crate::{
    hotstuff::substate_store::LockFailedError,
    traits::{ReadableSubstateStore, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::ootle::hotstuff::substate_store::pending_store";

pub struct PendingSubstateStore<'store, TTx: StateStoreReadTransaction> {
    store: &'store TTx,
    /// Map from substate address to the index in the diff list of the corresponding change
    pending: HashMap<SubstateAddress, usize>,
    /// Map from substate id to the index in the diff list of the latest change
    head: HashMap<SubstateId, usize>,
    /// Append only list of changes ordered oldest to newest
    changes: Vec<SubstateChange>,
    new_locks: IndexMap<SubstateId, Vec<SubstateLock>>,
    parent_block: LeafBlock,
    num_preshards: NumPreshards,
}

impl<'a, TTx: StateStoreReadTransaction> PendingSubstateStore<'a, TTx> {
    pub fn new(store: &'a TTx, parent_block: LeafBlock, num_preshards: NumPreshards) -> Self {
        Self {
            store,
            pending: HashMap::new(),
            head: HashMap::new(),
            changes: Vec::new(),
            new_locks: IndexMap::new(),
            parent_block,
            num_preshards,
        }
    }

    pub fn read_transaction(&self) -> &'a TTx {
        self.store
    }

    fn get_latest_change_from_store(&self, id: &SubstateId) -> Result<SubstateChange, SubstateStoreError> {
        if let Some(change) =
            BlockDiff::get_last_change_for_substate(self.read_transaction(), self.parent_block.block_id(), id)
                .optional()?
        {
            return Ok(change);
        }

        let substate = SubstateRecord::get_latest(self.read_transaction(), id)
            .optional()?
            .ok_or_else(|| SubstateStoreError::SubstateNotFound {
                id: VersionedSubstateId::new(id.clone(), 0),
            })?;
        if substate.is_destroyed() {
            return Ok(SubstateChange::Down {
                id: VersionedSubstateId::new(id.clone(), substate.version()),
                shard: substate.shard(),
            });
        }
        Ok(SubstateChange::Up {
            id: id.clone(),
            shard: substate.shard(),
            substate: Box::new(
                substate
                    .into_substate()
                    .expect("PendingSubstateStore::get_latest_change: UP substate has no value"),
            ),
        })
    }

    pub(crate) fn update_in_place<TErr, FUpdate, FCreate>(
        &mut self,
        substate_id: &SubstateId,
        updater: FUpdate,
        creator: FCreate,
    ) -> Result<(), TErr>
    where
        TErr: From<SubstateStoreError>,
        FUpdate: FnOnce(&mut SubstateValue) -> Result<(), TErr>,
        FCreate: FnOnce(Option<VersionedSubstateIdRef<'_>>) -> Result<SubstateValue, TErr>,
    {
        let num_preshards = self.num_preshards;
        if let Some(head_change_mut) = self.get_head_change_mut(substate_id) {
            match head_change_mut {
                SubstateChange::Up { substate, .. } => {
                    debug!(target: LOG_TARGET, "Updating substate in place: {} v{}", substate_id, substate.version());
                    return updater(substate.substate_value_mut());
                },
                SubstateChange::Down { id, .. } => {
                    debug!(target: LOG_TARGET, "Creating substate in place: {}", id);
                    let value = creator(Some(id.as_versioned_ref()))?;
                    let next_id = id.to_next_version();
                    let up = SubstateChange::Up {
                        shard: id.to_shard(num_preshards),
                        substate: Box::new(Substate::new(next_id.version(), value)),
                        id: next_id.into_substate_id(),
                    };
                    self.insert(up);
                },
            }
            return Ok(());
        }

        let Some(change) = self.get_latest_change_from_store(substate_id).optional()? else {
            debug!(target: LOG_TARGET, "Creating substate in place: {} v0", substate_id);
            let value = creator(None)?;
            let id = VersionedSubstateIdRef::new(substate_id, 0);
            let up = SubstateChange::Up {
                shard: id.to_shard(num_preshards),
                substate: Box::new(Substate::new(0, value)),
                id: substate_id.clone(),
            };
            self.insert(up);
            return Ok(());
        };
        match change {
            SubstateChange::Up {
                substate, id, shard, ..
            } => {
                self.insert(SubstateChange::Down {
                    id: VersionedSubstateId::new(id.clone(), substate.version()),
                    shard,
                });
                let version = substate.version();
                let next_version = version + 1;
                let mut substate_value = substate.into_substate_value();
                debug!(
                    target: LOG_TARGET,
                    "Updating substate in place: {} v{} -> v{}",
                    id,
                    version,
                    next_version
                );
                updater(&mut substate_value)?;
                let substate = Substate::new(next_version, substate_value);
                self.insert(SubstateChange::Up {
                    id,
                    shard,
                    substate: Box::new(substate),
                });
            },
            SubstateChange::Down { id, .. } => {
                debug!(target: LOG_TARGET, "Re-creating DOWNed substate in place: {}", id);
                let value = creator(Some(id.as_versioned_ref()))?;
                let next_id = id.into_next_version();
                let up = SubstateChange::Up {
                    shard: next_id.to_shard(self.num_preshards),
                    substate: Box::new(Substate::new(next_id.version(), value)),
                    id: next_id.into_substate_id(),
                };
                self.insert(up);
            },
        }
        Ok(())
    }
}

impl<'store, TTx: StateStoreReadTransaction> ReadableSubstateStore for PendingSubstateStore<'store, TTx> {
    type Error = SubstateStoreError;

    fn get(&self, id: VersionedSubstateIdRef<'_>) -> Result<Substate, Self::Error> {
        let substate_addr = id.to_substate_address();
        if let Some(change) = self.get_pending(&substate_addr) {
            return change
                .up_substate()
                .cloned()
                .ok_or_else(|| SubstateStoreError::SubstateNotFound {
                    id: change.versioned_substate_id().to_owned(),
                });
        }

        if let Some(change) =
            BlockDiff::get_for_versioned_substate(self.read_transaction(), self.parent_block.block_id(), id)
                .optional()?
        {
            return change
                .into_up()
                .ok_or_else(|| SubstateStoreError::SubstateNotFound { id: id.to_owned() });
        }

        let Some(substate) = SubstateRecord::get(self.read_transaction(), &substate_addr).optional()? else {
            return Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() });
        };
        if substate.is_destroyed() {
            return Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() });
        }
        Ok(substate
            .into_substate()
            .expect("PendingSubstateStore::get UP substate has no value"))
    }
}

impl<'a, TTx: StateStoreReadTransaction> WriteableSubstateStore for PendingSubstateStore<'a, TTx> {
    fn put(&mut self, change: SubstateChange) -> Result<(), Self::Error> {
        match &change {
            SubstateChange::Up { id, substate, .. } => {
                if let Some(v) = substate.previous_version() {
                    self.assert_is_down(VersionedSubstateIdRef::new(id, v))?;
                }
            },
            SubstateChange::Down { id, .. } => {
                self.assert_is_up(id.as_versioned_ref())?;
            },
        }

        self.insert(change);

        Ok(())
    }

    fn put_diff(&mut self, diff: &SubstateDiff) -> Result<(), Self::Error> {
        for (id, version) in diff.down_iter() {
            // Handled by fee withdrawals below
            if id.is_validator_fee_pool() {
                continue;
            }
            let id = VersionedSubstateId::new(id.clone(), *version);
            let shard = id.to_shard(self.num_preshards);
            debug!(target: LOG_TARGET, "🔽️ Down: {id} {shard}");
            self.put(SubstateChange::Down { id, shard })?;
        }

        for (id, substate) in diff.up_iter() {
            // Handled by fee withdrawals below
            if id.is_validator_fee_pool() {
                continue;
            }
            let vid = VersionedSubstateIdRef::new(id, substate.version());
            let shard = vid.to_shard(self.num_preshards);
            debug!(target: LOG_TARGET, "🔼️ Up: {} v{} {}", id, substate.version(), shard);
            self.put(SubstateChange::Up {
                id: id.clone(),
                shard,
                substate: Box::new(substate.clone()),
            })?;
        }

        for withdraw in diff.validator_fee_withdrawals() {
            let id = withdraw.address.into();
            self.update_in_place(
                &id,
                |value_mut| {
                    let substate_type = SubstateType::from(&*value_mut);
                    let fee_pool =
                        value_mut
                            .as_validator_fee_pool_mut()
                            .ok_or_else(|| SubstateStoreError::InvariantError {
                                details: format!(
                                    "Expected substate {id} to be a ValidatorFeePool but was {substate_type}",
                                ),
                            })?;
                    if !fee_pool.withdraw_direct(withdraw.amount) {
                        return Err(SubstateStoreError::InvariantError {
                            details: format!(
                                "Insufficient balance to withdraw {} from validator fee pool {} (balance: {})",
                                withdraw.amount,
                                withdraw.address,
                                fee_pool.amount()
                            ),
                        });
                    }
                    Ok(())
                },
                // Without a live version there is nothing to withdraw from
                |maybe_id| match maybe_id {
                    Some(id) => Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() }),
                    None => Err(SubstateStoreError::SubstateNotFound {
                        id: VersionedSubstateId::new(id.clone(), 0),
                    }),
                },
            )?;
        }

        Ok(())
    }
}

impl<'store, TTx: StateStoreReadTransaction> PendingSubstateStore<'store, TTx> {
    pub fn get_latest_version(&self, id: &SubstateId) -> Result<LatestSubstateVersion, SubstateStoreError> {
        if let Some(ch) = self
            .head
            .get(id)
            .map(|&pos| self.changes.get(pos).expect("diff and head are not in sync"))
        {
            return Ok(LatestSubstateVersion {
                version: ch.versioned_substate_id().version(),
                is_up: ch.is_up(),
            });
        }

        if let Some(change) =
            BlockDiff::get_last_change_for_substate(self.read_transaction(), self.parent_block.block_id(), id)
                .optional()?
        {
            let version = change.versioned_substate_id().version();
            return Ok(LatestSubstateVersion {
                version,
                is_up: change.is_up(),
            });
        }

        let (version, is_up) = SubstateRecord::get_latest_version(self.read_transaction(), id)
            .optional()?
            .ok_or_else(|| SubstateStoreError::SubstateNotFound {
                id: VersionedSubstateId::new(id.clone(), 0),
            })?;

        Ok(LatestSubstateVersion { version, is_up })
    }

    pub fn exists(&self, id: &SubstateId) -> Result<bool, SubstateStoreError> {
        let latest = self.get_latest_version(id).optional()?;
        Ok(latest.is_some())
    }

    pub fn get_many<I: IntoIterator<Item = (SubstateRequirement, u64)> + ExactSizeIterator>(
        &self,
        ids: I,
    ) -> Result<HashMap<SubstateRequirement, Substate>, SubstateStoreError> {
        let mut substates = HashMap::with_capacity(ids.len());
        for (req, version) in ids {
            let substate = self.get(VersionedSubstateIdRef::new(req.substate_id(), version))?;
            substates.insert(req, substate);
        }

        Ok(substates)
    }

    fn get_head_change(&self, id: &SubstateId) -> Option<&SubstateChange> {
        self.head
            .get(id)
            .map(|&pos| self.changes.get(pos).expect("diff and head are not in sync"))
    }

    fn get_head_change_mut(&mut self, id: &SubstateId) -> Option<&mut SubstateChange> {
        self.head
            .get(id)
            .map(|&pos| self.changes.get_mut(pos).expect("diff and head are not in sync"))
    }

    pub fn get_latest_change(&self, id: &SubstateId) -> Result<SubstateChange, SubstateStoreError> {
        if let Some(ch) = self.get_head_change(id) {
            return Ok(ch.clone());
        }

        self.get_latest_change_from_store(id)
    }

    pub fn has_any_conflicting_pledges<'a, I>(
        &self,
        transaction_id: &TransactionId,
        substate_ids: I,
    ) -> Result<bool, SubstateStoreError>
    where
        I: IntoIterator<Item = &'a SubstateId>,
    {
        // TODO(perf): consider optimizing
        let existing = self
            .store
            .foreign_substate_pledges_get_write_pledges_to_transaction(transaction_id, substate_ids)?;

        if existing.is_empty() {
            return Ok(false);
        }

        warn!(
            target: LOG_TARGET,
            "🔒️ Conflicting foreign pledges found for transaction {}: {}",
            transaction_id,
            existing.display()
        );

        Ok(true)
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
                        err @ LockFailedError::SubstateIsUp { .. } | err @ LockFailedError::SubstateNotFound { .. } => {
                            // If the substate does not exist or is not UP (unversioned: previously DOWNed and never
                            // UPed), the transaction is invalid
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
        debug!(
            target: LOG_TARGET,
            "🔒️ Requested substate lock: {}",
            requested_lock
        );

        let versioned_substate_id = requested_lock.to_versioned_substate_id_ref();

        let Some(existing) = self.get_latest_lock_by_id(&self.parent_block, versioned_substate_id.substate_id())?
        else {
            if requested_lock_type.is_output() {
                self.lock_assert_not_exist(versioned_substate_id)?;
            } else {
                self.lock_assert_is_up(versioned_substate_id)?;
            }

            let version = versioned_substate_id.version();
            self.add_new_lock(
                versioned_substate_id.substate_id().clone(),
                SubstateLock::new(transaction_id, version, requested_lock_type, is_local_only),
            );
            return Ok(());
        };

        // Local-Only-Rules apply if: current lock is local-only AND requested lock is local only
        let has_local_only_rules = existing.is_local_only() && is_local_only;
        let same_transaction = *existing.transaction_id() == transaction_id;
        let same_transaction_lock = same_transaction && existing.lock_type() == requested_lock_type;

        debug!(
            target: LOG_TARGET,
            "🔒️ Found existing lock: {} {}",
            versioned_substate_id.substate_id(),
            existing
        );

        // Duplicate lock requests on the same transaction are idempotent
        if same_transaction_lock {
            debug!(
                target: LOG_TARGET,
                "🔒️ Duplicate lock request: {}",
                requested_lock
            );
            return Ok(());
        }

        match existing.lock_type() {
            // If a substate is already locked as READ:
            // - it MAY be locked as READ
            // - if Local-Only-Rules:
            //   - it MAY be locked as READ or OUTPUT.
            SubstateLockType::Read => {
                // Cannot write to or create an output for a substate that is already read locked
                if has_local_only_rules && requested_lock_type.is_write() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict(1): [{}] Read lock(local_only={}) is present in {}. Requested lock is {}(local_only={})",
                        versioned_substate_id,
                        existing.is_local_only(),
                        existing.transaction_id(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id.to_owned(),
                        conflict: LockConflict {
                            existing_lock: existing.lock_type(),
                            requested_lock: requested_lock_type,
                            transaction_id: *existing.transaction_id(),
                            is_local_only: has_local_only_rules,
                        },
                    }
                    .into());
                }

                if !has_local_only_rules && !same_transaction && !requested_lock_type.is_read() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict(2): [{}] Read lock(local_only={}) is present in {}. Requested lock is {}(local_only={})",
                        versioned_substate_id,
                        existing.is_local_only(),
                        existing.transaction_id(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id.to_owned(),
                        conflict: LockConflict {
                            existing_lock: existing.lock_type(),
                            requested_lock: requested_lock_type,
                            transaction_id: *existing.transaction_id(),
                            is_local_only: has_local_only_rules,
                        },
                    }
                    .into());
                }

                if !has_local_only_rules && same_transaction && !requested_lock_type.is_output() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict(3): [{}] Read lock(local_only={}) is present in {}. Requested lock is {}(local_only={})",
                        versioned_substate_id,
                        existing.is_local_only(),
                        existing.transaction_id(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id.to_owned(),
                        conflict: LockConflict {
                            existing_lock: existing.lock_type(),
                            requested_lock: requested_lock_type,
                            transaction_id: *existing.transaction_id(),
                            is_local_only: has_local_only_rules,
                        },
                    }
                    .into());
                }

                let version = versioned_substate_id.version();
                self.add_new_lock(
                    versioned_substate_id.substate_id().clone(),
                    SubstateLock::new(transaction_id, version, requested_lock_type, is_local_only),
                );
            },

            // If a substate is already locked as WRITE:
            // - it MUST NOT be locked as READ, WRITE
            // - if Same-Transaction OR Local-Only-Rules:
            //   - it MAY be locked as OUTPUT
            SubstateLockType::Write => {
                // Cannot lock a non-local-only WRITE locked substate
                if !same_transaction && !has_local_only_rules {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Write lock(local_only={}, tx={}) is present. Requested lock is {}(local_only={}, tx={})",
                        versioned_substate_id,
                        existing.is_local_only(),
                        existing.transaction_id(),
                        requested_lock_type,
                        is_local_only,
                        transaction_id,
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id.to_owned(),
                        conflict: LockConflict {
                            existing_lock: existing.lock_type(),
                            requested_lock: requested_lock_type,
                            transaction_id: *existing.transaction_id(),
                            is_local_only: false,
                        },
                    }
                    .into());
                }

                if !requested_lock_type.is_output() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}] Write lock(local_only={}) is present in {}. Requested lock is {}(local_only={})",
                        versioned_substate_id,
                        existing.is_local_only(),
                        existing.transaction_id(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id.to_owned(),
                        conflict: LockConflict {
                            existing_lock: existing.lock_type(),
                            requested_lock: requested_lock_type,
                            transaction_id: *existing.transaction_id(),
                            is_local_only: has_local_only_rules,
                        },
                    }
                    .into());
                }

                let version = versioned_substate_id.version();
                self.add_new_lock(
                    versioned_substate_id.substate_id().clone(),
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
                        "⚠️ Lock conflict: [{}, {}] Output lock(local_only={}) is present in {}. Requested lock is {}(local_only={})",
                        transaction_id,
                        versioned_substate_id,
                        existing.is_local_only(),
                        existing.transaction_id(),
                        requested_lock_type,
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id.to_owned(),
                        conflict: LockConflict {
                            existing_lock: existing.lock_type(),
                            requested_lock: requested_lock_type,
                            transaction_id: *existing.transaction_id(),
                            is_local_only: has_local_only_rules,
                        },
                    }
                    .into());
                }

                if requested_lock_type.is_output() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Lock conflict: [{}, {}] Output lock(local_only={}) is present in {}. Requested lock is Output(local_only={})",
                        transaction_id,
                        versioned_substate_id,
                        existing.is_local_only(),
                        existing.transaction_id(),
                        is_local_only
                    );
                    return Err(LockFailedError::LockConflict {
                        substate_id: versioned_substate_id.to_owned(),
                        conflict: LockConflict {
                            existing_lock: existing.lock_type(),
                            requested_lock: requested_lock_type,
                            transaction_id: *existing.transaction_id(),
                            is_local_only: has_local_only_rules,
                        },
                    }
                    .into());
                }

                let version = versioned_substate_id.version();
                self.add_new_lock(
                    versioned_substate_id.substate_id().clone(),
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
            .map(|&pos| self.changes.get(pos).expect("pending map and diff are out of sync"))
    }

    fn insert(&mut self, change: SubstateChange) {
        let index = self.changes.len();
        self.pending.insert(change.to_substate_address(), index);
        self.head
            .insert(change.versioned_substate_id().substate_id().clone(), index);
        self.changes.push(change)
    }

    pub fn get_latest_lock_by_id(
        &self,
        block: &LeafBlock,
        id: &SubstateId,
    ) -> Result<Option<Cow<'_, SubstateLock>>, SubstateStoreError> {
        if let Some(lock) = self.new_locks.get(id).and_then(|locks| locks.last()) {
            return Ok(Some(Cow::Borrowed(lock)));
        }

        let maybe_lock = self
            .read_transaction()
            .substate_locks_get_latest_for_substate(block, id)
            .optional()?;
        Ok(maybe_lock.map(Cow::Owned))
    }

    fn add_new_lock(&mut self, substate_id: SubstateId, lock: SubstateLock) {
        debug!(
            target: LOG_TARGET,
            "🔒️ Adding new lock: {} {}",
            substate_id,
            lock
        );
        self.new_locks.entry(substate_id).or_default().push(lock);
    }

    fn assert_is_up(&self, id: VersionedSubstateIdRef<'_>) -> Result<(), SubstateStoreError> {
        trace!(
            target: LOG_TARGET,
            "assert_is_up: id: {}, pending: {}, head: {}",
            id,
            self.pending.display(),
            self.head.display()
        );
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            if change.is_down() {
                return Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() });
            }
            return Ok(());
        }

        trace!(
            target: LOG_TARGET,
            "assert_is_up id: {} not found in pending",
            id,
        );

        if let Some(change) =
            BlockDiff::get_for_versioned_substate(self.read_transaction(), self.parent_block.block_id(), id)
                .optional()?
        {
            if change.is_up() {
                return Ok(());
            }
            return Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() });
        }

        trace!(
            target: LOG_TARGET,
            "assert_is_up: id: {} not found in block diff",
            id,
        );

        match SubstateRecord::substate_is_up(self.read_transaction(), &id.to_substate_address()).optional()? {
            Some(true) => Ok(()),
            Some(false) | None => Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() }),
        }
    }

    pub fn lock_assert_is_up(&self, id: VersionedSubstateIdRef<'_>) -> Result<(), SubstateStoreError> {
        match self.assert_is_up(id) {
            Ok(_) => Ok(()),
            // Converts a substate store error to a LockFailedError (TODO: improve)
            Err(SubstateStoreError::SubstateNotFound { id }) => Err(LockFailedError::SubstateNotFound { id }.into()),
            Err(err) => Err(err),
        }
    }

    fn assert_is_down(&self, id: VersionedSubstateIdRef<'_>) -> Result<(), SubstateStoreError> {
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            if change.is_up() {
                return Err(SubstateStoreError::ExpectedSubstateDown { id: id.to_owned() });
            }
            return Ok(());
        }

        if let Some(change) =
            BlockDiff::get_for_versioned_substate(self.read_transaction(), self.parent_block.block_id(), id)
                .optional()?
        {
            if change.is_up() {
                return Err(SubstateStoreError::ExpectedSubstateDown { id: id.to_owned() });
            }
            return Ok(());
        }

        let address = id.to_substate_address();
        let Some(is_up) = SubstateRecord::substate_is_up(self.read_transaction(), &address).optional()? else {
            debug!(target: LOG_TARGET, "Expected substate {} to be DOWN but it does not exist", address);
            return Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() });
        };
        if is_up {
            return Err(SubstateStoreError::ExpectedSubstateDown { id: id.to_owned() });
        }

        Ok(())
    }

    fn lock_assert_not_exist(&self, id: VersionedSubstateIdRef<'_>) -> Result<(), SubstateStoreError> {
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            if change.is_up() {
                return Err(SubstateStoreError::LockFailed(LockFailedError::SubstateIsUp {
                    id: id.to_owned(),
                }));
            }
            return Ok(());
        }

        if let Some(change) =
            BlockDiff::get_for_versioned_substate(self.read_transaction(), self.parent_block.block_id(), id)
                .optional()?
        {
            if change.is_up() {
                return Err(SubstateStoreError::LockFailed(LockFailedError::SubstateIsUp {
                    id: id.to_owned(),
                }));
            }
            return Ok(());
        }

        if SubstateRecord::exists(self.read_transaction(), id)? {
            return Err(SubstateStoreError::LockFailed(LockFailedError::SubstateIsUp {
                id: id.to_owned(),
            }));
        }

        Ok(())
    }

    pub fn new_locks(&self) -> &IndexMap<SubstateId, Vec<SubstateLock>> {
        &self.new_locks
    }

    pub fn changes(&self) -> &Vec<SubstateChange> {
        &self.changes
    }

    pub fn generate_shard_state_change_bitmap(&self) -> Vec<bool> {
        let shard_group = self.parent_block.shard_group();
        let mut bitmap = iter::repeat_n(false, shard_group.len() + 1).collect::<Vec<_>>();
        debug!(
            target: LOG_TARGET,
            "Generating shard state change bitmap (len:{})for shard group {}",
            bitmap.len(),
            shard_group,
        );
        for ch in &self.changes {
            let shard = ch.shard();
            if !shard_group.contains_or_global(&shard) {
                continue;
            }
            if shard.is_global() {
                bitmap[0] = true;
                continue;
            }

            let index = shard
                .as_u32()
                .checked_sub(shard_group.start().as_u32())
                // All values are previously validated
                .expect("BUG(to_partial_shard_state_versions): Shard less than shard group start")
                as usize +
                1;
            assert!(
                index < bitmap.len(),
                "BUG(to_partial_shard_state_versions): (index: {index}, shard: {shard}, shard group: {shard_group})"
            );
            bitmap[index] = true;
        }
        bitmap
    }

    pub fn into_parts(self) -> (Vec<SubstateChange>, IndexMap<SubstateId, Vec<SubstateLock>>) {
        (self.changes, self.new_locks)
    }
}

#[derive(Debug, Default)]
pub struct LockStatus {
    lock_failures: Vec<LockFailedError>,
    hard_conflict_idx: Option<usize>,
    conflicts: Vec<LockConflict>,
}

impl LockStatus {
    pub const fn new() -> Self {
        Self {
            lock_failures: Vec::new(),
            hard_conflict_idx: None,
            conflicts: Vec::new(),
        }
    }

    fn add_conflict(&mut self, lock_conflict: LockConflict) {
        self.conflicts.push(lock_conflict);
    }

    pub(self) fn add_failed(&mut self, err: LockFailedError) -> usize {
        if let Some(conflict) = err.lock_conflict() {
            self.add_conflict(*conflict);
        }

        let index = self.lock_failures.len();
        self.lock_failures.push(err);
        index
    }

    pub fn conflicts(&self) -> &[LockConflict] {
        &self.conflicts
    }

    pub fn into_lock_conflicts(self) -> Vec<LockConflict> {
        self.conflicts
    }

    /// Returns true if any of the lock requests failed. If not a hard conflict (see [LockStatus::hard_conflict]), the
    /// transaction may be proposed later once the lock is released.
    pub fn is_any_failed(&self) -> bool {
        !self.lock_failures.is_empty()
    }

    /// Returns the error message if there is a hard conflict. A hard conflict occurs when a VERSIONED substate lock is
    /// requested and fails leading to the transaction to be ABORTED.
    pub fn hard_conflict(&self) -> Option<&LockFailedError> {
        self.hard_conflict_idx.map(|idx| {
            self.lock_failures
                .get(idx)
                .expect("hard conflict index is out of bounds")
        })
    }

    pub fn failures(&self) -> &[LockFailedError] {
        &self.lock_failures
    }

    pub fn is_hard_conflict(&self) -> bool {
        self.hard_conflict_idx.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct LatestSubstateVersion {
    version: u64,
    is_up: bool,
}

impl LatestSubstateVersion {
    pub fn is_down(&self) -> bool {
        !self.is_up
    }

    pub fn is_up(&self) -> bool {
        self.is_up
    }

    pub fn version(&self) -> u64 {
        self.version
    }
}
