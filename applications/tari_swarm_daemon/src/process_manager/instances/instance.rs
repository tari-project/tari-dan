//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf};

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
    extra_args: HashMap<String, String>,
    is_running: bool,
}

impl Instance {
    pub(super) fn new_started(
        id: InstanceId,
        name: String,
        instance_type: InstanceType,
        child: Child,
        allocated_ports: AllocatedPorts,
        base_path: PathBuf,
        extra_args: HashMap<String, String>,
    ) -> Self {
        Self {
            id,
            name,
            instance_type,
            child,
            allocated_ports,
            base_path,
            extra_args,
            is_running: true,
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

    pub fn extra_args(&self) -> &HashMap<String, String> {
        &self.extra_args
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn check_running(&mut self) -> bool {
        if !self.is_running {
            return false;
        }

        // try_wait returns none if not exited
        let is_running = self.child_mut().try_wait().map(|v| v.is_none()).unwrap_or(false);
        self.is_running = is_running;
        is_running
    }

    pub async fn terminate(&mut self) -> anyhow::Result<()> {
        if !self.is_running {
            return Ok(());
        }

        #[cfg(target_family = "unix")]
        self.terminate_nix().await?;
        #[cfg(target_family = "windows")]
        self.terminate_win().await?;

        self.is_running = false;
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
        self.child_mut().wait().await?;
        Ok(())
    }

    #[cfg(target_family = "windows")]
    async fn terminate_win(&mut self) -> anyhow::Result<()> {
        // Should probably also implement a clean exit
        self.child_mut().kill().await?;
        Ok(())
    }
}
