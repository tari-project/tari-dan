//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::jellyfish::JmtStorageError;

#[derive(Debug, thiserror::Error)]
pub enum StateTreeError {
    #[error("Storage error: {0}")]
    StorageError(#[from] JmtStorageError),
}
