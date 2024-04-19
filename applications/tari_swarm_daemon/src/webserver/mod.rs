//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod context;
mod error;
mod handler;
mod rpc;
mod server;

mod templates;

use std::future::Future;

use context::HandlerContext;
use log::*;
use tokio::task;

use crate::{config::Config, process_manager::ProcessManagerHandle};

const LOG_TARGET: &str = "tari::dan::swarm::webserver";

pub fn spawn<S>(config: Config, shutdown: S, pm_handle: ProcessManagerHandle) -> task::JoinHandle<anyhow::Result<()>>
where S: Future<Output = ()> + Unpin + Send + 'static {
    let context = HandlerContext::new(config, pm_handle);
    tokio::spawn(async move {
        tokio::select! {
            result = server::run(context) => {
                result
            },
            _ = shutdown => {
                info!(target: LOG_TARGET, "Webserver shutting down");
                Ok(())
            }
        }
    })
}
