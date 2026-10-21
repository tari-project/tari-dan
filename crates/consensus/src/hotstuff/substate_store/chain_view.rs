//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus_types::LeafBlock;
use tari_engine_types::substate::{Substate, SubstateId};
use tari_ootle_common_types::{
    NumPreshards,
    SubstateVersion,
    ToSubstateAddress,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    optional::Optional,
};
use tari_ootle_storage::{
    PendingChain,
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{BlockDiff, SubstateChange, SubstateRecord},
};

use super::{LatestSubstateVersion, SubstateStoreError, change_log::SubstateLayer};

/// The state a block builds on: durable substates overlaid by the diffs of the pending chain ending at
/// `parent_block`.
pub(super) struct ChainView<'store, TTx> {
    pub store: &'store TTx,
    pub parent_block: LeafBlock,
    /// Read once for every branch-scoped read at `parent_block`. `store` is borrowed shared for as long as this lives,
    /// so nothing can write to the pending chain while the view is in use.
    pub parent_chain: PendingChain,
    pub num_preshards: NumPreshards,
}

impl<'store, TTx: StateStoreReadTransaction> ChainView<'store, TTx> {
    pub fn init(
        store: &'store TTx,
        parent_block: LeafBlock,
        num_preshards: NumPreshards,
    ) -> Result<Self, StorageError> {
        let parent_chain = store.pending_chain_get(parent_block.block_id())?;
        Ok(Self {
            store,
            parent_block,
            parent_chain,
            num_preshards,
        })
    }
}

impl<TTx: StateStoreReadTransaction> SubstateLayer for ChainView<'_, TTx> {
    fn num_preshards(&self) -> NumPreshards {
        self.num_preshards
    }

    fn get_substate(&self, id: VersionedSubstateIdRef<'_>) -> Result<Substate, SubstateStoreError> {
        if let Some(change) = BlockDiff::get_for_versioned_substate(self.store, &self.parent_chain, id).optional()? {
            return change
                .into_up()
                .ok_or_else(|| SubstateStoreError::SubstateNotFound { id: id.to_owned() });
        }

        let Some(substate) = SubstateRecord::get(self.store, &id.to_substate_address()).optional()? else {
            return Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() });
        };
        if substate.is_destroyed() {
            return Err(SubstateStoreError::SubstateNotFound { id: id.to_owned() });
        }
        substate
            .into_substate()
            .ok_or_else(|| up_substate_without_value(id.to_owned()))
    }

    fn is_version_up(&self, id: VersionedSubstateIdRef<'_>) -> Result<Option<bool>, SubstateStoreError> {
        if let Some(change) = BlockDiff::get_for_versioned_substate(self.store, &self.parent_chain, id).optional()? {
            return Ok(Some(change.is_up()));
        }

        let is_up = SubstateRecord::substate_is_up(self.store, &id.to_substate_address()).optional()?;
        Ok(is_up)
    }

    fn latest_change(&self, id: &SubstateId) -> Result<SubstateChange, SubstateStoreError> {
        if let Some(change) = BlockDiff::get_last_change_for_substate(self.store, &self.parent_chain, id).optional()? {
            return Ok(change);
        }

        let substate = SubstateRecord::get_latest(self.store, id).optional()?.ok_or_else(|| {
            SubstateStoreError::SubstateNotFound {
                id: VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO),
            }
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

    fn latest_version(&self, id: &SubstateId) -> Result<LatestSubstateVersion, SubstateStoreError> {
        if let Some(change) = BlockDiff::get_last_change_for_substate(self.store, &self.parent_chain, id).optional()? {
            return Ok(LatestSubstateVersion::new(
                change.versioned_substate_id().version(),
                change.is_up(),
            ));
        }

        let (version, is_up) = SubstateRecord::get_latest_version(self.store, id)
            .optional()?
            .ok_or_else(|| SubstateStoreError::SubstateNotFound {
                id: VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO),
            })?;

        Ok(LatestSubstateVersion::new(version, is_up))
    }

    fn block_head_change(&self, _id: &SubstateId) -> Option<&SubstateChange> {
        None
    }
}

fn up_substate_without_value(id: VersionedSubstateId) -> SubstateStoreError {
    SubstateStoreError::InvariantError {
        details: format!("Substate {id} is UP but its value is missing from the store"),
    }
}
