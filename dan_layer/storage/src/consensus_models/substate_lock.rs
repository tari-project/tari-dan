//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt;

use tari_dan_common_types::{SubstateAddress, SubstateLockType, VersionedSubstateId};
use tari_engine_types::substate::{SubstateId, SubstateValue};
use tari_transaction::TransactionId;

use crate::consensus_models::{BlockId, VersionedSubstateIdLockIntent};

#[derive(Debug, Clone, Copy)]
pub struct SubstateLock {
    lock_type: SubstateLockType,
    transaction_id: TransactionId,
    version: u32,
    is_local_only: bool,
}

impl SubstateLock {
    pub fn new(transaction_id: TransactionId, version: u32, lock_type: SubstateLockType, is_local_only: bool) -> Self {
        Self {
            transaction_id,
            version,
            lock_type,
            is_local_only,
        }
    }

    pub fn transaction_id(&self) -> TransactionId {
        self.transaction_id
    }

    pub fn substate_lock(&self) -> SubstateLockType {
        self.lock_type
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn is_local_only(&self) -> bool {
        self.is_local_only
    }

    pub fn is_write(&self) -> bool {
        self.lock_type.is_write()
    }

    pub fn is_read(&self) -> bool {
        self.lock_type.is_read()
    }

    pub fn is_output(&self) -> bool {
        self.lock_type.is_output()
    }
}

impl fmt::Display for SubstateLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LockedSubstate(transaction_id: {}, version: {}, lock_flag: {}, is_local_only: {})",
            self.transaction_id, self.version, self.lock_type, self.is_local_only
        )
    }
}

#[derive(Debug, Clone)]
pub struct LockedSubstateValue {
    pub locked_by_block: BlockId,
    pub substate_id: SubstateId,
    pub lock: SubstateLock,
    /// The value of the locked substate. This may be None if the substate lock is Output.
    pub value: Option<SubstateValue>,
}

impl LockedSubstateValue {
    pub(crate) fn to_substate_lock_intent(&self) -> VersionedSubstateIdLockIntent {
        VersionedSubstateIdLockIntent::new(
            VersionedSubstateId::new(self.substate_id.clone(), self.lock.version()),
            self.lock.substate_lock(),
        )
    }

    pub fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(&self.substate_id, self.lock.version())
    }
}
