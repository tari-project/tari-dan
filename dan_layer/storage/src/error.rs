//  Copyright 2021, The Tari Project
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

use tari_common_types::types::FixedHashSizeError;
use tari_dan_common_types::optional::IsNotFoundError;
use crate::consensus_models::BlockError;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Could not connect to storage:{reason}")]
    ConnectionError { reason: String },
    #[error("Query error:{reason}")]
    QueryError { reason: String },
    #[error("Migration error: {reason}")]
    MigrationError { reason: String },
    #[error("Not found: item: {item}, key: {key}")]
    NotFound { item: String, key: String },
    #[error("Not found in operation {operation}: {source}")]
    NotFoundDbAdapter {
        operation: &'static str,
        source: anyhow::Error,
    },
    #[error("File system path does not exist")]
    FileSystemPathDoesNotExist,
    #[error("Failed to decode for operation {operation} on {item}: {details}")]
    DecodingError {
        operation: &'static str,
        item: &'static str,
        details: String,
    },
    #[error("Failed to encode for operation {operation} on {item}: {details}")]
    EncodingError {
        operation: &'static str,
        item: &'static str,
        details: String,
    },
    #[error("Fixed hash size error: {0}")]
    FixedHashSizeError(#[from] FixedHashSizeError),
    #[error("Invalid integer cast")]
    InvalidIntegerCast,
    #[error("Data inconsistency: {details}")]
    DataInconsistency { details: String },
    #[error("General storage error: {details}")]
    General { details: String },
    #[error("Block creation error: {0}")]
    BlockError(#[from] BlockError),
}

impl IsNotFoundError for StorageError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::NotFound { .. }) || matches!(self, Self::NotFoundDbAdapter { .. })
    }
}
