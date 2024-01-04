//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(subcommand)]
    pub sub_command: SubCommand,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }
}

#[derive(Subcommand, Debug)]
pub enum SubCommand {
    Write(WriteArgs),
    Read(ReadArgs),
}

#[derive(Args, Debug)]
pub struct WriteArgs {
    #[clap(long, short = 'n')]
    pub num_transactions: u64,
    #[clap(long, short = 'o')]
    pub output_file: PathBuf,
    #[clap(long)]
    pub overwrite: bool,
    #[clap(long, short = 'm')]
    pub manifest: Option<PathBuf>,
    #[clap(long, short = 'g', alias = "global")]
    pub manifest_globals: Vec<String>,
    #[clap(long, short = 'k', alias = "signer")]
    pub signer_secret_key: Option<String>,
}
#[derive(Args, Debug)]
pub struct ReadArgs {
    #[clap(long, short = 'f')]
    pub input_file: PathBuf,
}
