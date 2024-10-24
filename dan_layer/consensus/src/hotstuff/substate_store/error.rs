//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{optional::IsNotFoundError, VersionedSubstateId};
use tari_dan_storage::{consensus_models::LockConflict, StorageError};

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
        "Failed to {} lock substate {substate_id} due to conflict with existing {} lock in transaction {}", conflict.requested_lock, conflict.existing_lock, conflict.transaction_id
    )]
    LockConflict {
        substate_id: VersionedSubstateId,
        conflict: LockConflict,
    },
}

impl LockFailedError {
    pub fn lock_conflict(&self) -> Option<&LockConflict> {
        match self {
            LockFailedError::LockConflict { conflict, .. } => Some(conflict),
            _ => None,
        }
    }
}
