//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use log::*;
use tari_engine_types::substate::{Substate, SubstateDiff, SubstateId, SubstateValue};
use tari_ootle_common_types::{
    NumPreshards,
    SubstateAddress,
    SubstateRequirement,
    SubstateVersion,
    ToSubstateAddress,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    displayable::Displayable,
    optional::Optional,
    substate_type::SubstateType,
};
use tari_ootle_storage::consensus_models::SubstateChange;

use super::{LatestSubstateVersion, LockFailedError, SubstateStoreError};

const LOG_TARGET: &str = "tari::ootle::hotstuff::substate_store::change_log";

/// The state beneath a [`ChangeLog`]: what every lookup falls through to when the log holds no change for the
/// substate.
pub(super) trait SubstateLayer {
    fn num_preshards(&self) -> NumPreshards;

    /// The substate at exactly `id`. Fails with `SubstateNotFound` unless that version is UP.
    fn get_substate(&self, id: VersionedSubstateIdRef<'_>) -> Result<Substate, SubstateStoreError>;

    /// Whether the version `id` names is UP, or `None` if it has never existed.
    fn is_version_up(&self, id: VersionedSubstateIdRef<'_>) -> Result<Option<bool>, SubstateStoreError>;

    /// The most recent change to `id`. Fails with `SubstateNotFound` if no version of it has ever existed.
    fn latest_change(&self, id: &SubstateId) -> Result<SubstateChange, SubstateStoreError>;

    /// The most recent version of `id`. Fails with `SubstateNotFound` if no version of it has ever existed.
    fn latest_version(&self, id: &SubstateId) -> Result<LatestSubstateVersion, SubstateStoreError>;

    /// The most recent change to `id` made by the block being built. An in-place update rewrites it at its version.
    fn block_head_change(&self, id: &SubstateId) -> Option<&SubstateChange>;
}

/// Substate changes in the order they were made, over a [`SubstateLayer`]. Every lookup sees the most recent change
/// first, so each change reads the ones before it.
#[derive(Debug, Clone, Default)]
pub(super) struct ChangeLog {
    /// Map from substate address to the index in `changes` of the corresponding change
    pending: HashMap<SubstateAddress, usize>,
    /// Map from substate id to the index in `changes` of the latest change
    head: HashMap<SubstateId, usize>,
    /// Append only list of changes ordered oldest to newest
    changes: Vec<SubstateChange>,
}

impl ChangeLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn changes(&self) -> &Vec<SubstateChange> {
        &self.changes
    }

    pub fn into_changes(self) -> Vec<SubstateChange> {
        self.changes
    }

    pub fn get_pending(&self, addr: &SubstateAddress) -> Option<&SubstateChange> {
        self.pending
            .get(addr)
            .map(|&pos| self.changes.get(pos).expect("pending map and diff are out of sync"))
    }

    pub fn get_head(&self, id: &SubstateId) -> Option<&SubstateChange> {
        self.head
            .get(id)
            .map(|&pos| self.changes.get(pos).expect("diff and head are not in sync"))
    }

    fn get_head_mut(&mut self, id: &SubstateId) -> Option<&mut SubstateChange> {
        self.head
            .get(id)
            .map(|&pos| self.changes.get_mut(pos).expect("diff and head are not in sync"))
    }

    fn insert(&mut self, change: SubstateChange) {
        let index = self.changes.len();
        self.pending.insert(change.to_substate_address(), index);
        self.head
            .insert(change.versioned_substate_id().substate_id().clone(), index);
        self.changes.push(change)
    }

    pub fn get<B: SubstateLayer>(
        &self,
        base: &B,
        id: VersionedSubstateIdRef<'_>,
    ) -> Result<Substate, SubstateStoreError> {
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            return change
                .up_substate()
                .cloned()
                .ok_or_else(|| SubstateStoreError::SubstateNotFound {
                    id: change.versioned_substate_id().to_owned(),
                });
        }
        base.get_substate(id)
    }

    pub fn get_many<B, I>(&self, base: &B, ids: I) -> Result<HashMap<SubstateRequirement, Substate>, SubstateStoreError>
    where
        B: SubstateLayer,
        I: IntoIterator<Item = (SubstateRequirement, SubstateVersion)> + ExactSizeIterator,
    {
        let mut substates = HashMap::with_capacity(ids.len());
        for (req, version) in ids {
            let substate = self.get(base, VersionedSubstateIdRef::new(req.substate_id(), version))?;
            substates.insert(req, substate);
        }

        Ok(substates)
    }

    pub fn is_version_up<B: SubstateLayer>(
        &self,
        base: &B,
        id: VersionedSubstateIdRef<'_>,
    ) -> Result<Option<bool>, SubstateStoreError> {
        if let Some(change) = self.get_pending(&id.to_substate_address()) {
            return Ok(Some(change.is_up()));
        }
        base.is_version_up(id)
    }

    pub fn latest_change<B: SubstateLayer>(
        &self,
        base: &B,
        id: &SubstateId,
    ) -> Result<SubstateChange, SubstateStoreError> {
        if let Some(change) = self.get_head(id) {
            return Ok(change.clone());
        }
        base.latest_change(id)
    }

    pub fn latest_version<B: SubstateLayer>(
        &self,
        base: &B,
        id: &SubstateId,
    ) -> Result<LatestSubstateVersion, SubstateStoreError> {
        if let Some(change) = self.get_head(id) {
            return Ok(LatestSubstateVersion::new(
                change.versioned_substate_id().version(),
                change.is_up(),
            ));
        }
        base.latest_version(id)
    }

    pub fn assert_is_up<B: SubstateLayer>(
        &self,
        base: &B,
        id: VersionedSubstateIdRef<'_>,
    ) -> Result<(), SubstateStoreError> {
        trace!(
            target: LOG_TARGET,
            "assert_is_up: id: {}, pending: {}, head: {}",
            id,
            self.pending.display(),
            self.head.display()
        );
        match self.is_version_up(base, id)? {
            Some(true) => Ok(()),
            Some(false) | None => Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() }),
        }
    }

    pub fn lock_assert_is_up<B: SubstateLayer>(
        &self,
        base: &B,
        id: VersionedSubstateIdRef<'_>,
    ) -> Result<(), SubstateStoreError> {
        match self.assert_is_up(base, id) {
            Ok(_) => Ok(()),
            // Converts a substate store error to a LockFailedError (TODO: improve)
            Err(SubstateStoreError::SubstateNotFound { id }) => Err(LockFailedError::SubstateNotFound { id }.into()),
            Err(err) => Err(err),
        }
    }

    pub fn assert_is_down<B: SubstateLayer>(
        &self,
        base: &B,
        id: VersionedSubstateIdRef<'_>,
    ) -> Result<(), SubstateStoreError> {
        match self.is_version_up(base, id)? {
            Some(false) => Ok(()),
            Some(true) => Err(SubstateStoreError::ExpectedSubstateDown { id: id.to_owned() }),
            None => {
                debug!(target: LOG_TARGET, "Expected substate {} to be DOWN but it does not exist", id);
                Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() })
            },
        }
    }

    /// Records `change` once the version it replaces is DOWN (an UP) or the version it spends is UP (a DOWN).
    pub fn put<B: SubstateLayer>(&mut self, base: &B, change: SubstateChange) -> Result<(), SubstateStoreError> {
        self.put_then(base, change, |_| {})
    }

    fn put_then<B, F>(&mut self, base: &B, change: SubstateChange, on_put: F) -> Result<(), SubstateStoreError>
    where
        B: SubstateLayer,
        F: FnOnce(&SubstateChange),
    {
        match &change {
            SubstateChange::Up { id, substate, .. } => {
                if let Some(v) = substate.previous_version() {
                    self.assert_is_down(base, VersionedSubstateIdRef::new(id, v))?;
                }
            },
            SubstateChange::Down { id, .. } => {
                self.assert_is_up(base, id.as_versioned_ref())?;
            },
        }

        on_put(&change);
        self.insert(change);
        Ok(())
    }

    /// Puts every DOWN, then every UP, then every validator fee withdrawal in `diff`, calling `on_put` with each
    /// DOWN and UP as it is recorded. A failure part way leaves the changes before it recorded.
    pub fn put_diff<B, F>(&mut self, base: &B, diff: &SubstateDiff, mut on_put: F) -> Result<(), SubstateStoreError>
    where
        B: SubstateLayer,
        F: FnMut(&SubstateChange),
    {
        let num_preshards = base.num_preshards();
        for (id, version) in diff.down_iter() {
            let id = VersionedSubstateId::new(id.clone(), *version);
            let shard = id.to_shard(num_preshards);
            debug!(target: LOG_TARGET, "🔽️ Down: {id} {shard}");
            self.put_then(base, SubstateChange::Down { id, shard }, &mut on_put)?;
        }

        for (id, substate) in diff.up_iter() {
            let vid = VersionedSubstateIdRef::new(id, substate.version());
            let shard = vid.to_shard(num_preshards);
            debug!(target: LOG_TARGET, "🔼️ Up: {} v{} {}", id, substate.version(), shard);
            self.put_then(
                base,
                SubstateChange::Up {
                    id: id.clone(),
                    shard,
                    substate: Box::new(substate.clone()),
                },
                &mut on_put,
            )?;
        }

        for withdraw in diff.validator_fee_withdrawals() {
            let id = withdraw.address.into();
            self.update_in_place(
                base,
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

    /// Applies `updater` to the latest value of `substate_id`, or `creator` if it is DOWN or has never existed.
    ///
    /// A substate the block being built has already changed keeps that change's version: an UP is rewritten in place
    /// and a DOWN is followed by the next version. Otherwise the latest version is put DOWN and its successor UP.
    pub fn update_in_place<B, TErr, FUpdate, FCreate>(
        &mut self,
        base: &B,
        substate_id: &SubstateId,
        updater: FUpdate,
        creator: FCreate,
    ) -> Result<(), TErr>
    where
        B: SubstateLayer,
        TErr: From<SubstateStoreError>,
        FUpdate: FnOnce(&mut SubstateValue) -> Result<(), TErr>,
        FCreate: FnOnce(Option<VersionedSubstateIdRef<'_>>) -> Result<SubstateValue, TErr>,
    {
        let num_preshards = base.num_preshards();
        if let Some(head_change_mut) = self.get_head_mut(substate_id) {
            match head_change_mut {
                SubstateChange::Up { substate, .. } => {
                    debug!(target: LOG_TARGET, "Updating substate in place: {} v{}", substate_id, substate.version());
                    return updater(substate.substate_value_mut());
                },
                SubstateChange::Down { id, .. } => {
                    let next_id = id.to_next_version();
                    debug!(target: LOG_TARGET, "Creating substate in place: {}", next_id);
                    let value = creator(Some(id.as_versioned_ref()))?;
                    self.insert_next_up(next_id, value, num_preshards);
                },
            }
            return Ok(());
        }

        if let Some(head_change) = base.block_head_change(substate_id) {
            match head_change {
                SubstateChange::Up { .. } => {
                    let mut head_change = head_change.clone();
                    let SubstateChange::Up { substate, .. } = &mut head_change else {
                        unreachable!("cloned from an UP change")
                    };
                    debug!(target: LOG_TARGET, "Updating substate in place: {} v{}", substate_id, substate.version());
                    updater(substate.substate_value_mut())?;
                    self.insert(head_change);
                },
                SubstateChange::Down { id, .. } => {
                    let next_id = id.to_next_version();
                    debug!(target: LOG_TARGET, "Creating substate in place: {}", next_id);
                    let value = creator(Some(id.as_versioned_ref()))?;
                    self.insert_next_up(next_id, value, num_preshards);
                },
            }
            return Ok(());
        }

        let Some(change) = base.latest_change(substate_id).optional()? else {
            debug!(target: LOG_TARGET, "Creating substate in place: {} v0", substate_id);
            let value = creator(None)?;
            self.insert_next_up(
                VersionedSubstateId::new(substate_id.clone(), SubstateVersion::ZERO),
                value,
                num_preshards,
            );
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
                self.insert_next_up(
                    VersionedSubstateId::new(id.into_substate_id(), next_version),
                    value,
                    num_preshards,
                );
            },
        }
        Ok(())
    }

    fn insert_next_up(&mut self, id: VersionedSubstateId, value: SubstateValue, num_preshards: NumPreshards) {
        let up = SubstateChange::Up {
            shard: id.to_shard(num_preshards),
            substate: Box::new(Substate::new(id.version(), value)),
            id: id.into_substate_id(),
        };
        self.insert(up);
    }
}
