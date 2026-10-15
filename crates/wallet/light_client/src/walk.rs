//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::Epoch;

use crate::TrustedValidatorSet;

/// About a week of epochs at 20 minutes each.
pub const DEFAULT_MAX_WALK_EPOCHS: u64 = 504;

/// How a light client catches up from its trusted validator set to the latest checkpoint an indexer serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkPlan {
    /// The trusted validator set is the one the latest checkpoint names, or a later one.
    UpToDate,
    /// [`TrustedValidatorSet::advance`] through the checkpoints of `from_epoch` to `to_epoch` inclusive.
    Walk { from_epoch: Epoch, to_epoch: Epoch },
    /// The walk is longer than the light client allows. It anchors again on the latest checkpoint with
    /// [`TrustedValidatorSet::anchor_on_first_use`], and the wallet tells its user it did.
    Reanchor,
}

/// Plans the catch-up from `trusted` to the checkpoint of `latest_checkpoint_epoch`, walking at most `max_walk_epochs`
/// checkpoints.
///
/// `latest_checkpoint_epoch` is the indexer's claim, and an indexer that overstates it forces a re-anchor on a set
/// only that indexer vouches for. That is why a wallet surfaces every re-anchor rather than taking one silently.
pub fn plan_walk(trusted: &TrustedValidatorSet, latest_checkpoint_epoch: Epoch, max_walk_epochs: u64) -> WalkPlan {
    let from_epoch = trusted.epoch();
    if latest_checkpoint_epoch < from_epoch {
        return WalkPlan::UpToDate;
    }
    let num_checkpoints = (latest_checkpoint_epoch.as_u64() - from_epoch.as_u64()).saturating_add(1);
    if num_checkpoints > max_walk_epochs {
        return WalkPlan::Reanchor;
    }
    WalkPlan::Walk {
        from_epoch,
        to_epoch: latest_checkpoint_epoch,
    }
}
