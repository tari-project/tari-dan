//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::IsNotFoundError;

use crate::jellyfish::JmtStorageError;

#[derive(Debug, thiserror::Error)]
pub enum StateTreeError {
    #[error("JMT Storage error: {0}")]
    JmtStorageError(#[from] JmtStorageError),
}

impl IsNotFoundError for StateTreeError {
    fn is_not_found_error(&self) -> bool {
        #[allow(clippy::single_match)]
        match self {
            StateTreeError::JmtStorageError(err) => err.is_not_found_error(),
        }
    }
}
