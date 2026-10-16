//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use tari_consensus::{
    hotstuff::HotStuffError,
    traits::{SyncManager, SyncStatus},
};
use tari_engine_types::Epoch;

/// Reports the node as up to date, unless a test asks for the next check to report it behind.
///
/// A sync copies nothing, so a node sent through `Syncing` keeps the state it already holds. That is enough for a test
/// of what consensus itself does around a sync.
#[derive(Clone, Default)]
pub struct TestSyncManager {
    behind_on_next_check: Arc<AtomicBool>,
    num_syncs: Arc<AtomicUsize>,
}

impl TestSyncManager {
    /// Makes the next `check_sync` report the node behind, which sends it through `Syncing`.
    pub fn report_behind_on_next_check(&self) {
        self.behind_on_next_check.store(true, Ordering::SeqCst);
    }

    pub fn num_syncs(&self) -> usize {
        self.num_syncs.load(Ordering::SeqCst)
    }
}

impl SyncManager for TestSyncManager {
    type Error = HotStuffError;

    async fn check_sync(&self) -> Result<SyncStatus, Self::Error> {
        if self.behind_on_next_check.swap(false, Ordering::SeqCst) {
            return Ok(SyncStatus::Behind { target_epoch: None });
        }
        Ok(SyncStatus::UpToDate)
    }

    async fn sync(&mut self, _target_epoch: Option<Epoch>) -> Result<(), Self::Error> {
        self.num_syncs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
