//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
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
        operation: &'static str,
    },
    #[error("Could not migrate the database")]
    MigrationError {
        #[from]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Malformed DB data in {operation}: {details}")]
    MalformedDbData { operation: &'static str, details: String },
    #[error("[{operation}] Not all queried transactions were found: {details}")]
    NotAllTransactionsFound { operation: &'static str, details: String },
    #[error("[{operation}] Not all queried substates were found: {details}")]
    NotAllSubstatesFound { operation: &'static str, details: String },
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
