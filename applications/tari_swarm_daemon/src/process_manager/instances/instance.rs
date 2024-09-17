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
    settings: HashMap<String, String>,
    exit_status: Option<ExitStatus>,
}

impl Instance {
    pub(super) fn new_started(
        id: InstanceId,
        name: String,
        instance_type: InstanceType,
        child: Child,
        allocated_ports: AllocatedPorts,
        base_path: PathBuf,
        settings: HashMap<String, String>,
    ) -> Self {
        Self {
            id,
            name,
            instance_type,
            child,
            allocated_ports,
            base_path,
            settings,
            exit_status: None,
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

    pub fn settings(&self) -> &HashMap<String, String> {
        &self.settings
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

        #[cfg(target_family = "unix")]
        self.terminate_nix().await?;
        #[cfg(target_family = "windows")]
        self.terminate_win().await?;

        Ok(())
    }

    #[cfg(target_family = "unix")]
    async fn terminate_nix(&mut self) -> anyhow::Result<()> {
        use nix::{
            sys::signal::{kill, Signal},
            unistd::Pid,
        };
        let Some(pid) = self.child().id() else {
            return Ok(());
        };

        let pid = Pid::from_raw(pid as i32);
        kill(pid, Signal::SIGINT)?;
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
