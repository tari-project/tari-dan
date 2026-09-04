//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::{Duration, Instant},
};

use tari_ootle_common_types::{StateVersion, shard::Shard};

/// Per-shard completeness: the state version this indexer holds every transition up to, and when a
/// committee last confirmed that it does.
///
/// The substate cache answers a read on the argument that every commit which could supersede or
/// destroy the entry has already reached it through that shard's transition stream. The argument
/// only holds up to a watermark, and only while the watermark is being refreshed: a peer that stops
/// serving transitions freezes it, and the cache must then close for that shard rather than serve
/// entries nothing can retract.
///
/// Confirmations are per process run, not persisted. An indexer that has just started has proven
/// nothing about how far behind it is, whatever its stored progress says, and serves from cache only
/// once its first sync round lands - which is also when the transitions it missed while down arrive.
#[derive(Debug, Default)]
pub struct ShardWatermarks {
    inner: RwLock<HashMap<Shard, Watermark>>,
}

#[derive(Debug, Clone, Copy)]
struct Watermark {
    state_version: StateVersion,
    confirmed_at: Instant,
}

impl ShardWatermarks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that this indexer is level with `shard`'s committee at `state_version`, every
    /// transition up to it committed. Only a stream's completion marker establishes that: a batch
    /// part-way through a catch-up leaves the chain arbitrarily far ahead. Confirming a shard whose
    /// version did not move is what keeps a quiet shard from ageing out.
    pub fn confirm(&self, shard: Shard, state_version: StateVersion) {
        let now = Instant::now();
        let mut inner = self.write();
        let entry = inner.entry(shard).or_insert(Watermark {
            state_version,
            confirmed_at: now,
        });
        entry.state_version = entry.state_version.max(state_version);
        entry.confirmed_at = now;
    }

    /// Re-stamps `shard`'s watermark as confirmed now, at the version it already holds. A shard that
    /// was never confirmed stays unconfirmed: liveness alone says nothing about what it holds.
    pub fn refresh(&self, shard: Shard) {
        let now = Instant::now();
        if let Some(entry) = self.write().get_mut(&shard) {
            entry.confirmed_at = now;
        }
    }

    /// The watermark for `shard`, or `None` if it has never been confirmed in this run or was last
    /// confirmed longer than `max_lag` ago.
    pub fn get(&self, shard: Shard, max_lag: Duration) -> Option<StateVersion> {
        let inner = self.read();
        let watermark = inner.get(&shard)?;
        (watermark.confirmed_at.elapsed() <= max_lag).then_some(watermark.state_version)
    }

    /// The watermark for `shard` and how long ago it was confirmed, or `None` if it never was in
    /// this run. Read together so that a confirmation landing between the two cannot make them
    /// disagree.
    pub fn confirmed(&self, shard: Shard) -> Option<(StateVersion, Duration)> {
        self.read()
            .get(&shard)
            .map(|watermark| (watermark.state_version, watermark.confirmed_at.elapsed()))
    }

    // Poisoning is recovered from rather than propagated: the map holds no invariant a panic could
    // break.

    fn read(&self) -> RwLockReadGuard<'_, HashMap<Shard, Watermark>> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<Shard, Watermark>> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARD: Shard = Shard::from_u32(1);
    const MAX_LAG: Duration = Duration::from_secs(60);

    #[test]
    fn an_unconfirmed_shard_has_no_watermark() {
        let watermarks = ShardWatermarks::new();
        assert!(watermarks.get(SHARD, MAX_LAG).is_none());
    }

    #[test]
    fn a_confirmed_shard_reports_its_highest_version() {
        let watermarks = ShardWatermarks::new();
        watermarks.confirm(SHARD, StateVersion::new(10));
        watermarks.confirm(SHARD, StateVersion::new(4));
        assert_eq!(watermarks.get(SHARD, MAX_LAG), Some(StateVersion::new(10)));
    }

    #[test]
    fn a_stale_confirmation_closes_the_shard() {
        let watermarks = ShardWatermarks::new();
        watermarks.confirm(SHARD, StateVersion::new(10));
        assert!(watermarks.get(SHARD, Duration::ZERO).is_none());
    }

    #[test]
    fn a_refresh_keeps_a_shard_open_at_its_version() {
        let watermarks = ShardWatermarks::new();
        watermarks.confirm(SHARD, StateVersion::new(7));
        watermarks.refresh(SHARD);
        assert_eq!(watermarks.get(SHARD, MAX_LAG), Some(StateVersion::new(7)));
    }

    #[test]
    fn a_confirmation_reports_its_version_and_age_together() {
        let watermarks = ShardWatermarks::new();
        assert!(watermarks.confirmed(SHARD).is_none());
        watermarks.confirm(SHARD, StateVersion::new(7));
        let (version, age) = watermarks.confirmed(SHARD).unwrap();
        assert_eq!(version, StateVersion::new(7));
        assert!(age < MAX_LAG);
    }

    #[test]
    fn a_refresh_does_not_confirm_a_shard_that_never_was() {
        let watermarks = ShardWatermarks::new();
        watermarks.refresh(SHARD);
        assert_eq!(watermarks.get(SHARD, MAX_LAG), None);
    }
}
