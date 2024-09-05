// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::bail;
use log::*;
use tari_shutdown::Shutdown;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    process::{Child, Command as TokioCommand},
    sync::mpsc::{self},
    time::sleep,
};
use url::Url;

use crate::{
    config::Channels,
    constants::DEFAULT_VALIDATOR_PID_PATH,
    monitoring::{monitor_child, ProcessStatus},
};

#[allow(unused)]
pub async fn clean_stale_pid_file(pid_file_path: PathBuf) -> anyhow::Result<()> {
    log::info!("Checking for stale PID file at {}", pid_file_path.display());
    if !pid_file_path.exists() {
        info!("PID file for VN does not exist, create new one");
        return Ok(());
    }

    if let Ok(pid_str) = fs::read_to_string(&pid_file_path).await {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            // check if still running
            let status = TokioCommand::new("kill").arg("-0").arg(pid.to_string()).status().await;
            if status.map(|s| !s.success()).unwrap_or(true) {
                log::info!("Removing stale PID file");
                fs::remove_file(&pid_file_path).await?;
                return Ok(());
            }

            log::info!("Process with PID {} is still running", pid);
            bail!("PID file is locked by an active process");
        }
    }

    Ok(())
}

pub struct ChildChannel {
    pub rx_log: mpsc::Receiver<ProcessStatus>,
    pub tx_log: mpsc::Sender<ProcessStatus>,
    pub rx_alert: mpsc::Receiver<ProcessStatus>,
    pub tx_alert: mpsc::Sender<ProcessStatus>,
    pub cfg_alert: Channels,
}

async fn spawn_validator_node(
    binary_path: PathBuf,
    base_dir: PathBuf,
    minotari_node_grpc_url: &Url,
) -> anyhow::Result<Child> {
    debug!("Using VN binary at: {}", binary_path.display());
    debug!("Using VN base dir in directory: {}", base_dir.display());
    // Needed to ensure the base dir exists before we create the pid file
    fs::create_dir_all(&base_dir).await?;

    let child = TokioCommand::new(binary_path)
        .arg(format!("-b{}", base_dir.display()))
        .arg(format!("--node-grpc={minotari_node_grpc_url}"))
        .stdin(Stdio::null())
        // TODO: redirect these to a file and optionally stdout
        // .stdout(Stdio::null())
        // .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()?;

    Ok(child)
}

pub async fn spawn_validator_node_os(
    binary_path: PathBuf,
    vn_base_dir: PathBuf,
    cfg_alert: Channels,
    auto_restart: bool,
    minotari_node_grpc_url: Url,
    mut trigger_signal: Shutdown,
) -> anyhow::Result<ChildChannel> {
    let (tx_log, rx_log) = mpsc::channel(16);
    let (tx_alert, rx_alert) = mpsc::channel(16);
    let (tx_restart, mut rx_restart) = mpsc::channel(1);

    let tx_log_clone_main = tx_log.clone();
    let tx_alert_clone_main = tx_alert.clone();
    let tx_restart_clone_main = tx_restart.clone();
    tokio::spawn(async move {
        loop {
            let child_res =
                spawn_validator_node(binary_path.clone(), vn_base_dir.clone(), &minotari_node_grpc_url).await;

            match child_res {
                Ok(child) => {
                    let pid = child.id().unwrap_or(0);
                    info!("Spawned validator child process with id {}", pid);

                    // TODO: the VN should create a PID file in its base dir
                    let path = vn_base_dir.join(DEFAULT_VALIDATOR_PID_PATH);
                    if let Err(err) = create_pid_file(path, pid).await {
                        error!("Failed to create VN PID file: {}", err);
                    }

                    let tx_log_monitor = tx_log_clone_main.clone();
                    let tx_alert_monitor = tx_alert_clone_main.clone();
                    let tx_restart_monitor = tx_restart_clone_main.clone();
                    // spawn monitoring and handle logs and alerts
                    tokio::spawn(monitor_child(
                        child,
                        tx_log_monitor,
                        tx_alert_monitor,
                        tx_restart_monitor,
                    ));
                },
                Err(e) => {
                    error!("Failed to spawn child process: {}. Retrying in 5s", e);
                    sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                },
            }

            // block channel until we receive a restart signal
            match rx_restart.recv().await {
                Some(_) => {
                    if !auto_restart {
                        info!("Received restart signal, but auto restart is disabled, exiting");
                        trigger_signal.trigger();
                        break;
                    }

                    info!("Received signal, preparing to restart VN process");
                },
                None => {
                    error!("Failed to receive restart signal, exiting");
                    break;
                },
            }
        }
    });

    Ok(ChildChannel {
        rx_log,
        tx_log,
        tx_alert,
        rx_alert,
        cfg_alert,
    })
}

pub async fn create_pid_file<P: AsRef<Path>>(path: P, pid: u32) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await?;

    file.write_all(pid.to_string().as_bytes()).await?;

    Ok(())
}

async fn check_existing_node_os(base_dir: PathBuf) -> Option<u32> {
    let process_dir = base_dir.join("processes");
    if !process_dir.exists() {
        debug!("VN process directory does not exist");
        return None;
    }

    if let Ok(pid_str) = fs::read_to_string(DEFAULT_VALIDATOR_PID_PATH).await {
        debug!("Found PID file: {}", pid_str);

        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            if (TokioCommand::new("ps").arg("-p").arg(pid.to_string()).status().await).is_ok() {
                info!("Founding existing running validator process with PID: {}", pid);
                return Some(pid);
            }
            error!("Failed to find process with PID: {}", pid);
        } else {
            error!("Unable to parse PID file to number, this should not happen");
        }
    }

    None
}

pub async fn start_validator(
    validator_path: PathBuf,
    vn_base_dir: PathBuf,
    minotari_node_grpc_url: Url,
    alerting_config: Channels,
    auto_restart: bool,
    trigger_signal: Shutdown,
) -> Option<ChildChannel> {
    let opt = check_existing_node_os(vn_base_dir.clone()).await;
    if let Some(pid) = opt {
        info!("Picking up existing VN process with id: {}", pid);
        // todo: create new process status channel for picked up process
        return None;
    } else {
        debug!("No existing VN process found, spawn new one");
    }

    let cc = spawn_validator_node_os(
        validator_path,
        vn_base_dir,
        alerting_config,
        auto_restart,
        minotari_node_grpc_url,
        trigger_signal,
    )
    .await
    .ok()?;

    Some(cc)
}
