//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateId;
use tari_indexer_lib::substate_cache::caches_nonexistence;

#[derive(Debug, Clone, Queryable)]
pub(crate) struct SubstateCacheRow {
    #[allow(dead_code)]
    pub substate_id: String,
    pub version: Option<i32>,
    pub verified: bool,
    pub substate_result: Vec<u8>,
    pub cached_at: i64,
}

/// A substate a synced transition has retired cached entries for.
///
/// Carries what the transition settles, which is never the whole entry: a creation retires the
/// versions below it and the claim that the substate does not exist, and a destroy retires the
/// version it names. Neither touches a head above, which a cached entry can legitimately hold after
/// being fetched straight from the committee.
#[derive(Debug, Clone)]
pub struct SubstateCacheInvalidation {
    substate_id: SubstateId,
    retires_up_to: Option<u32>,
    retires_nonexistence: bool,
}

impl SubstateCacheInvalidation {
    /// The substate was created at `version`, so every lower version is spent and it demonstrably
    /// exists.
    ///
    /// A first creation retires no version - there is none below 0 - so all it carries is the
    /// retraction of a cached nonexistence. Where nothing caches that, it carries nothing, and is
    /// not one of these at all: emitting it would put a journal row on the sync path for every
    /// created-once substate in the stream and buy nothing with it.
    pub fn created(substate_id: &SubstateId, version: u32) -> Option<Self> {
        let retires_up_to = version.checked_sub(1);
        let retires_nonexistence = caches_nonexistence(substate_id);
        if retires_up_to.is_none() && !retires_nonexistence {
            return None;
        }
        Some(Self {
            substate_id: substate_id.clone(),
            retires_up_to,
            retires_nonexistence,
        })
    }

    /// `version` was destroyed, with or without a successor.
    ///
    /// A destroy leaves a cached nonexistence alone. `DoesNotExist` says the substate has no live
    /// version, which a destroy makes more true rather than less.
    pub fn destroyed(substate_id: SubstateId, version: u32) -> Self {
        Self {
            substate_id,
            retires_up_to: Some(version),
            retires_nonexistence: false,
        }
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    /// The highest cached head version this retires, if any.
    pub fn retires_up_to(&self) -> Option<u32> {
        self.retires_up_to
    }

    /// Whether a record that the substate does not exist may exist to be retired. False where
    /// nothing caches one, so the retirement is not attempted for the substates that dominate the
    /// stream by count.
    pub fn retires_nonexistence(&self) -> bool {
        self.retires_nonexistence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn substate() -> SubstateId {
        format!("component_{:064x}", 1).parse().unwrap()
    }

    fn receipt() -> SubstateId {
        format!("txreceipt_{:064x}", 1).parse().unwrap()
    }

    #[test]
    fn a_first_creation_retires_nothing_but_the_nonexistence() {
        let invalidation = SubstateCacheInvalidation::created(&substate(), 0).unwrap();
        assert_eq!(invalidation.retires_up_to(), None);
        assert!(invalidation.retires_nonexistence());
    }

    /// The whole reason first creations are journalled at all is to retract a cached nonexistence,
    /// so one that cannot be cached must not put a row on the sync path.
    #[test]
    fn a_first_creation_is_dropped_where_nonexistence_is_not_cached() {
        assert!(!caches_nonexistence(&receipt()));
        assert!(SubstateCacheInvalidation::created(&receipt(), 0).is_none());
        // Above the first version it carries a retirement that stands on its own, and does not
        // attempt one that could never match.
        let above = SubstateCacheInvalidation::created(&receipt(), 1).unwrap();
        assert_eq!(above.retires_up_to(), Some(0));
        assert!(!above.retires_nonexistence());
    }

    #[test]
    fn a_creation_retires_every_version_below_it() {
        let invalidation = SubstateCacheInvalidation::created(&substate(), 6).unwrap();
        assert_eq!(invalidation.retires_up_to(), Some(5));
        assert!(invalidation.retires_nonexistence());
    }

    #[test]
    fn a_destroy_retires_the_version_it_names_and_no_nonexistence() {
        let invalidation = SubstateCacheInvalidation::destroyed(substate(), 6);
        assert_eq!(invalidation.retires_up_to(), Some(6));
        assert!(!invalidation.retires_nonexistence());
    }
}
