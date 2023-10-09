//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::Display,
    sync::{atomic, atomic::AtomicU64, Arc},
};

use tari_dan_common_types::NodeHeight;

#[derive(Debug, Clone)]
pub struct CurrentHeight {
    height: Arc<AtomicU64>,
}

impl CurrentHeight {
    pub fn new() -> Self {
        Self {
            height: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn next_height(&self) -> NodeHeight {
        self.height.fetch_add(1, atomic::Ordering::SeqCst).into()
    }

    pub fn get(&self) -> NodeHeight {
        self.height.load(atomic::Ordering::SeqCst).into()
    }

    pub fn update(&self, height: NodeHeight) {
        let current_height = self.get();
        if height > current_height {
            self.set(height);
        }
    }

    fn set(&self, height: NodeHeight) {
        self.height.store(height.as_u64(), atomic::Ordering::SeqCst);
    }
}

impl Display for CurrentHeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}
