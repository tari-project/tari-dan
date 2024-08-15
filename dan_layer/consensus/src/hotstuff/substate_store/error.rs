//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{optional::IsNotFoundError, SubstateAddress};
use tari_dan_storage::{consensus_models::SubstateLockType, StorageError};
use tari_transaction::VersionedSubstateId;

#[derive(Debug, thiserror::Error)]
pub enum SubstateStoreError {
    #[error("Substate {address} not found")]
    SubstateNotFound { address: SubstateAddress },
    #[error("Substate {id} is DOWN")]
    SubstateIsDown { id: VersionedSubstateId },
    #[error("Expected substate {id} to not exist but it was found")]
    ExpectedSubstateNotExist { id: VersionedSubstateId },
    #[error("Expected substate {id} to be DOWN but it was UP")]
    ExpectedSubstateDown { id: VersionedSubstateId },
    #[error(
        "Failed to lock substate {substate_id} with flag {requested_lock} due to conflict with existing \
         {existing_lock} lock"
    )]
    LockConflict {
        substate_id: VersionedSubstateId,
        existing_lock: SubstateLockType,
        requested_lock: SubstateLockType,
    },
    #[error("Substate {substate_id} requires lock {required_lock} but is currently locked with {existing_lock}")]
    RequiresLock {
        substate_id: VersionedSubstateId,
        existing_lock: SubstateLockType,
        required_lock: SubstateLockType,
    },
    #[error("Substate {substate_id} is not {required_lock} locked")]
    NotLocked {
        substate_id: VersionedSubstateId,
        required_lock: SubstateLockType,
    },

    #[error(transparent)]
    StoreError(#[from] StorageError),
    #[error(transparent)]
    StateTreeError(#[from] tari_state_tree::StateTreeError),
}

impl IsNotFoundError for SubstateStoreError {
    fn is_not_found_error(&self) -> bool {
        match self {
            SubstateStoreError::SubstateNotFound { .. } => true,
            SubstateStoreError::StoreError(err) => err.is_not_found_error(),
            _ => false,
        }
    }
}

impl SubstateStoreError {
    pub fn or_fatal_error(self) -> Result<Self, Self> {
        match self {
            err @ SubstateStoreError::StoreError(_) => Err(err),
            err @ SubstateStoreError::StateTreeError(_) => Err(err),
            other => Ok(other),
        }
    }
}
