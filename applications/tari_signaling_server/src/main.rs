//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod cli;
mod data;
mod jrpc_server;

use std::fs;

use cli::Cli;
use data::Data;
use tari_common::initialize_logging;
use tari_shutdown::Shutdown;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::init();
    let _file = fs::remove_file(cli.base_dir().join("pid"));

    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();

    if let Err(e) = initialize_logging(
        cli.base_dir().join("config/logs.yml").as_path(),
        &cli.base_dir(),
        include_str!("../log4rs_sample.yml"),
    ) {
        eprintln!("{}", e);
        return Err(e.into());
    }

    let data = Data::new();

    let address = cli.listen_address();
    jrpc_server::listen(cli.base_dir(), address, data, shutdown_signal).await?;
    Ok(())
}
