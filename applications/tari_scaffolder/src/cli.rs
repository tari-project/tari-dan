//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use clap::Parser;

use crate::command::Command;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub(crate) struct Cli {
    #[clap(long, short = 'b', alias = "basedir")]
    pub base_dir: Option<PathBuf>,
    /// Bind address for JSON-rpc server
    // #[clap(long, short = 'r', alias = "rpc-address")]
    // pub json_rpc_address: Option<SocketAddr>,
    #[clap(subcommand)]
    pub command: Command,

    #[clap(long)]
    pub crates_root: Option<PathBuf>,

    #[clap(long, short = 'c', alias = "clean")]
    pub clean: bool,

    #[clap(long, short = 'o', alias = "output", default_value = "./output")]
    pub output_path: PathBuf,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }
}
