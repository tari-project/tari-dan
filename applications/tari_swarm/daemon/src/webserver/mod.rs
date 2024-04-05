//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod context;
mod error;
mod handler;
mod rpc;
mod server;

use std::future::Future;

use context::HandlerContext;
use log::*;
use tokio::task;

use crate::config::WebserverConfig;

const LOG_TARGET: &str = "tari::dan::swarm::webserver";

pub fn spawn<S>(config: WebserverConfig, shutdown: S) -> task::JoinHandle<anyhow::Result<()>>
where S: Future<Output = ()> + Unpin + Send + 'static {
    let context = HandlerContext::new(config);
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
