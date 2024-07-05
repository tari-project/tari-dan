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
        let current_epoch = self.get_epoch();
        let mut is_updated = false;
        if epoch > current_epoch {
            is_updated = true;
            self.epoch.store(epoch.as_u64(), atomic::Ordering::SeqCst);
        }
        let current_height = self.get_height();
        if height > current_height {
            is_updated = true;
            self.height.store(height.as_u64(), atomic::Ordering::SeqCst);
        }

        if is_updated {
            info!(target: LOG_TARGET, "🧿 PACEMAKER: View updated to {self}");
        }
    }

    /// Resets the height and epoch. Prefer update.
    pub fn reset(&self, epoch: Epoch, height: NodeHeight) {
        self.epoch.store(epoch.as_u64(), atomic::Ordering::SeqCst);
        self.height.store(height.as_u64(), atomic::Ordering::SeqCst);
        info!(target: LOG_TARGET, "🧿 PACEMAKER RESET: View updated to {epoch}/{height}");
    }
}

impl Display for CurrentView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.get_epoch(), self.get_height())
    }
}
