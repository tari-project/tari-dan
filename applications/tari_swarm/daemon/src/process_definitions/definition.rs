//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use async_trait::async_trait;
use tokio::process::Command;

use super::context::ProcessContext;

#[async_trait]
pub trait ProcessDefinition: Send {
    async fn get_command(&self, context: ProcessContext<'_>) -> anyhow::Result<Command>;

    fn get_relative_data_path(&self) -> Option<PathBuf> {
        None
    }
}
