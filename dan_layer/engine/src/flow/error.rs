// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use thiserror::Error;
#[derive(Debug, Error)]
pub enum FlowEngineError {
    #[error("The instruction execution failed: Inner error:{inner}")]
    InstructionFailed { inner: String },
    // #[error("The flow was not valid JSON: {0}")]
    // InvalidJson(#[from] serde_json::Error),
}
