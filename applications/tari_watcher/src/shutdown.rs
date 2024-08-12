//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::{future::Future, pin::Pin};

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub fn exit_signal() -> anyhow::Result<BoxFuture<()>> {
    #[cfg(unix)]
    let fut = unix_exit_signal()?;
    #[cfg(windows)]
    let fut = start_windows()?;

    Ok(fut)
}

#[cfg(unix)]
fn unix_exit_signal() -> anyhow::Result<BoxFuture<()>> {
    use tokio::signal::unix::SignalKind;

    let mut sighup = tokio::signal::unix::signal(SignalKind::hangup())?;
    let mut sigint = tokio::signal::unix::signal(SignalKind::interrupt())?;

    let fut = async move {
        tokio::select! {
            biased;
            _ = sigint.recv() => {
                log::info!("Received SIGINT, shutting down...");
            },
            // This is typically used to signal to reload configuration. Right now we simply exit.
            _ = sighup.recv() => {
                log::info!("Received SIGHUP, shutting down...");
            }
        }
    };

    Ok(Box::pin(fut))
}

#[cfg(windows)]
fn start_windows() -> anyhow::Result<BoxFuture<()>> {
    let mut sigint = tokio::signal::windows::ctrl_c()?;
    let mut sighup = tokio::signal::windows::ctrl_break()?;
    let mut sigshutdown = tokio::signal::windows::ctrl_shutdown()?;
    let fut = async move {
        tokio::select! {
            biased;
            _ = sigint.recv() => {
                log::info!("Received SIGINT, shutting down...");
            },
            _ = sighup.recv() => {
                log::info!("Received SIGHUP, shutting down...");
            }
            _ = sigshutdown.recv() => {
                log::info!("Received SIGSHUTDOWN, shutting down...");
            }
        }
    };
    Ok(Box::pin(fut))
}
