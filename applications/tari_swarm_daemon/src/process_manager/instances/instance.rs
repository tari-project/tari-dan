//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf, process::ExitStatus};

use tokio::process::Child;

use crate::{config::InstanceType, process_manager::AllocatedPorts};

pub type InstanceId = u32;

pub struct Instance {
    id: InstanceId,
    name: String,
    instance_type: InstanceType,
    child: Child,
    allocated_ports: AllocatedPorts,
    base_path: PathBuf,
    /// Directory holding the forwarded stdout/stderr of this process. The parent of `base_path`, which points at the
    /// network subdirectory the process itself writes to.
    process_dir: PathBuf,
    settings: HashMap<String, String>,
    envs: Vec<(String, String)>,
    exit_status: Option<ExitStatus>,
    is_config_dirty: bool,
}

impl Instance {
    pub(super) fn new_started(
        id: InstanceId,
        name: String,
        instance_type: InstanceType,
        child: Child,
        allocated_ports: AllocatedPorts,
        base_path: PathBuf,
        process_dir: PathBuf,
        envs: Vec<(String, String)>,
        settings: HashMap<String, String>,
    ) -> Self {
        Self {
            id,
            name,
            instance_type,
            child,
            allocated_ports,
            base_path,
            process_dir,
            envs,
            settings,
            exit_status: None,
            is_config_dirty: false,
        }
    }

    pub fn id(&self) -> InstanceId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn instance_type(&self) -> InstanceType {
        self.instance_type
    }

    pub fn child(&self) -> &Child {
        &self.child
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn allocated_ports(&self) -> &AllocatedPorts {
        &self.allocated_ports
    }

    pub fn base_path(&self) -> &PathBuf {
        &self.base_path
    }

    pub fn stdout_log_path(&self) -> PathBuf {
        self.process_dir.join("stdout.log")
    }

    pub fn stderr_log_path(&self) -> PathBuf {
        self.process_dir.join("stderr.log")
    }

    pub fn envs(&self) -> &[(String, String)] {
        &self.envs
    }

    pub fn settings(&self) -> &HashMap<String, String> {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: HashMap<String, String>) {
        self.settings = settings;
    }

    pub fn set_envs(&mut self, envs: Vec<(String, String)>) {
        self.envs = envs;
    }

    pub fn is_config_dirty(&self) -> bool {
        self.is_config_dirty
    }

    pub fn set_config_dirty(&mut self, dirty: bool) {
        self.is_config_dirty = dirty;
    }

    pub fn is_running(&self) -> bool {
        self.exit_status.is_none()
    }

    pub fn check_running(&mut self) -> anyhow::Result<Option<ExitStatus>> {
        if let Some(status) = self.exit_status {
            return Ok(Some(status));
        }

        // try_wait returns none if not exited
        let status = self.child_mut().try_wait()?;
        self.exit_status = status;
        Ok(status)
    }

    pub async fn terminate(&mut self) -> anyhow::Result<()> {
        if !self.is_running() {
            return Ok(());
        }

        // Base layer node does not support clean shutdown, so we use SIGTERM
        #[cfg(target_family = "unix")]
        self.terminate_nix(self.instance_type.is_tari_node()).await?;
        #[cfg(target_family = "windows")]
        self.terminate_win().await?;

        Ok(())
    }

    #[cfg(target_family = "unix")]
    async fn terminate_nix(&mut self, use_sig_int: bool) -> anyhow::Result<()> {
        use nix::{
            sys::signal::{Signal, kill},
            unistd::Pid,
        };
        let Some(pid) = self.child().id() else {
            return Ok(());
        };

        let pid = Pid::from_raw(pid as i32);
        let sig = if use_sig_int { Signal::SIGINT } else { Signal::SIGTERM };
        kill(pid, sig)?;
        let status = self.child_mut().wait().await?;
        self.exit_status = Some(status);
        Ok(())
    }

    #[cfg(target_family = "windows")]
    async fn terminate_win(&mut self) -> anyhow::Result<()> {
        // Should probably also implement a clean exit
        self.child_mut().kill().await?;
        self.exit_status = Some(ExitStatus::default());
        Ok(())
    }
}
