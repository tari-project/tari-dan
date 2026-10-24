//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::VersionedSubstateIdRef;

use super::{LatestSubstateVersion, SubstateStoreError};

/// Resolves the versions of a transaction's declared inputs, against a block's store or a speculative overlay of it.
pub trait SubstateVersionReader {
    fn get_latest_version(&self, id: &SubstateId) -> Result<LatestSubstateVersion, SubstateStoreError>;

    /// Fails with a lock failure if the version `id` names is not UP.
    fn lock_assert_is_up(&self, id: VersionedSubstateIdRef<'_>) -> Result<(), SubstateStoreError>;
}
