//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::future::Future;

use tari_common_types::types::FixedHash;
use tari_ootle_common_types::Epoch;

use crate::epoch_event_oracle::EpochEvent;

pub trait EpochEventOracle {
    /// Returns a Future that returns the next event, completing a round of scanning if necessary.
    /// The implementation must ensure that the returned Future is cancel-safe. Returns None if no further events can be
    /// returned.
    fn next_epoch_event(&mut self) -> impl Future<Output = Option<EpochEvent>> + Send;

    /// Returns true when, in the oracle's view, `current_epoch` is close enough to ending that
    /// consensus should speculatively accept an `EndEpoch` proposal even if the oracle has not
    /// yet emitted the corresponding `EpochChanged` event.
    ///
    /// "Close enough" is deliberately oracle-specific: the base-layer oracle measures proximity
    /// in base-layer blocks, a wall-clock oracle could measure in seconds, etc. The default
    /// implementation returns `false` (no leeway), which reduces to the strict behaviour of
    /// only accepting `EndEpoch` once the epoch has actually changed locally.
    fn is_within_epoch_end_spread(&self, _current_epoch: Epoch) -> bool {
        false
    }

    /// Returns the boundary hash the oracle has observed for `epoch`, or `None` if it has observed
    /// none. An error means the oracle could not determine either way and must be distinguished from
    /// `None`: a caller that treats a failed lookup as "not observed" turns a broken store into a
    /// silent abstention.
    ///
    /// This answers "have I seen this epoch's boundary?" independently of whether the corresponding
    /// `EpochChanged` event has been applied, which lets the voter ratify an `EndEpoch` hash it has
    /// genuinely scanned while the event is still queued behind an epoch activation. The default
    /// implementation returns `None`, so an oracle that cannot observe boundaries ahead of its own
    /// events is unaffected.
    fn observed_epoch_boundary_hash(&self, _epoch: Epoch) -> anyhow::Result<Option<FixedHash>> {
        Ok(None)
    }
}
