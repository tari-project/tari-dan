//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fmt::Display,
    iter,
};

use indexmap::IndexMap;
use log::*;
use tari_consensus_types::LeafBlock;
use tari_engine_types::{
    published_template::{PublishedTemplate, PublishedTemplateAddress},
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
};
use tari_ootle_common_types::{
    LockIntent,
    NumPreshards,
    SubstateAddress,
    SubstateLockType,
    SubstateRequirement,
    SubstateVersion,
    ToSubstateAddress,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    displayable::Displayable,
    optional::Optional,
    shard::Shard,
    substate_type::SubstateType,
};
use tari_ootle_storage::{
    PendingChain,
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{BlockDiff, LockConflict, SubstateChange, SubstateLock, SubstateRecord},
};
use tari_ootle_transaction::TransactionId;
use tari_template_lib_types::TemplateAddress;

use super::error::SubstateStoreError;
use crate::{
    hotstuff::substate_store::LockFailedError,
    traits::{BlockTransactionExecutorError, ParentChainTemplates, ReadableSubstateStore, WriteableSubstateStore},
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
    /// Encoded bytes of the substates this block puts up and the ids of those it puts down, per shard.
    /// Each shard's total is what the block adds to that shard's next state version.
    shard_output_bytes: HashMap<Shard, usize>,
    /// Input substates this block pledges to foreign shard groups. Every foreign shard group's pledge from this block
    /// is a subset of these.
    pledged_inputs: HashSet<VersionedSubstateId>,
    /// Encoded bytes of `pledged_inputs`, as they travel in a block pledge.
    pledge_bytes: usize,
    /// The most bytes [`Self::pledge_inputs`] lets this block pledge, if it is being proposed.
    max_pledge_bytes: Option<usize>,
    new_locks: IndexMap<SubstateId, Vec<SubstateLock>>,
    parent_block: LeafBlock,
    /// Read once for every branch-scoped read at `parent_block`. `store` is borrowed shared for as long as this lives,
    /// so nothing can write to the pending chain while the store is in use.
    parent_chain: PendingChain,
    num_preshards: NumPreshards,
}

impl<'a, TTx: StateStoreReadTransaction> PendingSubstateStore<'a, TTx> {
    pub fn init(store: &'a TTx, parent_block: LeafBlock, num_preshards: NumPreshards) -> Result<Self, StorageError> {
        let parent_chain = store.pending_chain_get(parent_block.block_id())?;
        Ok(Self {
            store,
            pending: HashMap::new(),
            head: HashMap::new(),
            changes: Vec::new(),
            shard_output_bytes: HashMap::new(),
            pledged_inputs: HashSet::new(),
            pledge_bytes: 0,
            max_pledge_bytes: None,
            new_locks: IndexMap::new(),
            parent_block,
            parent_chain,
            num_preshards,
        })
    }

    /// Makes [`Self::pledge_inputs`] refuse inputs that would take the block's pledge past `max_bytes`. A proposer
    /// sets this so it defers what does not fit; a validator leaves it unset and checks [`Self::pledge_bytes`] once
    /// the whole block is evaluated.
    pub fn with_pledge_budget(mut self, max_bytes: usize) -> Self {
        self.max_pledge_bytes = Some(max_bytes);
        self
    }

    /// The block this store builds on.
    pub fn parent_block(&self) -> LeafBlock {
        self.parent_block
    }

    /// The pending chain ending at the block this store builds on.
    pub fn parent_chain(&self) -> &PendingChain {
        &self.parent_chain
    }

    pub fn read_transaction(&self) -> &'a TTx {
        self.store
    }

    /// The substate as the chain this store builds on holds it, leaving out this block's own changes.
    fn get_from_parent_chain(&self, id: VersionedSubstateIdRef<'_>) -> Result<Option<Substate>, SubstateStoreError> {
        if let Some(change) =
            BlockDiff::get_for_versioned_substate(self.read_transaction(), &self.parent_chain, id).optional()?
        {
            return Ok(change.into_up());
        }

        let Some(substate) = SubstateRecord::get(self.read_transaction(), &id.to_substate_address()).optional()? else {
            return Ok(None);
        };
        if substate.is_destroyed() {
            return Ok(None);
        }
        substate
            .into_substate()
            .map(Some)
            .ok_or_else(|| up_substate_without_value(id.to_owned()))
    }

    fn get_latest_change_from_store(&self, id: &SubstateId) -> Result<SubstateChange, SubstateStoreError> {
        if let Some(change) =
            BlockDiff::get_last_change_for_substate(self.read_transaction(), &self.parent_chain, id).optional()?
        {
            return Ok(change);
        }

        let substate = SubstateRecord::get_latest(self.read_transaction(), id)
            .optional()?
            .ok_or_else(|| SubstateStoreError::SubstateNotFound {
                id: VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO),
            })?;
        if substate.is_destroyed() {
            return Ok(SubstateChange::Down {
                id: VersionedSubstateId::new(id.clone(), substate.version()),
                shard: substate.shard(),
            });
        }
        let shard = substate.shard();
        let version = substate.version();
        let substate = substate
            .into_substate()
            .ok_or_else(|| up_substate_without_value(VersionedSubstateId::new(id.clone(), version)))?;
        Ok(SubstateChange::Up {
            id: id.clone(),
            shard,
            substate: Box::new(substate),
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
            let id = VersionedSubstateIdRef::new(substate_id, SubstateVersion::ZERO);
            let up = SubstateChange::Up {
                shard: id.to_shard(num_preshards),
                substate: Box::new(Substate::new(SubstateVersion::ZERO, value)),
                id: substate_id.clone(),
            };
            self.insert(up);
            return Ok(());
        };
        match change {
            SubstateChange::Up {
                substate, id, shard, ..
            } => {
                let version = substate.version();
                let next_version = version
                    .checked_next()
                    .ok_or_else(|| SubstateStoreError::InvariantError {
                        details: format!("Substate {id} version {version} cannot be incremented"),
                    })?;
                self.insert(SubstateChange::Down {
                    id: VersionedSubstateId::new(id.clone(), version),
                    shard,
                });
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
                let next_version = id
                    .version()
                    .checked_next()
                    .ok_or_else(|| SubstateStoreError::InvariantError {
                        details: format!("Substate {id} cannot be incremented"),
                    })?;
                let next_id = VersionedSubstateId::new(id.into_substate_id(), next_version);
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
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            return change
                .up_substate()
                .cloned()
                .ok_or_else(|| SubstateStoreError::SubstateNotFound {
                    id: change.versioned_substate_id().to_owned(),
                });
        }

        self.get_from_parent_chain(id)?
            .ok_or_else(|| SubstateStoreError::SubstateNotFound { id: id.to_owned() })
    }
}

impl<TTx: StateStoreReadTransaction> ParentChainTemplates for PendingSubstateStore<'_, TTx> {
    fn get_published_template(
        &self,
        address: &TemplateAddress,
    ) -> Result<Option<PublishedTemplate>, BlockTransactionExecutorError> {
        let id = SubstateId::Template(PublishedTemplateAddress::from_template_address(*address));
        let Some(substate) = self.get_from_parent_chain(VersionedSubstateIdRef::new(&id, SubstateVersion::ZERO))?
        else {
            return Ok(None);
        };
        let template =
            substate
                .into_substate_value()
                .into_template()
                .ok_or_else(|| SubstateStoreError::InvariantError {
                    details: format!("Template substate {id} does not hold a published template"),
                })?;
        Ok(Some(template))
    }
}

fn up_substate_without_value(id: VersionedSubstateId) -> SubstateStoreError {
    SubstateStoreError::InvariantError {
        details: format!("Substate {id} is UP but its value is missing from the store"),
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
            let id = VersionedSubstateId::new(id.clone(), *version);
            let shard = id.to_shard(self.num_preshards);
            debug!(target: LOG_TARGET, "🔽️ Down: {id} {shard}");
            let bytes = tari_bor::encoded_len(id.substate_id());
            self.put(SubstateChange::Down { id, shard })?;
            self.record_shard_output_bytes(shard, bytes);
        }

        for (id, substate) in diff.up_iter() {
            let vid = VersionedSubstateIdRef::new(id, substate.version());
            let shard = vid.to_shard(self.num_preshards);
            debug!(target: LOG_TARGET, "🔼️ Up: {} v{} {}", id, substate.version(), shard);
            self.put(SubstateChange::Up {
                id: id.clone(),
                shard,
                substate: Box::new(substate.clone()),
            })?;
            self.record_shard_output_bytes(shard, up_encoded_len(id, substate));
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
                        id: VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO),
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
            BlockDiff::get_last_change_for_substate(self.read_transaction(), &self.parent_chain, id).optional()?
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
                id: VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO),
            })?;

        Ok(LatestSubstateVersion { version, is_up })
    }

    pub fn exists(&self, id: &SubstateId) -> Result<bool, SubstateStoreError> {
        let latest = self.get_latest_version(id).optional()?;
        Ok(latest.is_some())
    }

    pub fn get_many<I: IntoIterator<Item = (SubstateRequirement, SubstateVersion)> + ExactSizeIterator>(
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
                        err @ LockFailedError::SubstateExists { .. } |
                        err @ LockFailedError::SubstateNotFound { .. } => {
                            // An input with no live version, or an output claiming a version that already exists,
                            // is invalid at every height - no later block can make either succeed
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

    /// Returns the conflicts that [Self::try_lock_all] must report for `transaction_id`'s input locks, whatever lock
    /// type its execution resolves each input to and whether or not it locks under local-only rules. Nothing is
    /// locked. An existing WRITE lock refuses every input lock from another transaction, as does an OUTPUT lock that
    /// is not local-only. A READ lock admits a READ, so it is never reported.
    pub fn check_input_locks<I, L>(
        &self,
        transaction_id: &TransactionId,
        input_locks: I,
    ) -> Result<LockStatus, SubstateStoreError>
    where
        I: IntoIterator<Item = L>,
        L: LockIntent,
    {
        let mut lock_status = LockStatus::new();
        for lock in input_locks {
            let versioned_substate_id = lock.to_versioned_substate_id_ref();
            let Some(existing) = self.get_latest_lock_by_id(versioned_substate_id.substate_id())? else {
                continue;
            };
            if existing.transaction_id() == transaction_id {
                continue;
            }
            let refuses_every_input = match existing.lock_type() {
                SubstateLockType::Write => true,
                SubstateLockType::Output => !existing.is_local_only(),
                SubstateLockType::Read => false,
            };
            if !refuses_every_input {
                continue;
            }

            let index = lock_status.add_failed(LockFailedError::LockConflict {
                substate_id: versioned_substate_id.to_owned(),
                conflict: LockConflict {
                    existing_lock: existing.lock_type(),
                    requested_lock: lock.lock_type(),
                    transaction_id: *existing.transaction_id(),
                    is_local_only: existing.is_local_only(),
                },
            });
            if lock.requested_version().is_some() {
                lock_status.hard_conflict_idx = Some(index);
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

        let Some(existing) = self.get_latest_lock_by_id(versioned_substate_id.substate_id())? else {
            if requested_lock_type.is_input() {
                self.lock_assert_is_up(versioned_substate_id)?;
            }

            self.grant_lock(
                transaction_id,
                versioned_substate_id,
                requested_lock_type,
                is_local_only,
            )?;
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

                self.grant_lock(
                    transaction_id,
                    versioned_substate_id,
                    requested_lock_type,
                    is_local_only,
                )?;
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

                self.grant_lock(
                    transaction_id,
                    versioned_substate_id,
                    SubstateLockType::Output,
                    is_local_only,
                )?;
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

                // WRITE or READ
                self.grant_lock(
                    transaction_id,
                    versioned_substate_id,
                    requested_lock_type,
                    is_local_only,
                )?;
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

    pub fn get_latest_lock_by_id(&self, id: &SubstateId) -> Result<Option<Cow<'_, SubstateLock>>, SubstateStoreError> {
        if let Some(lock) = self.new_locks.get(id).and_then(|locks| locks.last()) {
            return Ok(Some(Cow::Borrowed(lock)));
        }

        let maybe_lock = self
            .read_transaction()
            .substate_locks_get_latest_for_substate_in_chain(&self.parent_chain, id)
            .optional()?;
        Ok(maybe_lock.map(Cow::Owned))
    }

    /// Grants `lock_type` over `id` to `transaction_id`.
    ///
    /// An OUTPUT lock claims a version that the transaction will create, so it is only valid while that version does
    /// not exist. A lock already held over the substate says nothing about the requested version, so the assertion
    /// must run wherever an OUTPUT lock is granted.
    fn grant_lock(
        &mut self,
        transaction_id: TransactionId,
        id: VersionedSubstateIdRef<'_>,
        lock_type: SubstateLockType,
        is_local_only: bool,
    ) -> Result<(), SubstateStoreError> {
        if lock_type.is_output() {
            self.lock_assert_not_exist(id)?;
        }

        self.add_new_lock(
            id.substate_id().clone(),
            SubstateLock::new(transaction_id, id.version(), lock_type, is_local_only),
        );
        Ok(())
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
            BlockDiff::get_for_versioned_substate(self.read_transaction(), &self.parent_chain, id).optional()?
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
            BlockDiff::get_for_versioned_substate(self.read_transaction(), &self.parent_chain, id).optional()?
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

    /// Asserts that the exact version `id` names has never existed in this chain.
    ///
    /// A change recorded against a versioned substate address - UP or DOWN - is proof that the version was created,
    /// and `SubstateRecord`s outlive their destruction, so existence, not liveness, is the test. The three sources are
    /// checked cheapest first: the in-memory diff, then a point lookup, then the branch scan.
    fn lock_assert_not_exist(&self, id: VersionedSubstateIdRef<'_>) -> Result<(), SubstateStoreError> {
        if self.get_pending(&id.to_substate_address()).is_some() {
            return Err(LockFailedError::SubstateExists { id: id.to_owned() }.into());
        }

        if SubstateRecord::exists(self.read_transaction(), id)? {
            return Err(LockFailedError::SubstateExists { id: id.to_owned() }.into());
        }

        if BlockDiff::contains_versioned_substate(self.read_transaction(), &self.parent_chain, id)? {
            return Err(LockFailedError::SubstateExists { id: id.to_owned() }.into());
        }

        Ok(())
    }

    pub fn new_locks(&self) -> &IndexMap<SubstateId, Vec<SubstateLock>> {
        &self.new_locks
    }

    fn record_shard_output_bytes(&mut self, shard: Shard, bytes: usize) {
        let total = self.shard_output_bytes.entry(shard).or_default();
        *total = total.saturating_add(bytes);
    }

    /// The most bytes this block writes to any one shard: substates put up and the ids of those put down.
    pub fn max_shard_output_bytes(&self) -> usize {
        self.shard_output_bytes.values().copied().max().unwrap_or(0)
    }

    /// Whether putting `diff` would take any shard past `max_bytes` written in this block.
    pub fn diff_exceeds_shard_output_bytes(&self, diff: &SubstateDiff, max_bytes: usize) -> bool {
        let downs = diff.down_iter().map(|(id, version)| {
            let shard = VersionedSubstateIdRef::new(id, *version).to_shard(self.num_preshards);
            (shard, tari_bor::encoded_len(id))
        });
        let ups = diff.up_iter().map(|(id, substate)| {
            let shard = VersionedSubstateIdRef::new(id, substate.version()).to_shard(self.num_preshards);
            (shard, up_encoded_len(id, substate))
        });
        let mut totals = HashMap::<Shard, usize>::new();
        downs.chain(ups).any(|(shard, bytes)| {
            let total = totals
                .entry(shard)
                .or_insert_with(|| self.shard_output_bytes.get(&shard).copied().unwrap_or(0));
            *total = total.saturating_add(bytes);
            *total > max_bytes
        })
    }

    /// Adds a transaction's local inputs to the substates this block pledges to foreign shard groups.
    ///
    /// Nothing is added if the inputs alone come to more than `max_transaction_bytes`, or if they would take the
    /// block past its pledge budget (see [`Self::with_pledge_budget`]). An input another command of the block already
    /// pledges is counted once, because a block pledge carries each substate once.
    pub fn pledge_inputs<'i, I>(
        &mut self,
        inputs: I,
        max_transaction_bytes: usize,
    ) -> Result<InputPledge, SubstateStoreError>
    where
        I: IntoIterator<Item = VersionedSubstateIdRef<'i>>,
    {
        let mut transaction_bytes = 0usize;
        let mut new_inputs = HashSet::new();
        let mut new_bytes = 0usize;
        for id in inputs {
            let substate = self.get(id)?;
            let bytes = up_encoded_len(id.substate_id(), &substate);
            transaction_bytes = transaction_bytes.saturating_add(bytes);
            let id = id.to_owned();
            if !self.pledged_inputs.contains(&id) && new_inputs.insert(id) {
                new_bytes = new_bytes.saturating_add(bytes);
            }
        }

        if transaction_bytes > max_transaction_bytes {
            return Ok(InputPledge::TransactionTooLarge {
                bytes: transaction_bytes,
            });
        }
        let total_bytes = self.pledge_bytes.saturating_add(new_bytes);
        if let Some(max_bytes) = self.max_pledge_bytes &&
            total_bytes > max_bytes
        {
            return Ok(InputPledge::BlockBudgetReached { total_bytes, max_bytes });
        }

        self.pledged_inputs.extend(new_inputs);
        self.pledge_bytes = total_bytes;
        Ok(InputPledge::Pledged)
    }

    /// The encoded bytes of the input substates this block pledges to foreign shard groups.
    pub fn pledge_bytes(&self) -> usize {
        self.pledge_bytes
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

/// The outcome of [`PendingSubstateStore::pledge_inputs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPledge {
    Pledged,
    /// The transaction's inputs alone are more than one block may pledge.
    TransactionTooLarge {
        bytes: usize,
    },
    /// The inputs fit an empty block but not alongside what this block already pledges.
    BlockBudgetReached {
        total_bytes: usize,
        max_bytes: usize,
    },
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

    /// Returns true if locking failed only in ways that may succeed once the conflicting locks are released. The
    /// proposer defers such a transaction to a later block rather than aborting it.
    pub fn is_deferrable_conflict(&self) -> bool {
        self.is_any_failed() && !self.is_hard_conflict()
    }
}

#[derive(Debug, Clone)]
pub struct LatestSubstateVersion {
    version: SubstateVersion,
    is_up: bool,
}

impl LatestSubstateVersion {
    pub fn is_down(&self) -> bool {
        !self.is_up
    }

    pub fn is_up(&self) -> bool {
        self.is_up
    }

    pub fn version(&self) -> SubstateVersion {
        self.version
    }
}

fn up_encoded_len(id: &SubstateId, substate: &Substate) -> usize {
    tari_bor::encoded_len(id).saturating_add(tari_bor::encoded_len(substate))
}
