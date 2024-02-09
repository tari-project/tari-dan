//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf};

use clap::Parser;

use crate::{command::Command, generators::GeneratorType};

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

    #[clap(long, alias = "clean")]
    pub clean: bool,

    #[clap(long, short = 'o', alias = "output")]
    pub output_path: Option<PathBuf>,

    #[clap(long, short = 'g', alias = "generator")]
    pub generator: GeneratorType,

    #[clap(long, short = 'd', alias = "data", value_parser = parse_hashmap)]
    pub data: Option<HashMap<String, String>>,

    #[clap(long, short = 'c', alias = "config")]
    pub generator_config_file: Option<PathBuf>,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }
}

fn parse_hashmap(input: &str) -> anyhow::Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in input.split(',') {
        let mut parts = pair.splitn(2, ':');
        let key = parts.next().unwrap().to_string();
        let value = parts.next().unwrap_or("").to_string();
        map.insert(key, value);
    }
    Ok(map)
}
