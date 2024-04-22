//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tokio::process::Command;

use super::context::ProcessContext;

#[async_trait]
pub trait ProcessDefinition: Send {
    async fn get_command(&self, context: ProcessContext<'_>) -> anyhow::Result<Command>;
}
