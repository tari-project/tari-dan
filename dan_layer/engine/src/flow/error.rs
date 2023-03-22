// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use thiserror::Error;

use crate::runtime::RuntimeError;

#[derive(Debug, Error)]
pub enum FlowEngineError {
    #[error("The instruction execution failed: Inner error:{inner}")]
    InstructionFailed { inner: String },
    #[error("Missing argument: {name}")]
    MissingArgument { name: String },
    #[error(transparent)]
    RuntimeError(#[from] RuntimeError),
    #[error(transparent)]
    ExecutionError(#[from] anyhow::Error),
}
