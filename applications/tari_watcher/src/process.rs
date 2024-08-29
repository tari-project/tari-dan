// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{path::PathBuf, process::Stdio};

use anyhow::bail;
use log::*;
use tari_shutdown::Shutdown;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    process::{Child, Command as TokioCommand},
    sync::mpsc::{self},
    time::{sleep, Duration},
};

use crate::{
    config::Channels,
    constants::DEFAULT_VALIDATOR_PID_PATH,
    monitoring::{monitor_child, ProcessStatus},
};

#[allow(unused)]
pub async fn clean_stale_pid_file(pid_file_path: PathBuf) -> anyhow::Result<()> {
    log::info!("Checking for stale PID file at {}", pid_file_path.display());
    if !pid_file_path.exists() {
        info!("PID file for validator does not exist, create new one");
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

async fn create_pid_file(path: PathBuf) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .await?;

    file.write_all(std::process::id().to_string().as_bytes()).await?;

    Ok(())
}

pub struct ChildChannel {
    pub rx_log: mpsc::Receiver<ProcessStatus>,
    pub tx_log: mpsc::Sender<ProcessStatus>,
    pub rx_alert: mpsc::Receiver<ProcessStatus>,
    pub tx_alert: mpsc::Sender<ProcessStatus>,
    pub cfg_alert: Channels,
}

async fn spawn_child(
    validator_node_path: PathBuf,
    validator_config_path: PathBuf,
    base_dir: PathBuf,
) -> anyhow::Result<Child> {
    let node_binary_path = base_dir.join(validator_node_path);
    let vn_cfg_path = base_dir.join(validator_config_path);
    debug!("Using VN binary at: {}", node_binary_path.display());
    debug!("Using VN config in directory: {}", vn_cfg_path.display());

    let child = TokioCommand::new(node_binary_path.clone().into_os_string())
        .arg("-b")
        .arg(vn_cfg_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()?;

    let pid = child.id().expect("Failed to get PID for child process");
    info!("Spawned validator child process with id {}", pid);

    let path = base_dir.join(DEFAULT_VALIDATOR_PID_PATH);
    if let Err(e) = create_pid_file(path.clone()).await {
        log::error!("Failed to create PID file when spawning node: {}", e);
    }

    create_pid_file(path.clone()).await?;

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await?;
    file.write_all(pid.to_string().as_bytes()).await?;

    Ok(child)
}

pub async fn spawn_validator_node_os(
    validator_node_path: PathBuf,
    validator_config_path: PathBuf,
    base_dir: PathBuf,
    cfg_alert: Channels,
    auto_restart: bool,
    mut trigger_signal: Shutdown,
) -> anyhow::Result<ChildChannel> {
    let (tx_log, rx_log) = mpsc::channel(16);
    let (tx_alert, rx_alert) = mpsc::channel(16);
    let (tx_restart, mut rx_restart) = mpsc::channel(1);

    let tx_log_clone_main = tx_log.clone();
    let tx_alert_clone_main = tx_alert.clone();
    let tx_restart_clone_main = tx_restart.clone();
    tokio::spawn(async move {
        let mut restarted = false;

        loop {
            let child_res = spawn_child(
                validator_node_path.clone(),
                validator_config_path.clone(),
                base_dir.clone(),
            )
            .await;

            if restarted {
                // give it some time to clean up
                sleep(Duration::from_secs(10)).await;
            }

            match child_res {
                Ok(child) => {
                    let tx_log_monitor = tx_log_clone_main.clone();
                    let tx_alert_monitor = tx_alert_clone_main.clone();
                    let tx_restart_monitor = tx_restart_clone_main.clone();
                    // spawn monitoring and handle logs and alerts
                    tokio::spawn(async move {
                        monitor_child(child, tx_log_monitor, tx_alert_monitor, tx_restart_monitor).await;
                    });
                },
                Err(e) => {
                    error!("Failed to spawn child process: {:?}", e);
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

                    info!("Received signal, preparing to restart validator node");
                    restarted = true;
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

async fn check_existing_node_os(base_dir: PathBuf) -> Option<u32> {
    let process_dir = base_dir.join("processes");
    if !process_dir.exists() {
        debug!("Validator node process directory does not exist");
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
    validator_config_path: PathBuf,
    base_dir: PathBuf,
    alerting_config: Channels,
    auto_restart: bool,
    trigger_signal: Shutdown,
) -> Option<ChildChannel> {
    let opt = check_existing_node_os(base_dir.clone()).await;
    if let Some(pid) = opt {
        info!("Picking up existing validator node process with id: {}", pid);
        // todo: create new process status channel for picked up process
        return None;
    } else {
        debug!("No existing validator node process found, spawn new one");
    }

    let cc = spawn_validator_node_os(
        validator_path,
        validator_config_path,
        base_dir,
        alerting_config,
        auto_restart,
        trigger_signal,
    )
    .await
    .ok()?;

    Some(cc)
}
