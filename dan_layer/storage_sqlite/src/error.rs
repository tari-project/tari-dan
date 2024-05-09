use std::error::Error;

//  Copyright 2021. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that
// the  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the
// following  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED
// WARRANTIES,  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
// PARTICULAR PURPOSE ARE  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL,  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
// PROCUREMENT OF SUBSTITUTE GOODS OR  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
// CAUSED AND ON ANY THEORY OF LIABILITY,  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
// OTHERWISE) ARISING IN ANY WAY OUT OF THE  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH
// DAMAGE.
use diesel;
use tari_common_types::types::FixedHashSizeError;
use tari_dan_common_types::optional::IsNotFoundError;
use tari_dan_storage::StorageError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SqliteStorageError {
    #[error("Could not connect to database: {source}")]
    ConnectionError {
        #[from]
        source: diesel::ConnectionError,
    },
    #[error("General diesel error during operation {operation}: {source}")]
    DieselError {
        source: diesel::result::Error,
        operation: String,
    },
    #[error("Could not migrate the database: {source}")]
    MigrationError {
        #[from]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("Encountered malformed hash data")]
    MalformedHashData,
    #[error("Malformed DB data: {0}")]
    MalformedDbData(String),
    // #[error(transparent)]
    // ModelError(#[from] ModelError),
    #[error("Conversion error:{reason}")]
    ConversionError { reason: String },
    #[error("Malformed metadata for key '{key}'")]
    MalformedMetadata { key: String },
    #[error("Serialization failed")]
    SerializationFailed(#[from] serde_json::Error),
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
}

impl From<SqliteStorageError> for StorageError {
    fn from(source: SqliteStorageError) -> Self {
        match source {
            SqliteStorageError::ConnectionError { .. } => StorageError::ConnectionError {
                reason: source.to_string(),
            },
            SqliteStorageError::DieselError { .. } => StorageError::QueryError {
                reason: source.to_string(),
            },
            SqliteStorageError::MigrationError { .. } => StorageError::MigrationError {
                reason: source.to_string(),
            },
            other => StorageError::General {
                details: other.to_string(),
            },
        }
    }
}

impl From<FixedHashSizeError> for SqliteStorageError {
    fn from(_: FixedHashSizeError) -> Self {
        SqliteStorageError::MalformedHashData
    }
}

impl IsNotFoundError for SqliteStorageError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, SqliteStorageError::DieselError { source, .. } if matches!(source, diesel::result::Error::NotFound))
    }
}
