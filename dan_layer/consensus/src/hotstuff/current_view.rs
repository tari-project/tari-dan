//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::Display,
    sync::{atomic, atomic::AtomicU64, Arc},
};

use log::info;
use tari_dan_common_types::{Epoch, NodeHeight};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::current_view";

#[derive(Debug, Clone)]
pub struct CurrentView {
    height: Arc<AtomicU64>,
    epoch: Arc<AtomicU64>,
}

impl CurrentView {
    pub fn new() -> Self {
        Self {
            height: Arc::new(AtomicU64::new(0)),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn set_next_height(&self) {
        self.height.fetch_add(1, atomic::Ordering::SeqCst);
    }

    pub fn get_epoch(&self) -> Epoch {
        self.epoch.load(atomic::Ordering::SeqCst).into()
    }

    pub fn get_height(&self) -> NodeHeight {
        self.height.load(atomic::Ordering::SeqCst).into()
    }

    /// Updates the height and epoch if they are greater than the current values.
    pub fn update(&self, epoch: Epoch, height: NodeHeight) {
        self.update_epoch(epoch);
        let current_height = self.get_height();
        if height > current_height {
            info!(target: LOG_TARGET, "🧿 View updated to height {height}");
            self.height.store(height.as_u64(), atomic::Ordering::SeqCst);
        }
    }

    pub fn update_epoch(&self, epoch: Epoch) {
        let current_epoch = self.get_epoch();
        if epoch > current_epoch {
            info!(target: LOG_TARGET, "🧿 View updated to epoch {epoch}");
            self.epoch.store(epoch.as_u64(), atomic::Ordering::SeqCst);
        }
    }
}

impl Display for CurrentView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "epoch: {}, height: {}", self.get_epoch(), self.get_height())
    }
}
