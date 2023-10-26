//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{Arc, RwLock};

use tari_dan_engine::runtime::{RuntimeModule, RuntimeModuleError, StateTracker};

#[derive(Debug, Clone)]
pub struct TrackCallsModule {
    calls: Arc<RwLock<Vec<&'static str>>>,
}

impl TrackCallsModule {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn get(&self) -> Vec<&'static str> {
        self.calls.read().unwrap().clone()
    }

    pub fn clear(&self) {
        self.calls.write().unwrap().clear();
    }
}

impl RuntimeModule for TrackCallsModule {
    fn on_runtime_call(&self, _tracker: &StateTracker, call: &'static str) -> Result<(), RuntimeModuleError> {
        self.calls.write().unwrap().push(call);
        Ok(())
    }
}
