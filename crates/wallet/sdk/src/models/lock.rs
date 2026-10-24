//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_transaction::TransactionId;
use time::PrimitiveDateTime;

use crate::models::WalletLockId;

/// The caller a lock was created for.
///
/// A handler that hands a lock id back to its caller records the caller here,
/// and only that caller may later name the lock in a transaction request. Lock
/// ids are sequential, so the id alone proves nothing about who selected the
/// outputs behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LockOwner {
    /// The wallet's own interactive session, or the wallet itself for a lock
    /// it takes and links internally.
    Wallet,
    /// A local process authenticating with the daemon's `.cookie` token.
    LocalCookie,
    /// A connected app, by the id of the `webrtc.start` call that minted its
    /// token.
    Delegated(u64),
    /// An API key, by its id. Names are not unique; ids are.
    ApiKey(i32),
}

#[derive(Debug, Clone)]
pub struct WalletLock {
    pub id: WalletLockId,
    pub owner: LockOwner,
    /// Set once the lock's outputs are spent by a submitted transaction. A
    /// linked lock resolves with that transaction, never by timeout.
    pub transaction_id: Option<TransactionId>,
    /// When the stale sweep releases the lock. `None` exempts it.
    pub timeout_at: Option<PrimitiveDateTime>,
}
