//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use clap::{Args, Subcommand};

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand, Clone)]
pub(crate) enum Command {
    Scaffold(ScaffoldCommand),
}

#[derive(Debug, Clone, Args)]
pub(crate) struct ScaffoldCommand {
    pub wasm_path: PathBuf,
}
