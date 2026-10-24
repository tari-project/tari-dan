//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_engine_types::substate::{Substate, SubstateDiff, SubstateId};
use tari_ootle_common_types::{SubstateRequirement, SubstateVersion, VersionedSubstateIdRef, optional::Optional};
use tari_ootle_storage::{StateStoreReadTransaction, consensus_models::SubstateChange};

use super::{
    LatestSubstateVersion,
    PendingSubstateStore,
    SubstateStoreError,
    SubstateVersionReader,
    change_log::ChangeLog,
};
use crate::traits::{ReadableSubstateStore, WriteableSubstateStore};

/// Speculative changes over a [`PendingSubstateStore`], read as though they followed every change the store holds.
///
/// The overlay reads the store as it stood when the overlay was taken: the store is borrowed shared, so it cannot
/// change while any overlay of it lives, and many overlays may read it at once from different threads. A put
/// follows the same rules as a put to the store, and one that fails here may still succeed against the store once the
/// transactions before it are applied there.
pub struct SubstateOverlay<'base, 'store, TTx: StateStoreReadTransaction> {
    base: &'base PendingSubstateStore<'store, TTx>,
    log: ChangeLog,
}

impl<'base, 'store, TTx: StateStoreReadTransaction> SubstateOverlay<'base, 'store, TTx> {
    pub(super) fn new(base: &'base PendingSubstateStore<'store, TTx>) -> Self {
        Self {
            base,
            log: ChangeLog::new(),
        }
    }

    /// The store this overlay reads beneath its own changes.
    pub fn base(&self) -> &'base PendingSubstateStore<'store, TTx> {
        self.base
    }

    pub fn get_latest_version(&self, id: &SubstateId) -> Result<LatestSubstateVersion, SubstateStoreError> {
        self.log.latest_version(self.base, id)
    }

    pub fn exists(&self, id: &SubstateId) -> Result<bool, SubstateStoreError> {
        let latest = self.get_latest_version(id).optional()?;
        Ok(latest.is_some())
    }

    pub fn get_latest_change(&self, id: &SubstateId) -> Result<SubstateChange, SubstateStoreError> {
        self.log.latest_change(self.base, id)
    }

    pub fn get_many<I: IntoIterator<Item = (SubstateRequirement, SubstateVersion)> + ExactSizeIterator>(
        &self,
        ids: I,
    ) -> Result<HashMap<SubstateRequirement, Substate>, SubstateStoreError> {
        self.log.get_many(self.base, ids)
    }

    pub fn lock_assert_is_up(&self, id: VersionedSubstateIdRef<'_>) -> Result<(), SubstateStoreError> {
        self.log.lock_assert_is_up(self.base, id)
    }

    /// The changes made in this overlay, oldest first. An in-place update of a substate the store's block already
    /// changed restates that change at its version, so these changes are read through the overlay and never applied
    /// to the store as a diff.
    pub fn changes(&self) -> &Vec<SubstateChange> {
        self.log.changes()
    }
}

impl<TTx: StateStoreReadTransaction> ReadableSubstateStore for SubstateOverlay<'_, '_, TTx> {
    type Error = SubstateStoreError;

    fn get(&self, id: VersionedSubstateIdRef<'_>) -> Result<Substate, Self::Error> {
        self.log.get(self.base, id)
    }
}

impl<TTx: StateStoreReadTransaction> SubstateVersionReader for SubstateOverlay<'_, '_, TTx> {
    fn get_latest_version(&self, id: &SubstateId) -> Result<LatestSubstateVersion, SubstateStoreError> {
        SubstateOverlay::get_latest_version(self, id)
    }

    fn lock_assert_is_up(&self, id: VersionedSubstateIdRef<'_>) -> Result<(), SubstateStoreError> {
        SubstateOverlay::lock_assert_is_up(self, id)
    }
}

impl<TTx: StateStoreReadTransaction> WriteableSubstateStore for SubstateOverlay<'_, '_, TTx> {
    fn put(&mut self, change: SubstateChange) -> Result<(), Self::Error> {
        self.log.put(self.base, change)
    }

    fn put_diff(&mut self, diff: &SubstateDiff) -> Result<(), Self::Error> {
        self.log.put_diff(self.base, diff, |_| {})
    }
}
