//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{optional::IsNotFoundError, SubstateLockType, VersionedSubstateId};
use tari_dan_storage::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum SubstateStoreError {
    #[error("Lock failure: {0}")]
    LockFailed(#[from] LockFailedError),
    #[error("Substate {id} not found")]
    SubstateNotFound { id: VersionedSubstateId },
    #[error("Substate {id} is DOWN")]
    SubstateIsDown { id: VersionedSubstateId },
    #[error("Expected substate {id} to not exist but it was found")]
    ExpectedSubstateNotExist { id: VersionedSubstateId },
    #[error("Expected substate {id} to be DOWN but it was UP")]
    ExpectedSubstateDown { id: VersionedSubstateId },

    #[error(transparent)]
    StoreError(#[from] StorageError),
    #[error(transparent)]
    StateTreeError(#[from] tari_state_tree::StateTreeError),
}

impl IsNotFoundError for SubstateStoreError {
    fn is_not_found_error(&self) -> bool {
        match self {
            SubstateStoreError::LockFailed(LockFailedError::SubstateNotFound { .. }) => true,
            SubstateStoreError::SubstateNotFound { .. } => true,
            SubstateStoreError::StoreError(err) => err.is_not_found_error(),
            _ => false,
        }
    }
}

impl SubstateStoreError {
    pub fn ok_lock_failed(self) -> Result<LockFailedError, Self> {
        match self {
            SubstateStoreError::LockFailed(err) => Ok(err),
            other => Err(other),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LockFailedError {
    #[error("Substate {id} not found")]
    SubstateNotFound { id: VersionedSubstateId },
    #[error("Substate {id} is DOWN")]
    SubstateIsDown { id: VersionedSubstateId },
    #[error(
        "Failed to {requested_lock} lock substate {substate_id} due to conflict with existing {existing_lock} lock"
    )]
    LockConflict {
        substate_id: VersionedSubstateId,
        existing_lock: SubstateLockType,
        requested_lock: SubstateLockType,
    },
}
