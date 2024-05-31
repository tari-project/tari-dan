//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt;

use tari_transaction::TransactionId;

use crate::consensus_models::SubstateLockFlag;

#[derive(Debug, Clone, Copy)]
pub struct LockedSubstate {
    lock_flag: SubstateLockFlag,
    transaction_id: TransactionId,
    version: u32,
    is_local_only: bool,
}

impl LockedSubstate {
    pub fn new(transaction_id: TransactionId, version: u32, lock_flag: SubstateLockFlag, is_local_only: bool) -> Self {
        Self {
            transaction_id,
            version,
            lock_flag,
            is_local_only,
        }
    }

    pub fn transaction_id(&self) -> TransactionId {
        self.transaction_id
    }

    pub fn substate_lock(&self) -> SubstateLockFlag {
        self.lock_flag
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn is_local_only(&self) -> bool {
        self.is_local_only
    }

    pub fn is_write(&self) -> bool {
        self.lock_flag.is_write()
    }

    pub fn is_read(&self) -> bool {
        self.lock_flag.is_read()
    }

    pub fn is_output(&self) -> bool {
        self.lock_flag.is_output()
    }
}

impl fmt::Display for LockedSubstate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LockedSubstate(transaction_id: {}, version: {}, lock_flag: {}, is_local_only: {})",
            self.transaction_id, self.version, self.lock_flag, self.is_local_only
        )
    }
}
