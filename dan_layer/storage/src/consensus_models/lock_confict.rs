//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::SubstateLockType;
use tari_transaction::TransactionId;

#[derive(Debug, Clone, Copy)]
pub struct LockConflict {
    pub transaction_id: TransactionId,
    pub existing_lock: SubstateLockType,
    pub requested_lock: SubstateLockType,
}
