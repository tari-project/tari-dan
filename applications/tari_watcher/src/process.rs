// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{path::PathBuf, process::Stdio};

use anyhow::bail;
use log::*;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    process::Command as TokioCommand,
    sync::mpsc::{self},
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
    pub pid: u32,
    pub rx_log: mpsc::Receiver<ProcessStatus>,
    pub tx_log: mpsc::Sender<ProcessStatus>,
    pub rx_alert: mpsc::Receiver<ProcessStatus>,
    pub tx_alert: mpsc::Sender<ProcessStatus>,
    pub cfg_alert: Channels,
}

pub async fn spawn_validator_node_os(
    validator_node_path: PathBuf,
    validator_config_path: PathBuf,
    base_dir: PathBuf,
    cfg_alert: Channels,
) -> anyhow::Result<ChildChannel> {
    let node_binary_path = base_dir.join(validator_node_path);
    let mut vn_cfg_path = base_dir.join(validator_config_path);
    let vn_cfg_str = vn_cfg_path.as_mut_os_str().to_str();
    debug!("Using VN binary at: {}", node_binary_path.display());
    debug!("Using VN config in directory: {}", vn_cfg_str.unwrap_or_default());

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

    if let Err(e) = create_pid_file(PathBuf::from(DEFAULT_VALIDATOR_PID_PATH)).await {
        log::error!("Failed to create PID file when spawning node: {}", e);
    }

    let path = base_dir.join(DEFAULT_VALIDATOR_PID_PATH);
    debug!(
        "Spawning validator node with process persisted at file: {}",
        path.display()
    );

    create_pid_file(path.clone()).await?;

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await?;
    file.write_all(pid.to_string().as_bytes()).await?;

    let (tx_log, rx_log) = mpsc::channel(16);
    let (tx_alert, rx_alert) = mpsc::channel(16);
    tokio::spawn(monitor_child(child, tx_log.clone(), tx_alert.clone()));

    Ok(ChildChannel {
        pid,
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

pub struct Process {
    // Child process ID of the forked validator instance.
    pid: Option<u32>,
}

impl Process {
    pub fn new() -> Self {
        Self { pid: None }
    }

    pub async fn start_validator(
        &mut self,
        validator_path: PathBuf,
        validator_config_path: PathBuf,
        base_dir: PathBuf,
        alerting_config: Channels,
    ) -> Option<ChildChannel> {
        let opt = check_existing_node_os(base_dir.clone()).await;
        if let Some(pid) = opt {
            info!("Picking up existing validator node process with id: {}", pid);

            self.pid = Some(pid);
            // todo: create new process status channel for picked up process
            return None;
        } else {
            debug!("No existing validator node process found, spawn new one");
        }

        let cc = spawn_validator_node_os(validator_path, validator_config_path, base_dir, alerting_config)
            .await
            .ok()?;
        self.pid = Some(cc.pid);

        Some(cc)
    }
}
