//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use url::Url;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(subcommand)]
    pub sub_command: SubCommand,
    #[clap(flatten)]
    pub common: CommonArgs,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }
}

#[derive(Args, Debug)]
pub struct CommonArgs {
    #[clap(long, short = 'd', alias = "db", default_value = "data/tariswap-test-bench.sqlite")]
    pub db_path: PathBuf,
    #[clap(
        long,
        short = 'i',
        alias = "indexer",
        default_value = "http://localhost:18300/json_rpc"
    )]
    pub indexer_url: Url,
    #[clap(long, short = 'v', alias = "vn", default_value = "http://localhost:18200/json_rpc")]
    pub validator_node_url: Url,
}

#[derive(Subcommand, Debug)]
pub enum SubCommand {
    Run(RunArgs),
}

#[derive(Args, Debug)]
pub struct RunArgs {}
