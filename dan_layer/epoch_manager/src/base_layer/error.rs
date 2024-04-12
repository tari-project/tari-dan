//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_base_node_client::BaseNodeClientError;
use tari_dan_storage_sqlite::error::SqliteStorageError;

use crate::EpochManagerError;

impl From<BaseNodeClientError> for EpochManagerError {
    fn from(e: BaseNodeClientError) -> Self {
        Self::BaseNodeError(anyhow::Error::from(e))
    }
}

impl From<SqliteStorageError> for EpochManagerError {
    fn from(e: SqliteStorageError) -> Self {
        Self::SqlLiteStorageError(anyhow::Error::from(e))
    }
}
