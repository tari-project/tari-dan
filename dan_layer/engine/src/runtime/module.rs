//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::runtime::StateTracker;

pub trait RuntimeModule: Send + Sync {
    fn on_runtime_call(&self, _track: &StateTracker, _call: &'static str) -> Result<(), RuntimeModuleError> {
        Ok(())
    }
    // Add more runtime "hooks"
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum RuntimeModuleError {
    #[error("Todo")]
    Todo,
}
