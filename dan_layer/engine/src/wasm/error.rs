// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use tari_bor::BorError;
use tari_engine_types::indexed_value::IndexedValueVisitorError;
use thiserror::Error;
use wasmer::{ExportError, HostEnvInitError, InstantiationError};

use crate::runtime::RuntimeError;

#[derive(Debug, Error)]
pub enum WasmError {
    #[error("Missing argument at position {position} (name: {argument_name}")]
    MissingArgument { argument_name: String, position: usize },
}

#[derive(Debug, thiserror::Error)]
pub enum WasmExecutionError {
    #[error(transparent)]
    InstantiationError(Box<InstantiationError>),
    #[error(transparent)]
    ExportError(#[from] ExportError),
    #[error(transparent)]
    WasmRuntimeError(#[from] wasmer::RuntimeError),
    #[error(transparent)]
    HostEnvInitError(#[from] HostEnvInitError),
    #[error("Function {name} not found")]
    FunctionNotFound { name: String },
    #[error("Expected function {function} to return a pointer")]
    ExpectedPointerReturn { function: String },
    #[error("Attempted to write {requested} bytes but pointer allocated {allocated}")]
    InvalidWriteLength { allocated: u32, requested: u32 },
    #[error("memory underflow: {required} bytes required but {remaining} remaining")]
    MemoryUnderflow { required: usize, remaining: usize },
    #[error("memory pointer out of range: memory size of {size} but pointer is {pointer}")]
    MemoryPointerOutOfRange { size: u64, pointer: u64, len: u64 },
    #[error("Memory allocation failed")]
    MemoryAllocationFailed,
    #[error("Memory not initialized")]
    MemoryNotInitialized,
    #[error("Missing function {function}")]
    MissingAbiFunction { function: String },
    #[error("Runtime error: {0}")]
    RuntimeError(#[from] RuntimeError),
    #[error("Failed to decode argument for engine call: {0:?}")]
    EngineArgDecodeFailed(BorError),
    #[error("maximum module memory size exceeded")]
    MaxMemorySizeExceeded,
    #[error("Failed to decode ABI: {0:?}")]
    AbiDecodeError(BorError),
    #[error("package ABI function returned an invalid type")]
    InvalidReturnTypeFromAbiFunc,
    #[error("package did not contain an ABI definition")]
    NoAbiDefinition,
    #[error("Unexpected ABI function {name}")]
    UnexpectedAbiFunction { name: String },
    #[error("Panic! {message}")]
    Panic {
        message: String,
        runtime_error: wasmer::RuntimeError,
    },
    #[error("Value visitor error: {0}")]
    ValueVisitorError(#[from] IndexedValueVisitorError),
}
impl From<wasmer::InstantiationError> for WasmExecutionError {
    fn from(value: InstantiationError) -> Self {
        Self::InstantiationError(Box::new(value))
    }
}
