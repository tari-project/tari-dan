// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use tari_bor::BorError;
use tari_engine_types::{commit_result::ExecutionFailureCode, indexed_value::IndexedValueError};
use tari_template_abi::{TEMPLATE_DEF_CUSTOM_SECTION, version::WasmAbiVersion};
use wasmer::{ExportError, InstantiationError, MemoryAccessError};

use crate::runtime::RuntimeError;

#[derive(Debug, thiserror::Error)]
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
    // NOTE this renders as "Wasm RuntimeError: <message>"
    #[error("Wasm {0}")]
    WasmRuntimeError(#[from] wasmer::RuntimeError),
    #[error(
        "Exceeded a fixed compute maximum after consuming {consumed_points} WASM metering points. Whichever cap bound \
         the call, per call or per transaction, a higher fee does not raise it — split the work up."
    )]
    MaxComputeExceeded { consumed_points: u64 },
    #[error(
        "Insufficient fees to pay for compute: consumed {consumed_points} WASM metering points, which exceeds what \
         the paid fees cover. Increase the transaction fee."
    )]
    InsufficientFeesForCompute { consumed_points: u64 },
    #[error(
        "The fee intent exceeded its {credit_points} points of compute credit after consuming {consumed_points} WASM \
         metering points. The credit is a flat allowance for sourcing a fee and does not rise with the fee paid — \
         move this work to the main instructions."
    )]
    FeeIntentComputeExceeded { consumed_points: u64, credit_points: u64 },
    #[error("Expected function {function} to return a pointer")]
    ExpectedPointerReturn { function: String },
    #[error("Memory access error: {0}")]
    MemoryAccessError(#[from] MemoryAccessError),
    #[error("memory underflow: {required} bytes required but {remaining} remaining")]
    MemoryUnderflow { required: usize, remaining: usize },
    #[error("memory pointer out of range: memory size is {size}, pointer is {pointer} and length is {len}")]
    MemoryPointerOutOfRange { size: u64, pointer: u32, len: u32 },
    #[error("Memory allocation too large")]
    MemoryAllocationTooLarge,
    #[error("Memory allocation failed")]
    MemoryAllocationFailed,
    #[error("BUG: memory not set in environment")]
    MemoryNotSet,
    #[error("BUG: template code ran on an instance with no call bound to it")]
    NoCallBound,
    #[error("Missing function {function}")]
    MissingAbiFunction { function: &'static str },
    #[error("Engine call size limit of {limit} bytes exceeded")]
    CallSizeLimitExceeded { limit: usize },
    /// The argument buffer a template passed to an engine call. Distinct from
    /// [`Self::CallSizeLimitExceeded`], which bounds the arguments passed the other way, into a
    /// template function.
    #[error("Engine call argument size limit of {limit} bytes exceeded: {size} bytes")]
    EngineCallArgSizeExceeded { limit: usize, size: usize },
    #[error("Invalid engine op {op}")]
    InvalidEngineOp { op: i32 },
    #[error("Runtime error: {0}")]
    RuntimeError(#[from] RuntimeError),
    #[error("Failed to decode argument for engine call: {0:?}")]
    EngineArgDecodeFailed(BorError),
    #[error("Failed to decode template definition: {0:?}")]
    AbiTemplateDefDecodeError(BorError),
    #[error("Malformed `{TEMPLATE_DEF_CUSTOM_SECTION}` custom section: {reason}")]
    AbiTemplateDefSectionMalformed { reason: String },
    #[error(
        "Module does not contain a `{TEMPLATE_DEF_CUSTOM_SECTION}` custom section. Rebuild the template with a \
         current `tari_template_lib`."
    )]
    AbiTemplateDefSectionMissing,
    #[error("Unexpected ABI function {name}")]
    UnexpectedAbiFunction { name: String },
    #[error("Encoding error: {0}")]
    EncodingError(#[from] BorError),
    #[error("Template error: {message}")]
    Panic {
        message: String,
        runtime_error: wasmer::RuntimeError,
    },
    #[error("Value visitor error: {0}")]
    ValueVisitorError(#[from] IndexedValueError),
    #[error("Template version {template_version} is incompatible with current engine version {engine_version}")]
    TemplateVersionMismatch {
        engine_version: WasmAbiVersion,
        template_version: WasmAbiVersion,
    },
    #[error("Function '{name}' expected {expected} arguments, but got {actual}")]
    InvalidArgumentCount {
        name: String,
        expected: usize,
        actual: usize,
    },
    #[error("Wasm validation error: {0}")]
    WasmValidationError(#[from] WasmValidationError),
}

#[derive(Debug, thiserror::Error)]
pub enum WasmValidationError {
    #[error("Function name {name} is too long, maximum length is {max_length}")]
    FunctionNameTooLong { name: String, max_length: usize },
    #[error("Function {name} contained too many arguments, maximum is {max_args}, but got {num_args}")]
    FunctionTooManyArguments {
        name: String,
        max_args: usize,
        num_args: usize,
    },
    #[error(
        "Function {name} contained too many return values in a tuple, maximum is {max_tuple_size}, but got \
         {tuple_size}"
    )]
    FunctionTooManyTupleReturn {
        name: String,
        max_tuple_size: usize,
        tuple_size: usize,
    },
    #[error("Too many functions in the module, maximum is {max_functions}")]
    TooManyFunctions { max_functions: usize },
    #[error("Function {function_name} has invalid return type: {return_type:?}")]
    InvalidMigrationReturnType {
        function_name: String,
        return_type: tari_template_abi::Type,
    },
    #[error(
        "Module contains disallowed custom section `{name}`; only `{TEMPLATE_DEF_CUSTOM_SECTION}` is permitted. Strip \
         debug and metadata sections before publishing (e.g. `wasm-opt --strip-debug --strip-producers` or \
         `wasm-strip`)."
    )]
    DisallowedCustomSection { name: String },
    #[error("Module declares a start function, which templates may not do")]
    StartSectionNotAllowed,
    #[error("Module declares {count} tables, the maximum is {max_tables}")]
    TooManyTables { count: usize, max_tables: usize },
    #[error("Module declares {count} globals, the maximum is {max_globals}")]
    TooManyGlobals { count: usize, max_globals: usize },
    #[error("Module contains more than {max_functions} functions")]
    TooManyModuleFunctions { max_functions: usize },
    #[error("Module's functions declare more than {max_variables} parameters and locals")]
    TooManyModuleVariables { max_variables: usize },
    #[error("Function {function} contains more than {max_checks} control-flow and bulk-memory operators")]
    TooManyFunctionMeteringChecks { function: u32, max_checks: usize },
    #[error("Function {function} contains more than {max_loops} loops")]
    TooManyFunctionLoops { function: u32, max_loops: usize },
    #[error("Module contains more than {max_loops} loops")]
    TooManyModuleLoops { max_loops: usize },
    #[error("Function {function} nests blocks more than {max_depth} deep")]
    FunctionNestedTooDeep { function: u32, max_depth: usize },
    #[error(
        "Module asks the compile for the work of a {billed_bytes}-byte binary, more than the {max_bytes}-byte publish \
         limit"
    )]
    TooMuchCompileWork { billed_bytes: u64, max_bytes: usize },
    #[error("Module does not export `{name}`")]
    MissingExport { name: String },
    #[error("Export `{name}` has signature `{signature}`, expected `{expected}`")]
    InvalidExportSignature {
        name: String,
        signature: String,
        expected: String,
    },
}

impl From<wasmer::InstantiationError> for WasmExecutionError {
    fn from(value: InstantiationError) -> Self {
        Self::InstantiationError(Box::new(value))
    }
}

impl WasmExecutionError {
    /// The coarse reason this failure is reported to consumers as. Exhaustive by design — see
    /// `RuntimeError::failure_code`.
    pub fn failure_code(&self) -> ExecutionFailureCode {
        use ExecutionFailureCode as C;
        match self {
            // A trap, a bad pointer or a malformed ABI section are all the template's doing: the engine
            // handed it a well-formed call and the module could not honour it.
            Self::InstantiationError(_) |
            Self::ExportError(_) |
            Self::WasmRuntimeError(_) |
            Self::ExpectedPointerReturn { .. } |
            Self::MemoryAccessError(_) |
            Self::MemoryUnderflow { .. } |
            Self::MemoryPointerOutOfRange { .. } |
            Self::MissingAbiFunction { .. } |
            Self::InvalidEngineOp { .. } |
            Self::EngineArgDecodeFailed(_) |
            Self::AbiTemplateDefDecodeError(_) |
            Self::AbiTemplateDefSectionMalformed { .. } |
            Self::AbiTemplateDefSectionMissing |
            Self::UnexpectedAbiFunction { .. } |
            Self::EncodingError(_) |
            Self::Panic { .. } |
            Self::ValueVisitorError(_) |
            Self::TemplateVersionMismatch { .. } => C::TemplateError,

            Self::InsufficientFeesForCompute { .. } => C::OutOfCompute,

            // Both are flat ceilings a larger fee does not raise; only moving the work clears them.
            Self::FeeIntentComputeExceeded { .. } | Self::MaxComputeExceeded { .. } => C::LimitExceeded,

            Self::MemoryAllocationTooLarge |
            Self::MemoryAllocationFailed |
            Self::CallSizeLimitExceeded { .. } |
            Self::EngineCallArgSizeExceeded { .. } => C::LimitExceeded,

            Self::InvalidArgumentCount { .. } => C::InvalidArgument,
            Self::MemoryNotSet | Self::NoCallBound => C::EngineInvariant,

            Self::RuntimeError(err) => err.failure_code(),
            Self::WasmValidationError(err) => err.failure_code(),
        }
    }
}

impl WasmValidationError {
    /// Every validation failure is a property of the published module, so the template is always at fault.
    pub fn failure_code(&self) -> ExecutionFailureCode {
        match self {
            Self::FunctionNameTooLong { .. } |
            Self::FunctionTooManyArguments { .. } |
            Self::FunctionTooManyTupleReturn { .. } |
            Self::TooManyFunctions { .. } |
            Self::InvalidMigrationReturnType { .. } |
            Self::DisallowedCustomSection { .. } |
            Self::StartSectionNotAllowed |
            Self::TooManyTables { .. } |
            Self::TooManyGlobals { .. } |
            Self::TooManyModuleFunctions { .. } |
            Self::TooManyModuleVariables { .. } |
            Self::TooManyFunctionMeteringChecks { .. } |
            Self::TooManyFunctionLoops { .. } |
            Self::TooManyModuleLoops { .. } |
            Self::FunctionNestedTooDeep { .. } |
            Self::TooMuchCompileWork { .. } |
            Self::MissingExport { .. } |
            Self::InvalidExportSignature { .. } => ExecutionFailureCode::TemplateError,
        }
    }
}
