//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error("Error: {0}")]
    Anyhow(#[from] anyhow::Error),
    // #[error("Not found")]
    // NotFound,
}
