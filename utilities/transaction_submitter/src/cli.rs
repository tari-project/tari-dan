//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, path::PathBuf};

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
    StressTest(StressTestArgs),
}

#[derive(Args, Debug)]
pub struct StressTestArgs {
    #[clap(long, short = 'n')]
    pub num_transactions: Option<u64>,
    #[clap(long, alias = "skip", short = 'k')]
    pub skip_transactions: Option<u64>,
    #[clap(long, short = 'a')]
    pub jrpc_addresses: Vec<SocketAddr>,
    #[clap(long, short = 'f')]
    pub transaction_file: PathBuf,
    #[clap(long, short = 'y')]
    pub no_confirm: bool,
}
