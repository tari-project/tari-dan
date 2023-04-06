//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(long, alias = "endpoint", env = "JRPC_ENDPOINT")]
    pub listen_addr: Option<SocketAddr>,
    #[clap(long, short = 'b', alias = "basedir")]
    pub base_dir: Option<PathBuf>,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }

    pub fn listen_address(&self) -> SocketAddr {
        self.listen_addr
            .unwrap_or_else(|| SocketAddr::from(([127u8, 0, 0, 1], 9100)))
    }

    pub fn base_dir(&self) -> PathBuf {
        self.base_dir
            .clone()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".tari/signallingserver"))
    }
}
