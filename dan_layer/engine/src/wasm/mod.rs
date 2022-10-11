// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

pub mod compile;

mod error;
pub use error::{WasmError, WasmExecutionError};

mod environment;

mod module;
pub use module::{LoadedWasmTemplate, WasmModule};

mod metering;
mod process;

pub use process::{ExecutionResult, WasmProcess};
