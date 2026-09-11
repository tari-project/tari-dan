//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::error::Error;

use tari_engine_types::{commit_result::RejectReason, indexed_value::IndexedValueError};
use tari_template_lib::types::{ComponentAddress, HashParseError, TemplateAddress};

use crate::{runtime::RuntimeError, template::TemplateLoaderError, wasm::WasmExecutionError};

#[derive(Debug)]
pub struct TransactionError {
    instruction_idx: Option<usize>,
    kind: Box<TransactionErrorKind>,
}

impl TransactionError {
    pub(crate) fn new(instruction_num: usize, variant: TransactionErrorKind) -> Self {
        Self {
            instruction_idx: Some(instruction_num),
            kind: Box::new(variant),
        }
    }

    pub fn instruction_num(&self) -> Option<usize> {
        self.instruction_idx
    }

    pub fn kind(&self) -> &TransactionErrorKind {
        &self.kind
    }

    pub fn to_reject_reason(&self) -> RejectReason {
        match &*self.kind {
            TransactionErrorKind::RuntimeError(err) => err.to_reject_reason(self.instruction_idx),
            TransactionErrorKind::WasmExecutionError(WasmExecutionError::RuntimeError(err)) => {
                err.to_reject_reason(self.instruction_idx)
            },
            _ => RejectReason::ExecutionFailure(self.to_string()),
        }
    }
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(idx) = self.instruction_idx {
            write!(f, "At instruction #{}: {}", idx, self.kind)
        } else {
            write!(f, "{}", self.kind)
        }
    }
}

impl Error for TransactionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.kind)
    }
}

impl From<RuntimeError> for TransactionError {
    fn from(variant: RuntimeError) -> Self {
        Self {
            instruction_idx: None,
            kind: Box::new(variant.into()),
        }
    }
}

impl From<TransactionErrorKind> for TransactionError {
    fn from(variant: TransactionErrorKind) -> Self {
        Self {
            instruction_idx: None,
            kind: Box::new(variant),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionErrorKind {
    #[error(transparent)]
    WasmExecutionError(#[from] WasmExecutionError),
    #[error("Template not found at address {address}")]
    TemplateNotFound { address: TemplateAddress },
    #[error(transparent)]
    RuntimeError(#[from] RuntimeError),
    #[error("Failed to load template '{address}': {details}")]
    FailedToLoadTemplate { address: TemplateAddress, details: String },
    #[error("BOR error: {0}")]
    BorError(#[from] tari_bor::BorError),
    #[error("Value visitor error: {0}")]
    ValueVisitorError(#[from] IndexedValueError),
    #[error("Function {name} not found")]
    FunctionNotFound { name: String },
    #[error("Invariant error: {details}")]
    InvariantError { details: String },
    #[error("Load template error: {0}")]
    LoadTemplate(#[from] TemplateLoaderError),
    #[error("WASM binary too big! {size} bytes are greater than allowed maximum {max} bytes.")]
    WasmBinaryTooBig { size: usize, max: usize },
    #[error("Template provider error: {0}")]
    TemplateProvider(String),
    #[error("Converting to hash error: {0}")]
    HashConversion(#[from] HashParseError),
    #[error("Function specified for component update is not marked as a migration function: {name}")]
    NotAMigrationFunction { name: String },
    #[error("Migration functions cannot be called directly: {name}")]
    CannotCallMigrationFunctionDirectly { name: String },
    #[error("The account constructor cannot be called directly. Use the CreateAccount instruction.")]
    CannotCallAccountConstructor,
    #[error("Invalid CreateAccount operation for component {component_address}: {details}")]
    InvalidCreateAccount {
        component_address: ComponentAddress,
        details: String,
    },
    #[error("Blob index {index} out of bounds (transaction has {count} blob(s))")]
    BlobIndexOutOfBounds { index: u8, count: usize },
}
