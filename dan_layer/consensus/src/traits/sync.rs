//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;

#[async_trait]
pub trait SyncManager {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn check_sync(&self) -> Result<SyncStatus, Self::Error>;

    async fn sync(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SyncStatus {
    UpToDate,
    Behind,
}
